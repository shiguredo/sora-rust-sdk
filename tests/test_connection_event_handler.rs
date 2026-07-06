//! SoraConnectionEventHandler トレイトのデフォルト実装の単体テスト。
use sora_sdk::{Role, SoraConnection, SoraConnectionContext, SoraConnectionEventHandler};

/// 全 12 メソッドのデフォルト実装を確認するための空の struct。
struct EmptyHandler;

impl SoraConnectionEventHandler for EmptyHandler {}

/// 特定メソッドのみオーバーライドし、残りはデフォルト実装を使う struct。
struct PartialHandler {
    notify_received: bool,
}

impl SoraConnectionEventHandler for PartialHandler {
    fn on_notify(&mut self, _text: &str) {
        self.notify_received = true;
    }
}

#[test]
fn default_implementation_has_noop_for_all_methods() {
    // 全メソッドがデフォルト実装で、呼び出してもパニックしないことを確認する
    let mut handler = EmptyHandler;

    handler.on_signaling_message(
        sora_sdk::SignalingType::WebSocket,
        sora_sdk::SignalingDirection::Sent,
        "{}",
    );
    handler.on_notify("{}");
    handler.on_push("{}");

    // RtpTransceiver と RtpReceiver は SoraConnectionContext がないと生成できないため、
    // デフォルト実装の呼び出し結果だけを検証する。
    // on_track, on_remove_track, on_switched, on_websocket_close,
    // on_message, on_data_channel, on_data_channel_open,
    // on_data_channel_message, on_data_channel_close は
    // デフォルト実装では何もせず、パニックしない。
    handler.on_switched();
    handler.on_websocket_close(Some(1000), "正常終了");
    handler.on_message("test", b"hello");
    handler.on_data_channel("test");
    handler.on_data_channel_open("test");
    handler.on_data_channel_message("test", b"hello");
    handler.on_data_channel_close("test");
}

#[test]
fn partial_override_uses_defaults_for_other_methods() {
    // on_notify のみオーバーライドし、他はデフォルト実装のまま呼び出せることを確認する
    let mut handler = PartialHandler {
        notify_received: false,
    };

    assert!(!handler.notify_received);

    // デフォルト実装のままのメソッドを呼び出してもパニックしない
    handler.on_push("{}");
    handler.on_switched();
    handler.on_websocket_close(None, "");
    handler.on_message("test", b"hello");
    handler.on_data_channel("test");
    handler.on_data_channel_open("test");
    handler.on_data_channel_message("test", b"hello");
    handler.on_data_channel_close("test");

    // オーバーライドしたメソッドが呼ばれることを確認する
    handler.on_notify("{}");
    assert!(handler.notify_received, "on_notify が呼ばれていません");
}

#[test]
fn builder_accepts_event_handler() {
    // SoraConnectionBuilder::new() が event_handler をデフォルト値として設定することを確認する
    let context = SoraConnectionContext::new().expect("SoraConnectionContext の作成に失敗しました");
    let builder = SoraConnection::builder(
        context,
        vec!["wss://example.com/signaling".to_string()],
        "test-channel".to_string(),
        Role::SendRecv,
        EmptyHandler,
    );
    let _ = builder; // builder が event_handler を持っていることを確認するだけ

    // event_handler を使って build() できることも確認する
    let context = SoraConnectionContext::new().expect("SoraConnectionContext の作成に失敗しました");
    let builder = SoraConnection::builder(
        context,
        vec!["wss://example.com/signaling".to_string()],
        "test-channel".to_string(),
        Role::SendRecv,
        EmptyHandler,
    );
    let result = builder.build();
    assert!(result.is_ok(), "build() が失敗しました: {:?}", result.err());
}
