use std::time::Duration;

use e2e_tests::{
    SoraTestConnection, SoraTestEvent, build_metadata_with_access_token, generate_channel_id,
    load_env, secret_key, signaling_urls, verify_data_channel_label,
};
use sora_sdk::{Role, SoraConnectionContext};

fn build_recvonly_connection(urls: Vec<String>, channel_id: String) -> SoraTestConnection {
    let context = SoraConnectionContext::new().expect("コンテキスト作成失敗");
    let mut builder = SoraTestConnection::builder(context, urls, channel_id, Role::RecvOnly)
        .data_channel_signaling(true)
        .ignore_disconnect_websocket(true);
    if let Some(token) = secret_key() {
        builder = builder.metadata(build_metadata_with_access_token(&token));
    }
    builder
        .connect()
        .expect("SoraTestConnection の作成に失敗しました")
}

/// `ignore_disconnect_websocket=true` で接続したとき、
/// switched 後に WebSocket が閉じられても DataChannel シグナリングが継続することを確認する。
///
/// SDK は全 DataChannel オープンから 10 秒後に WebSocket を自分から閉じる
/// (`src/connection.rs` の `WS_DISCONNECT_DELAY`)。
/// その後も DataChannel 経由のコマンド (stats 取得) が成立すれば、
/// run task が生き続けていることを確認できる。
#[tokio::test]
async fn test_recvonly_ignore_disconnect_websocket_keeps_signaling() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let channel_id = generate_channel_id();
    let mut connection = build_recvonly_connection(urls, channel_id);

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

    // WebSocket クローズ後も DataChannel 経由で stats が取得できることを確認する。
    // バグがある場合は run task が早期終了し、command channel が切れて
    // get_stats が CommandResponseMissing で失敗する。
    connection
        .wait_stats(
            |stats| verify_data_channel_label(stats, "signaling"),
            Duration::from_secs(15),
        )
        .await
        .expect(
            "WebSocket クローズ後の stats 取得に失敗しました (run task が異常終了している可能性)",
        );

    connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("disconnect に失敗しました");
}
