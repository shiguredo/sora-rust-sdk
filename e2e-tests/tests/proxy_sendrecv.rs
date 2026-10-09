use std::collections::HashSet;
use std::time::Duration;

use e2e_tests::proxy::{ConnectTarget, ProxyHarness, parse_signaling_target};
use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, generate_channel_id, load_env,
    secret_key, signaling_urls, sum_video_stats_field_for_type, verify_video_stats_field_positive,
};
use sora_sdk::{Role, SoraConnectionContext};

fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

const MIN_PROXY_TRANSFER_BYTES_PER_DIRECTION: u64 = 4 * 1024;
const MIN_PROXY_TRANSFER_BYTES_TOTAL: u64 = 12 * 1024;
const MIN_RTP_BYTES_PER_CLIENT: u64 = 4 * 1024;

#[tokio::test]
async fn test_sendrecv_bidirectional_via_proxy() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let expected_signaling_targets: HashSet<ConnectTarget> = urls
        .iter()
        .filter_map(|url| parse_signaling_target(url))
        .collect();
    assert!(
        !expected_signaling_targets.is_empty(),
        "TEST_SIGNALING_URLS の解析に失敗しました"
    );
    let proxy = ProxyHarness::start(&urls)
        .await
        .expect("テスト用 Proxy の起動に失敗しました");
    let proxy_info = proxy.proxy_info();
    let channel_id = test_channel_id("sendrecv-via-proxy");

    let context1 = SoraConnectionContext::new().expect("クライアント 1 コンテキスト作成失敗");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 1 作成失敗");
    let (video_track1, audio_track1) =
        build_sender_tracks(&context1, &mut capturer1).expect("送信用トラック作成失敗");

    let mut builder1 =
        SoraTestConnection::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .proxy(proxy_info.clone())
            .data_channel_signaling(true)
            .disconnect_wait_timeout(Duration::from_secs(1));

    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }

    let mut client1 = builder1
        .connect()
        .expect("SoraTestConnection 1 の作成に失敗しました");
    client1
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の接続がタイムアウトしました");

    let context2 = SoraConnectionContext::new().expect("クライアント 2 コンテキスト作成失敗");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 2 作成失敗");
    let (video_track2, audio_track2) =
        build_sender_tracks(&context2, &mut capturer2).expect("送信用トラック作成失敗");

    let mut builder2 =
        SoraTestConnection::builder(context2, urls.clone(), channel_id, Role::SendRecv)
            .sender_video_track(video_track2)
            .sender_audio_track(audio_track2)
            .proxy(proxy_info)
            .data_channel_signaling(true)
            .disconnect_wait_timeout(Duration::from_secs(1))
            .ice_server_url_configurer(|server, urls| {
                for url in urls {
                    // 必ず TURN-TCP または TURN-TLS に接続してほしいので、transport=tcp を含む URL のみ追加する
                    if url.contains("transport=tcp") {
                        server.add_url(url);
                    }
                }
            });

    if let Some(token) = secret_key() {
        builder2 = builder2.metadata(build_metadata_with_access_token(&token));
    }

    let mut client2 = builder2
        .connect()
        .expect("SoraTestConnection 2 の作成に失敗しました");
    client2
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の接続がタイムアウトしました");
    client1
        .wait_for_video_track(Duration::from_secs(15))
        .await
        .expect("クライアント 1 の on_track 受信待機がタイムアウトしました");
    client2
        .wait_for_video_track(Duration::from_secs(15))
        .await
        .expect("クライアント 2 の on_track 受信待機がタイムアウトしました");

    client1
        .wait_video_outbound_packets_sent(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の outbound-rtp packetsSent が 0 より大きくなりませんでした");
    client2
        .wait_video_outbound_packets_sent(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の outbound-rtp packetsSent が 0 より大きくなりませんでした");
    client1
        .wait_video_inbound_packets_received(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の inbound-rtp の packetsReceived と framesDecoded が 0 より大きくなりませんでした");
    client2
        .wait_video_inbound_packets_received(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の inbound-rtp の packetsReceived と framesDecoded が 0 より大きくなりませんでした");

    client1
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "framesDecoded")
                    && sum_video_stats_field_for_type(stats, "outbound-rtp", "bytesSent")
                        >= MIN_RTP_BYTES_PER_CLIENT
                    && sum_video_stats_field_for_type(stats, "inbound-rtp", "bytesReceived")
                        >= MIN_RTP_BYTES_PER_CLIENT
            },
            Duration::from_secs(15),
        )
        .await
        .expect("クライアント 1 の stats が期待値に到達しませんでした");
    client2
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "framesDecoded")
                    && sum_video_stats_field_for_type(stats, "outbound-rtp", "bytesSent")
                        >= MIN_RTP_BYTES_PER_CLIENT
                    && sum_video_stats_field_for_type(stats, "inbound-rtp", "bytesReceived")
                        >= MIN_RTP_BYTES_PER_CLIENT
            },
            Duration::from_secs(15),
        )
        .await
        .expect("クライアント 2 の stats が期待値に到達しませんでした");

    client1
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の disconnect に失敗しました");
    client2
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の disconnect に失敗しました");
    assert!(
        proxy
            .wait_for_all_connections_closed(Duration::from_secs(5))
            .await,
        "Proxy 接続がクローズされませんでした: active_connection_count={}",
        proxy.active_connection_count()
    );

    // 2 クライアントが urls.len() + TURN 回以上接続しているはず
    let connect_targets = proxy.connect_targets();
    assert!(
        connect_targets.len() >= (urls.len() + 1) * 2,
        "Proxy の CONNECT 回数が不足しています: actual({}) >= expected({})",
        connect_targets.len(),
        (urls.len() + 1) * 2
    );
    let signaling_connect_count = connect_targets
        .iter()
        .filter(|target| expected_signaling_targets.contains(*target))
        .count();
    assert!(
        signaling_connect_count >= 2,
        "シグナリング宛先への CONNECT 回数が不足しています: signaling_connect_count={}",
        signaling_connect_count
    );
    let (downstream_to_upstream_bytes, upstream_to_downstream_bytes) = proxy.transferred_bytes();
    assert!(
        downstream_to_upstream_bytes >= MIN_PROXY_TRANSFER_BYTES_PER_DIRECTION,
        "Proxy の下流→上流バイト数が不足しています: bytes={}, min={}",
        downstream_to_upstream_bytes,
        MIN_PROXY_TRANSFER_BYTES_PER_DIRECTION
    );
    assert!(
        upstream_to_downstream_bytes >= MIN_PROXY_TRANSFER_BYTES_PER_DIRECTION,
        "Proxy の上流→下流バイト数が不足しています: bytes={}, min={}",
        upstream_to_downstream_bytes,
        MIN_PROXY_TRANSFER_BYTES_PER_DIRECTION
    );
    assert!(
        downstream_to_upstream_bytes + upstream_to_downstream_bytes
            >= MIN_PROXY_TRANSFER_BYTES_TOTAL,
        "Proxy の総転送バイト数が不足しています: downstream_to_upstream={}, upstream_to_downstream={}, total_min={}",
        downstream_to_upstream_bytes,
        upstream_to_downstream_bytes,
        MIN_PROXY_TRANSFER_BYTES_TOTAL
    );
}
