//! `SoraConnection::run()` が返す `DisconnectReason` の E2E テスト。
//!
//! 実際の Sora に接続し、接続が終了したときの理由が `DisconnectReason` と一致することを確認する。
//! 接続と切断を伴う各シナリオのテストでも同じ理由を確認している
//! (クライアントからの切断は `send_disconnect_message.rs`、server Close は
//! `server_close_message.rs` を参照)。

use std::time::Duration;

use e2e_tests::proxy::ProxyHarness;
use e2e_tests::{
    SoraTestConnection, SoraTestEvent, api_url, build_metadata_with_access_token,
    build_recvonly_data_channel_signaling_connection, disconnect_channel, generate_channel_id,
    load_env, secret_key, signaling_urls,
};
use sora_sdk::{DisconnectReason, Role, SoraConnectionContext};

/// WebSocket シグナリングで接続し、クライアントからの切断で終了したときの理由を確認する。
#[tokio::test]
async fn disconnect_reason_is_client_disconnect() {
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

    // 接続の確立後にクライアントから切断する。
    connection
        .disconnect()
        .await
        .expect("disconnect に失敗しました");
    let reason = connection
        .wait_for_run_finished(Duration::from_secs(10))
        .await
        .expect("run task は Ok で終了する必要があります");
    assert_eq!(
        reason,
        DisconnectReason::ClientDisconnect,
        "クライアントからの切断の理由は ClientDisconnect になる必要があります"
    );
}

/// DataChannel シグナリングで接続し、クライアントからの切断で終了したときの理由を確認する。
///
/// WebSocket を閉じた後も DataChannel 経由で切断するため、
/// WebSocket シグナリングの切断とは経路が異なる。
#[tokio::test]
async fn disconnect_reason_is_client_disconnect_on_data_channel_signaling() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let mut connection =
        build_recvonly_data_channel_signaling_connection(urls, channel_id, None, None);

    connection
        .wait_for_switched(Duration::from_secs(15))
        .await
        .expect("switched 通知の受信がタイムアウトしました");
    // SDK が WebSocket を閉じて DataChannel シグナリングへ切り替えるまで待つ。
    connection
        .wait_for_event(
            |event| matches!(event, SoraTestEvent::WebsocketClose { .. }),
            Duration::from_secs(60),
        )
        .await
        .expect("WebSocket Close コールバックが届きませんでした");

    connection
        .disconnect()
        .await
        .expect("disconnect に失敗しました");
    let reason = connection
        .wait_for_run_finished(Duration::from_secs(10))
        .await
        .expect("run task は Ok で終了する必要があります");
    assert_eq!(
        reason,
        DisconnectReason::ClientDisconnect,
        "クライアントからの切断の理由は ClientDisconnect になる必要があります"
    );
}

/// DataChannel シグナリングで接続し、DisconnectChannel API で Sora 側から切断されたときの
/// 理由を確認する。
#[tokio::test]
async fn disconnect_reason_is_server_close_on_disconnect_channel_api() {
    load_env();

    let api_url = api_url().expect("TEST_API_URL が必要");
    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let mut connection =
        build_recvonly_data_channel_signaling_connection(urls, channel_id.clone(), None, None);

    connection
        .wait_for_switched(Duration::from_secs(15))
        .await
        .expect("switched 通知の受信がタイムアウトしました");

    // Sora 側から切断すると signaling DataChannel 経由で close メッセージが届く。
    disconnect_channel(&api_url, &channel_id)
        .await
        .expect("DisconnectChannel API の実行に失敗しました");

    let reason = connection
        .wait_for_run_finished(Duration::from_secs(15))
        .await
        .expect("run task は Ok で終了する必要があります");
    assert_eq!(
        reason,
        DisconnectReason::ServerClose {
            code: 1000,
            reason: "DISCONNECTED-API".to_string(),
        },
        "server Close の理由は close メッセージが通知した code と reason になる必要があります"
    );
}

/// シグナリングエラーで Sora が WebSocket を Close code 4490 で閉じたときの理由を確認する。
///
/// `spotlight` を指定せずに `spotlight_focus_rid` を指定すると、
/// Sora はシグナリングエラーとして WebSocket を 4490 で閉じる。
#[tokio::test]
async fn disconnect_reason_is_signaling_error() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");
    let mut builder = SoraTestConnection::builder(context, urls, channel_id, Role::RecvOnly)
        .spotlight_focus_rid("nonexistent-rid".to_string());
    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }
    let mut connection = builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました");

    // シグナリングエラーは JSON の close メッセージではなく
    // WebSocket の Close フレームで通知される。
    connection
        .wait_for_event(
            |event| {
                matches!(
                    event,
                    SoraTestEvent::WebsocketClose {
                        code: Some(4490),
                        ..
                    }
                )
            },
            Duration::from_secs(15),
        )
        .await
        .expect("WebSocket が Close code 4490 で閉じられませんでした");

    let reason = connection
        .wait_for_run_finished(Duration::from_secs(15))
        .await
        .expect("run task は Ok で終了する必要があります");
    match reason {
        DisconnectReason::SignalingError { reason } => assert!(
            !reason.is_empty(),
            "シグナリングエラーの理由は Sora が通知した Close reason になる必要があります"
        ),
        reason => {
            panic!("シグナリングエラーの理由は SignalingError になる必要があります: {reason:?}")
        }
    }
}

/// プロキシでシグナリングの WebSocket を Close フレームを伴わずに切断したときの理由を確認する。
#[tokio::test]
async fn disconnect_reason_is_websocket_closed_on_closed_without_close_frame() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let proxy = ProxyHarness::start(&urls)
        .await
        .expect("テスト用 Proxy の起動に失敗しました");
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");
    let mut builder = SoraTestConnection::builder(context, urls, channel_id, Role::RecvOnly)
        .proxy(proxy.proxy_info());
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

    // Close フレームを送らずに TCP を切断する。
    proxy.drop_connections();

    let reason = connection
        .wait_for_run_finished(Duration::from_secs(15))
        .await
        .expect("run task は Ok で終了する必要があります");
    assert_eq!(
        reason,
        DisconnectReason::WebSocketClosed {
            code: None,
            reason: String::new(),
        },
        "Close フレームを伴わない WebSocket の切断の理由は code 無しの WebSocketClosed になる必要があります"
    );
}
