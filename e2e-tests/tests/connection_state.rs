use std::time::Duration;

use e2e_tests::{
    SoraTestConnection, SoraTestEvent, build_metadata_with_access_token, generate_channel_id,
    load_env, secret_key, signaling_urls,
};
use shiguredo_webrtc::{
    IceConnectionState, IceGatheringState, PeerConnectionState, SignalingState,
};
use sora_sdk::{Role, SoraConnectionContext};

/// WebRTC の状態変化のコールバックが実際の接続で通知されることを確認する。
///
/// 状態変化のイベントが SoraConnectionEventHandler まで届くことは
/// 実際に接続しなければ確認できないため、Sora サーバーに接続して確認する。
#[tokio::test]
async fn test_connection_state_change() {
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

    // シグナリングは offer を適用して answer を返す過程で Stable まで遷移する。
    connection
        .wait_for_event(
            |event| {
                matches!(
                    event,
                    SoraTestEvent::SignalingStateChange {
                        state: SignalingState::Stable
                    }
                )
            },
            Duration::from_secs(10),
        )
        .await
        .expect("signaling state の変化が通知されませんでした");

    // 候補収集は接続が確立するより先に始まる。
    connection
        .wait_for_event(
            |event| {
                matches!(
                    event,
                    SoraTestEvent::IceGatheringStateChange {
                        state: IceGatheringState::Gathering
                    }
                )
            },
            Duration::from_secs(10),
        )
        .await
        .expect("ice gathering state の変化が通知されませんでした");

    // ICE の接続が確立すると Connected になる。
    connection
        .wait_for_event(
            |event| {
                matches!(
                    event,
                    SoraTestEvent::IceConnectionStateChange {
                        state: IceConnectionState::Connected
                    }
                )
            },
            Duration::from_secs(10),
        )
        .await
        .expect("ice connection state の変化が通知されませんでした");

    // ICE に続いて DTLS の接続も確立すると Connected になる。
    connection
        .wait_for_event(
            |event| {
                matches!(
                    event,
                    SoraTestEvent::ConnectionStateChange {
                        state: PeerConnectionState::Connected
                    }
                )
            },
            Duration::from_secs(10),
        )
        .await
        .expect("connection state の変化が通知されませんでした");

    connection
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("disconnect に失敗しました");
}
