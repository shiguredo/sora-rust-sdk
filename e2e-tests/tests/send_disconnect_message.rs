use std::time::Duration;

use e2e_tests::{
    SoraTestConnection, build_metadata_with_access_token,
    build_recvonly_data_channel_signaling_connection, generate_channel_id, load_env, secret_key,
    signaling_urls,
};
use nojson::RawJson;
use sora_sdk::{Role, SignalingDirection, SignalingType, SoraConnectionContext};

// disconnect_wait_timeout は実到達検証の close 待機窓 (1 秒差) を広く取るため 10 秒にする。
// 通常系はサーバーが速やかにチャネルを閉じるため実行時間には影響しない。
const DISCONNECT_WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const WEBSOCKET_CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

/// クライアントが送信する disconnect メッセージかどうかを判定する。
///
/// Sora クライアント要求仕様の SignalingDisconnectMessage に基づき、
/// type が "disconnect" で reason が "NO-ERROR" の場合に true を返す。
fn is_disconnect_message(text: &str) -> bool {
    let Ok(json) = RawJson::parse(text) else {
        return false;
    };
    let value = json.value();
    let message_type: String = match value
        .to_member("type")
        .and_then(|v| v.required())
        .and_then(|v| v.try_into())
    {
        Ok(message_type) => message_type,
        Err(_) => return false,
    };
    if message_type != "disconnect" {
        return false;
    }
    let reason: String = match value
        .to_member("reason")
        .and_then(|v| v.required())
        .and_then(|v| v.try_into())
    {
        Ok(reason) => reason,
        Err(_) => return false,
    };
    reason == "NO-ERROR"
}

/// `disconnect()` を呼んだとき、signaling DataChannel 経由で disconnect メッセージが送信されることを確認する。
#[tokio::test]
async fn test_disconnect_message_is_sent_via_datachannel() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let mut connection = build_recvonly_data_channel_signaling_connection(
        urls,
        channel_id,
        Some(DISCONNECT_WAIT_TIMEOUT),
        Some(WEBSOCKET_CLOSE_TIMEOUT),
    );

    // DataChannel シグナリングへの切替は全 DataChannel の
    // オープンと switched 受信の両方で確定するので、
    // switched 受信して全チャネルがオープンするのを待つ。
    connection
        .wait_for_switched(Duration::from_secs(15))
        .await
        .expect("switched 通知の受信がタイムアウトしました");

    for label in ["signaling", "stats", "notify", "push", "rpc"] {
        connection
            .wait_for_data_channel_open(|l| l == label, Duration::from_secs(15))
            .await
            .unwrap_or_else(|_| panic!("DataChannel '{label}' の open が観測されませんでした"));
    }

    // disconnect() を呼ぶと signaling DataChannel 経由で disconnect メッセージが送信される。
    connection
        .disconnect()
        .await
        .expect("disconnect に失敗しました");

    connection
        .wait_for_signaling_message(
            |signaling_type, direction, text| {
                signaling_type == SignalingType::DataChannel
                    && direction == SignalingDirection::Sent
                    && is_disconnect_message(text)
            },
            Duration::from_secs(10),
        )
        .await
        .expect("DataChannel 経由の disconnect メッセージが送信されませんでした");

    // サーバーが disconnect を受信した証跡として、signaling DataChannel のクローズを待つ。
    // 待機時間は disconnect_wait_timeout (10 秒) より短く設定する。サーバーが disconnect を
    // 受信していればチャネルは速やかに閉じるが、未到達ならクローズ待機のタイムアウト
    // フォールバック (10 秒後) でしか close が通知されないため、この待機は失敗する。
    connection
        .wait_for_data_channel_close(
            |label| label == "signaling",
            DISCONNECT_WAIT_TIMEOUT - Duration::from_secs(1),
        )
        .await
        .expect(
            "disconnect 送信後に signaling DataChannel が閉じられませんでした (サーバーに disconnect が到達していない可能性)",
        );

    // disconnect 送信後の DataChannel クローズ待機と後始末を経て run が Ok(()) で終了する。
    // クローズ待機 (最大 disconnect_wait_timeout) と WebSocket close handshake
    // (最大 websocket_close_timeout) が直列に走るため、両方の予算に 1 秒の余裕を足す。
    let run_result = connection
        .wait_for_run_finished(
            DISCONNECT_WAIT_TIMEOUT + WEBSOCKET_CLOSE_TIMEOUT + Duration::from_secs(1),
        )
        .await;
    assert!(
        run_result.is_ok(),
        "run task は Ok(()) で終了する必要があります: {:?}",
        run_result
    );
}

/// `disconnect()` を呼んだとき、WebSocket 経由で disconnect メッセージが送信されることを確認する。
#[tokio::test]
async fn test_disconnect_message_is_sent_via_websocket() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");
    let mut builder = SoraTestConnection::builder(context, urls, channel_id, Role::RecvOnly);
    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }
    let mut connection = builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました");

    connection
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("接続に失敗しました");

    // disconnect() を呼ぶと WebSocket 経由で disconnect メッセージが送信される。
    connection
        .disconnect()
        .await
        .expect("disconnect に失敗しました");

    connection
        .wait_for_signaling_message(
            |signaling_type, direction, text| {
                signaling_type == SignalingType::WebSocket
                    && direction == SignalingDirection::Sent
                    && is_disconnect_message(text)
            },
            Duration::from_secs(10),
        )
        .await
        .expect("WebSocket 経由の disconnect メッセージが送信されませんでした");

    // WebSocket close handshake と後始末を経て run が Ok(()) で終了する。
    let run_result = connection
        .wait_for_run_finished(WEBSOCKET_CLOSE_TIMEOUT + Duration::from_secs(1))
        .await;
    assert!(
        run_result.is_ok(),
        "run task は Ok(()) で終了する必要があります: {:?}",
        run_result
    );
}
