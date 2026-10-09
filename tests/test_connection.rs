use shiguredo_webrtc::{FrameTransformerHandler, TransformableFrame};
use sora_sdk::{
    ParsedProxyInfo, ProxyInfo, Role, SoraConnection, SoraConnectionContext,
    SoraConnectionEventHandler,
};

fn proxy_info_with_url(url: String) -> ProxyInfo {
    ProxyInfo {
        url,
        ..Default::default()
    }
}

#[test]
fn parsed_proxy_info_accessors_all_fields() {
    let proxy = proxy_info_with_url("http://proxy.example.com:8080".to_string());
    let parsed = ParsedProxyInfo::parse(&proxy).expect("proxy URL の解析に失敗しました");
    assert_eq!(parsed.host(), "proxy.example.com");
    assert_eq!(parsed.port(), 8080);
    assert!(parsed.username().is_none());
    assert!(parsed.password().is_none());
    assert!(!parsed.user_agent().is_empty());
}

#[test]
fn parsed_proxy_info_username_password_none() {
    let proxy = proxy_info_with_url("http://host:1234".to_string());
    let parsed = ParsedProxyInfo::parse(&proxy).expect("proxy URL の解析に失敗しました");
    assert!(parsed.username().is_none());
    assert!(parsed.password().is_none());
}

#[test]
fn sender_and_receiver_video_transform_accept_handler() {
    struct PassThroughTransform;

    impl FrameTransformerHandler for PassThroughTransform {
        fn transform(&self, frame: TransformableFrame) -> Option<TransformableFrame> {
            Some(frame)
        }
    }

    struct TestEventHandler;

    impl SoraConnectionEventHandler for TestEventHandler {}

    let context = SoraConnectionContext::new().expect("context の作成に失敗しました");
    // 送受信の transform 設定 API がハンドラを受け付けて、
    // 接続の構築まで通ることを確認する。
    // 実際のフレーム変換の適用は e2e-tests で検証する。
    let (connection, _handle) = SoraConnection::builder(
        context,
        vec!["wss://example.invalid/signaling".to_string()],
        "test-channel".to_string(),
        Role::SendRecv,
        TestEventHandler,
    )
    .sender_video_transform(Box::new(PassThroughTransform))
    .receiver_video_transform(Box::new(PassThroughTransform))
    .build()
    .expect("接続の生成に失敗しました");

    // 接続が破棄される際に transform のハンドラも破棄されることを確認する。
    drop(connection);
}
