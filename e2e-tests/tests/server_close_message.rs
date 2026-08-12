use std::collections::HashSet;
use std::time::Duration;

use e2e_tests::{
    SoraTestConnection, SoraTestEvent, api_url, build_recvonly_data_channel_signaling_connection,
    disconnect_channel, generate_channel_id, load_env, signaling_urls,
};
use nojson::RawJson;
use sora_sdk::{SignalingDirection, SignalingType};

// run task の終了待機はこの値 + websocket_close_timeout + 1 秒で判定する。
// Sora 側から DataChannel が閉じられるのに十分な時間を確保する。
const DISCONNECT_WAIT_TIMEOUT: Duration = Duration::from_secs(3);
const WEBSOCKET_CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

/// 受信したテキストが DisconnectChannel API による Close メッセージかどうかを判定する。
///
/// Sora ドキュメント「シグナリングの型定義」の SignalingCloseMessage に基づき、
/// type が "close" で code が 1000、reason が "DISCONNECTED-API" の場合に true を返す。
fn is_disconnect_channel_close(text: &str) -> bool {
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
    if message_type != "close" {
        return false;
    }
    let code: u16 = match value
        .to_member("code")
        .and_then(|v| v.required())
        .and_then(|v| v.try_into())
    {
        Ok(code) => code,
        Err(_) => return false,
    };
    let reason: String = match value
        .to_member("reason")
        .and_then(|v| v.required())
        .and_then(|v| v.try_into())
    {
        Ok(reason) => reason,
        Err(_) => return false,
    };
    code == 1000 && reason == "DISCONNECTED-API"
}

/// on_signaling_message に通知された Close メッセージを待つ。
async fn wait_for_close_signaling_message(connection: &mut SoraTestConnection) {
    connection
        .wait_for_signaling_message(
            |signaling_type, direction, text| {
                signaling_type == SignalingType::DataChannel
                    && direction == SignalingDirection::Received
                    && is_disconnect_channel_close(text)
            },
            Duration::from_secs(15),
        )
        .await
        .expect("signaling label の Close メッセージを受信できませんでした");
}

/// Close の raw message が on_data_channel_message と on_signaling_message に各 1 回通知されることを確認する。
async fn assert_close_message_notified_once(connection: &mut SoraTestConnection) {
    let signaling_count = connection
        .count_signaling_message(|signaling_type, direction, text| {
            signaling_type == SignalingType::DataChannel
                && direction == SignalingDirection::Received
                && is_disconnect_channel_close(text)
        })
        .await;
    assert_eq!(
        signaling_count, 1,
        "Close メッセージは on_signaling_message に 1 回だけ通知される必要があります"
    );

    let datachannel_count = connection
        .count_events(|event| match event {
            SoraTestEvent::DataChannelMessage { label, data } => {
                label == "signaling" && is_disconnect_channel_close(&String::from_utf8_lossy(data))
            }
            _ => false,
        })
        .await;
    assert_eq!(
        datachannel_count, 1,
        "Close メッセージは on_data_channel_message に 1 回だけ通知される必要があります"
    );
}

/// server Close の処理で on_websocket_close が追加通知されないことを確認する。
///
/// Sora は DisconnectChannel による切断時に WebSocket Close フレームを送信し、
/// SDK 側も WebSocket を閉じた時点で通知するため、on_websocket_close は
/// WebSocket の終了時に 1 回だけ通知される。
/// server Close メッセージ (DataChannel) の処理では通知されないことを、
/// run 終了後の合計回数が 1 回であることで確認する。
async fn assert_websocket_close_not_duplicated(connection: &mut SoraTestConnection) {
    let count = connection
        .count_events(|event| matches!(event, SoraTestEvent::WebsocketClose { .. }))
        .await;
    assert_eq!(
        count, 1,
        "on_websocket_close は 1 回だけ通知される必要があります: {count} 回"
    );
}

/// Close 受信前に open を観測した各 DataChannel label について、on_data_channel_close が重複しないことを確認する。
async fn assert_data_channel_close_not_duplicated(connection: &mut SoraTestConnection) {
    let events = connection.events().await;
    let opened_labels: HashSet<String> = events
        .into_iter()
        .filter_map(|event| match event {
            SoraTestEvent::DataChannelOpen { label } => Some(label),
            _ => None,
        })
        .collect();
    assert!(
        !opened_labels.is_empty(),
        "Close 受信前に DataChannel の open が観測されている必要があります"
    );
    for label in &opened_labels {
        let close_count = connection
            .count_data_channel_close(|actual| actual == label)
            .await;
        assert!(
            close_count <= 1,
            "on_data_channel_close が label='{label}' で重複して呼ばれました: {close_count} 回"
        );
    }
}

/// DisconnectChannel API で Sora 側から切断し、server Close で run が正常終了することを確認する。
///
/// WebSocket が接続中の状態で server Close を受信した場合、
/// DataChannel の終了待機後に WebSocket close handshake を実行して
/// `disconnect_wait_timeout + websocket_close_timeout + 1 秒` 以内に `Ok(())` を返す。
#[tokio::test]
async fn server_close_message_terminates_run_while_websocket_connected() {
    load_env();

    let Some(api_url) = api_url() else {
        return;
    };
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let mut connection = build_recvonly_data_channel_signaling_connection(
        urls,
        channel_id.clone(),
        DISCONNECT_WAIT_TIMEOUT,
        WEBSOCKET_CLOSE_TIMEOUT,
    );

    connection
        .wait_for_switched(Duration::from_secs(15))
        .await
        .expect("switched 通知の受信がタイムアウトしました");

    // signaling DataChannel の open を待つ。
    // 全 DataChannel オープンから SDK の WS_DISCONNECT_DELAY (10 秒) が経過する前に
    // API を実行するため、まず最小限の signaling チャネルの open を確認する。
    connection
        .wait_for_data_channel_open(|label| label == "signaling", Duration::from_secs(15))
        .await
        .expect("signaling DataChannel の open が観測されませんでした");

    // API request 開始前に on_websocket_close が未通知であることを確認する。
    let websocket_close_count_before_api = connection
        .count_events(|event| matches!(event, SoraTestEvent::WebsocketClose { .. }))
        .await;
    assert_eq!(
        websocket_close_count_before_api, 0,
        "API request 開始前に on_websocket_close が通知されていてはいけません"
    );

    // クライアントから disconnect() は呼ばず、DisconnectChannel API で Sora 側から切断する。
    disconnect_channel(&api_url, &channel_id)
        .await
        .expect("DisconnectChannel API の実行に失敗しました");

    wait_for_close_signaling_message(&mut connection).await;
    assert_close_message_notified_once(&mut connection).await;

    // server Close は terminal event のため、run が Ok(()) で終了する。
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

    // Sora は切断時に WebSocket Close フレームを送信するため、on_websocket_close は
    // その時点で 1 回通知される。server Close メッセージの処理で追加通知されない。
    assert_websocket_close_not_duplicated(&mut connection).await;

    assert_data_channel_close_not_duplicated(&mut connection).await;
}

/// WebSocket 切断後に DisconnectChannel API で Sora 側から切断し、
/// server Close で run が正常終了することを確認する。
///
/// WebSocket がすでに閉じた状態の場合、`disconnect_wait_timeout + 1 秒` 以内に
/// `Ok(())` を返す。
#[tokio::test]
async fn server_close_message_terminates_run_after_websocket_closed() {
    load_env();

    let Some(api_url) = api_url() else {
        return;
    };
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let mut connection = build_recvonly_data_channel_signaling_connection(
        urls,
        channel_id.clone(),
        DISCONNECT_WAIT_TIMEOUT,
        WEBSOCKET_CLOSE_TIMEOUT,
    );

    connection
        .wait_for_switched(Duration::from_secs(15))
        .await
        .expect("switched 通知の受信がタイムアウトしました");

    // SDK が WebSocket を自分から閉じるまで待つ。
    // 全 DataChannel オープン + 10 秒 + 余裕を見て 60 秒。
    connection
        .wait_for_event(
            |event| matches!(event, SoraTestEvent::WebsocketClose { .. }),
            Duration::from_secs(60),
        )
        .await
        .expect("WebSocket Close コールバックが届きませんでした");

    // クライアントから disconnect() は呼ばず、DisconnectChannel API で Sora 側から切断する。
    disconnect_channel(&api_url, &channel_id)
        .await
        .expect("DisconnectChannel API の実行に失敗しました");

    wait_for_close_signaling_message(&mut connection).await;
    assert_close_message_notified_once(&mut connection).await;

    // WebSocket は切断済みのため close handshake は実行されず、run が Ok(()) で終了する。
    let run_result = connection
        .wait_for_run_finished(DISCONNECT_WAIT_TIMEOUT + Duration::from_secs(1))
        .await;
    assert!(
        run_result.is_ok(),
        "run task は Ok(()) で終了する必要があります: {:?}",
        run_result
    );

    // SDK が自分で WebSocket を閉じた時点で on_websocket_close は 1 回通知される。
    // server Close メッセージの処理で追加通知されない。
    assert_websocket_close_not_duplicated(&mut connection).await;

    assert_data_channel_close_not_duplicated(&mut connection).await;
}
