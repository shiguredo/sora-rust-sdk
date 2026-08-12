use std::time::Duration;

use e2e_tests::{
    SoraTestConnection, build_metadata_with_access_token, generate_channel_id, load_env,
    secret_key, signaling_urls,
};
use nojson::RawJson;
use sora_sdk::{Role, SignalingDirection, SignalingType, SoraConnectionContext};

const DISCONNECT_WAIT_TIMEOUT: Duration = Duration::from_secs(3);
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

/// DataChannel シグナリングでの client disconnect 検証用の recvonly 接続を構築する。
fn build_datachannel_connection(urls: Vec<String>, channel_id: String) -> SoraTestConnection {
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");
    let mut builder = SoraTestConnection::builder(context, urls, channel_id, Role::RecvOnly)
        .data_channel_signaling(true)
        .ignore_disconnect_websocket(true)
        .disconnect_wait_timeout(DISCONNECT_WAIT_TIMEOUT)
        .websocket_close_timeout(WEBSOCKET_CLOSE_TIMEOUT);
    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }
    builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました")
}

/// `disconnect()` を呼んだとき、signaling DataChannel 経由で disconnect メッセージが送信されることを確認する。
#[tokio::test]
async fn disconnect_message_is_sent_via_datachannel() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let mut connection = build_datachannel_connection(urls, channel_id);

    connection
        .wait_for_switched(Duration::from_secs(15))
        .await
        .expect("switched 通知の受信がタイムアウトしました");

    // DataChannel シグナリングへの切替は全 DataChannel (signaling / stats / notify /
    // push / rpc) のオープンと switched 受信の両方で確定する
    // (is_datachannel_signaling_ready)。signaling だけを待って disconnect() を呼ぶと、
    // 未確定のまま WebSocket 経路にフォールバックしてテストが不安定になるため、
    // 全チャネルのオープンを待ってから切断する。
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

    // disconnect 送信後の DataChannel クローズ待機と後始末を経て run が Ok(()) で終了する。
    let run_result = connection
        .wait_for_run_finished(DISCONNECT_WAIT_TIMEOUT + Duration::from_secs(1))
        .await;
    assert!(
        run_result.is_ok(),
        "run task は Ok(()) で終了する必要があります: {:?}",
        run_result
    );
}

/// `disconnect()` を呼んだとき、WebSocket 経由で disconnect メッセージが送信されることを確認する。
#[tokio::test]
async fn disconnect_message_is_sent_via_websocket() {
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
