use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, generate_channel_id, load_env,
    secret_key, signaling_urls, sum_video_stats_field_for_type, verify_video_stats_field_positive,
};
use shiguredo_webrtc::{
    FrameTransformerHandler, TransformableFrame, TransformableVideoFrame, VideoFrameMetadata,
    VideoRotation,
};
use sora_sdk::{Role, SoraConnectionContext};

/// テスト用のチャンネル ID を生成する (suffix 付き)
fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

/// フレーム変換の呼び出し回数を数え、任意でドロップやデータ書き換えを行う transform。
struct CountingTransform {
    count: Arc<AtomicUsize>,
    drop_all: bool,
    corrupt: bool,
}

impl FrameTransformerHandler for CountingTransform {
    fn transform(&self, mut frame: TransformableFrame) -> Option<TransformableFrame> {
        self.count.fetch_add(1, Ordering::Relaxed);
        if self.corrupt {
            let len = frame.data().len();
            let zeros = vec![0u8; len];
            frame.set_data(&zeros);
        }
        if self.drop_all { None } else { Some(frame) }
    }
}

fn counting_transform(
    count: Arc<AtomicUsize>,
    drop_all: bool,
    corrupt: bool,
) -> Box<dyn FrameTransformerHandler + Send> {
    Box::new(CountingTransform {
        count,
        drop_all,
        corrupt,
    })
}

/// フレームのメタデータを書き換え (set_metadata / set_capture_time) して読み戻し、
/// 書き込み経路が動作することを確認する transform。
struct MetadataWriteTransform {
    count: Arc<AtomicUsize>,
}

impl FrameTransformerHandler for MetadataWriteTransform {
    fn transform(&self, frame: TransformableFrame) -> Option<TransformableFrame> {
        let mut video_frame = match TransformableVideoFrame::try_from(frame) {
            Ok(frame) => frame,
            Err(_) => return None,
        };
        self.count.fetch_add(1, Ordering::Relaxed);

        // metadata() はディープコピーを返すため、書き換えた内容を
        // set_metadata で反映できることを確認する。
        let mut metadata = video_frame.metadata();
        let original_rotation = metadata.rotation();
        metadata.set_rotation(if original_rotation == VideoRotation::R0 {
            VideoRotation::R90
        } else {
            VideoRotation::R0
        });
        video_frame.set_metadata(&metadata);
        let written_metadata: VideoFrameMetadata = video_frame.metadata();
        if written_metadata.rotation() == original_rotation {
            return None;
        }

        // キャプチャ時間の設定経路が利用できる場合は、設定して読み戻す。
        if video_frame.can_set_capture_time() {
            video_frame.set_capture_time(Some(1234567));
            if video_frame.capture_time() != Some(1234567) {
                return None;
            }
        }

        Some(video_frame.into_base())
    }
}

fn metadata_write_transform(count: Arc<AtomicUsize>) -> Box<dyn FrameTransformerHandler + Send> {
    Box::new(MetadataWriteTransform { count })
}

/// 2 つの SendRecv クライアントが送受信にパススルー transform を設定し、
/// ハンドラへフレームが届いてメディアが正常に流れることを確認するテスト。
#[tokio::test]
async fn test_video_transform_passthrough() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要です");
    let channel_id = test_channel_id("video-transform-passthrough");

    let context1 =
        SoraConnectionContext::new().expect("クライアント 1 の context 作成に失敗しました");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 1 の作成に失敗しました");
    let (video_track1, audio_track1) = build_sender_tracks(&context1, &mut capturer1)
        .expect("クライアント 1 の送信用トラック作成に失敗しました");
    let sender_count1 = Arc::new(AtomicUsize::new(0));
    let receiver_count1 = Arc::new(AtomicUsize::new(0));

    let mut builder1 =
        SoraTestConnection::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .sender_video_transform(counting_transform(sender_count1.clone(), false, false))
            .receiver_video_transform(counting_transform(receiver_count1.clone(), false, false))
            .data_channel_signaling(true);
    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }
    let mut client1 = builder1
        .connect()
        .expect("SoraTestConnection 1 の作成に失敗しました");
    client1
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の接続待機がタイムアウトしました");

    let context2 =
        SoraConnectionContext::new().expect("クライアント 2 の context 作成に失敗しました");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 2 の作成に失敗しました");
    let (video_track2, audio_track2) = build_sender_tracks(&context2, &mut capturer2)
        .expect("クライアント 2 の送信用トラック作成に失敗しました");
    let sender_count2 = Arc::new(AtomicUsize::new(0));
    let receiver_count2 = Arc::new(AtomicUsize::new(0));

    let mut builder2 = SoraTestConnection::builder(context2, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track2)
        .sender_audio_track(audio_track2)
        .sender_video_transform(counting_transform(sender_count2.clone(), false, false))
        .receiver_video_transform(counting_transform(receiver_count2.clone(), false, false))
        .data_channel_signaling(true);
    if let Some(token) = secret_key() {
        builder2 = builder2.metadata(build_metadata_with_access_token(&token));
    }
    let mut client2 = builder2
        .connect()
        .expect("SoraTestConnection 2 の作成に失敗しました");
    client2
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の接続待機がタイムアウトしました");

    client1
        .wait_for_track_kind("video", Duration::from_secs(15))
        .await
        .expect("クライアント 1 が video トラックを受信できませんでした");
    client2
        .wait_for_track_kind("video", Duration::from_secs(15))
        .await
        .expect("クライアント 2 が video トラックを受信できませんでした");

    client1
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "framesDecoded")
            },
            Duration::from_secs(10),
        )
        .await
        .expect("クライアント 1 の送受信 stats が条件を満たしませんでした");
    client2
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "framesDecoded")
            },
            Duration::from_secs(10),
        )
        .await
        .expect("クライアント 2 の送受信 stats が条件を満たしませんでした");

    assert!(
        sender_count1.load(Ordering::Relaxed) > 0,
        "クライアント 1 の送信 transform が呼び出されませんでした"
    );
    assert!(
        receiver_count1.load(Ordering::Relaxed) > 0,
        "クライアント 1 の受信 transform が呼び出されませんでした"
    );
    assert!(
        sender_count2.load(Ordering::Relaxed) > 0,
        "クライアント 2 の送信 transform が呼び出されませんでした"
    );
    assert!(
        receiver_count2.load(Ordering::Relaxed) > 0,
        "クライアント 2 の受信 transform が呼び出されませんでした"
    );

    client1
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の切断に失敗しました");
    client2
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の切断に失敗しました");
}

/// 受信 transform が全フレームをドロップし、パケットは届くがデコードされないことを確認するテスト。
#[tokio::test]
async fn test_video_transform_drop_all() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要です");
    let channel_id = test_channel_id("video-transform-drop-all");

    let context1 =
        SoraConnectionContext::new().expect("クライアント 1 の context 作成に失敗しました");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 1 の作成に失敗しました");
    let (video_track1, audio_track1) = build_sender_tracks(&context1, &mut capturer1)
        .expect("クライアント 1 の送信用トラック作成に失敗しました");

    let mut builder1 =
        SoraTestConnection::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .data_channel_signaling(true);
    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }
    let mut client1 = builder1
        .connect()
        .expect("SoraTestConnection 1 の作成に失敗しました");
    client1
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の接続待機がタイムアウトしました");

    let context2 =
        SoraConnectionContext::new().expect("クライアント 2 の context 作成に失敗しました");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 2 の作成に失敗しました");
    let (video_track2, audio_track2) = build_sender_tracks(&context2, &mut capturer2)
        .expect("クライアント 2 の送信用トラック作成に失敗しました");
    let drop_count = Arc::new(AtomicUsize::new(0));

    let mut builder2 = SoraTestConnection::builder(context2, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track2)
        .sender_audio_track(audio_track2)
        .receiver_video_transform(counting_transform(drop_count.clone(), true, false))
        .data_channel_signaling(true);
    if let Some(token) = secret_key() {
        builder2 = builder2.metadata(build_metadata_with_access_token(&token));
    }
    let mut client2 = builder2
        .connect()
        .expect("SoraTestConnection 2 の作成に失敗しました");
    client2
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の接続待機がタイムアウトしました");

    client2
        .wait_for_track_kind("video", Duration::from_secs(15))
        .await
        .expect("クライアント 2 が video トラックを受信できませんでした");

    client2
        .wait_stats(
            |stats| verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived"),
            Duration::from_secs(10),
        )
        .await
        .expect("クライアント 2 がパケットを受信できませんでした");
    assert!(
        drop_count.load(Ordering::Relaxed) > 0,
        "クライアント 2 の受信 transform が呼び出されませんでした"
    );

    // フレームが全てドロップされデコードされないことを確認する。
    client2
        .wait_stats(
            |stats| sum_video_stats_field_for_type(stats, "inbound-rtp", "framesDecoded") == 0,
            Duration::from_secs(5),
        )
        .await
        .expect("ドロップ後にデコードされたフレームが存在します");

    client1
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の切断に失敗しました");
    client2
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の切断に失敗しました");
}

/// 送信 transform がフレームデータを書き換え、書き換え内容が
/// リモート側のデコード結果に反映されることを確認するテスト。
#[tokio::test]
async fn test_video_transform_corrupt_data() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要です");
    let channel_id = test_channel_id("video-transform-corrupt");

    let context1 =
        SoraConnectionContext::new().expect("クライアント 1 の context 作成に失敗しました");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 1 の作成に失敗しました");
    let (video_track1, audio_track1) = build_sender_tracks(&context1, &mut capturer1)
        .expect("クライアント 1 の送信用トラック作成に失敗しました");
    let corrupt_count = Arc::new(AtomicUsize::new(0));

    let mut builder1 =
        SoraTestConnection::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .sender_video_transform(counting_transform(corrupt_count.clone(), false, true))
            .data_channel_signaling(true);
    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }
    let mut client1 = builder1
        .connect()
        .expect("SoraTestConnection 1 の作成に失敗しました");
    client1
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の接続待機がタイムアウトしました");

    let context2 =
        SoraConnectionContext::new().expect("クライアント 2 の context 作成に失敗しました");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 2 の作成に失敗しました");
    let (video_track2, audio_track2) = build_sender_tracks(&context2, &mut capturer2)
        .expect("クライアント 2 の送信用トラック作成に失敗しました");

    let mut builder2 = SoraTestConnection::builder(context2, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track2)
        .sender_audio_track(audio_track2)
        .data_channel_signaling(true);
    if let Some(token) = secret_key() {
        builder2 = builder2.metadata(build_metadata_with_access_token(&token));
    }
    let mut client2 = builder2
        .connect()
        .expect("SoraTestConnection 2 の作成に失敗しました");
    client2
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の接続待機がタイムアウトしました");

    client2
        .wait_for_track_kind("video", Duration::from_secs(15))
        .await
        .expect("クライアント 2 が video トラックを受信できませんでした");

    client2
        .wait_stats(
            |stats| verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived"),
            Duration::from_secs(10),
        )
        .await
        .expect("クライアント 2 がパケットを受信できませんでした");
    assert!(
        corrupt_count.load(Ordering::Relaxed) > 0,
        "クライアント 1 の送信 transform が呼び出されませんでした"
    );

    // データを全てゼロに書き換えたためデコードに失敗し、
    // framesDecoded が 0 のままであることを確認する。
    client2
        .wait_stats(
            |stats| sum_video_stats_field_for_type(stats, "inbound-rtp", "framesDecoded") == 0,
            Duration::from_secs(5),
        )
        .await
        .expect("書き換えたフレームがデコードされました");

    client1
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の切断に失敗しました");
    client2
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の切断に失敗しました");
}

/// フレーム変換ハンドラ内でメタデータを書き換え (set_metadata) して読み戻し、
/// 書き込み経路が動作することを確認するテスト。
#[tokio::test]
async fn test_video_transform_write_metadata() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要です");
    let channel_id = test_channel_id("video-transform-write-metadata");

    let context1 =
        SoraConnectionContext::new().expect("クライアント 1 の context 作成に失敗しました");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 1 の作成に失敗しました");
    let (video_track1, audio_track1) = build_sender_tracks(&context1, &mut capturer1)
        .expect("クライアント 1 の送信用トラック作成に失敗しました");
    let write_count = Arc::new(AtomicUsize::new(0));

    let mut builder1 =
        SoraTestConnection::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .sender_video_transform(metadata_write_transform(write_count.clone()))
            .data_channel_signaling(true);
    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }
    let mut client1 = builder1
        .connect()
        .expect("SoraTestConnection 1 の作成に失敗しました");
    client1
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の接続待機がタイムアウトしました");

    let context2 =
        SoraConnectionContext::new().expect("クライアント 2 の context 作成に失敗しました");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 2 の作成に失敗しました");
    let (video_track2, audio_track2) = build_sender_tracks(&context2, &mut capturer2)
        .expect("クライアント 2 の送信用トラック作成に失敗しました");

    let mut builder2 = SoraTestConnection::builder(context2, urls, channel_id, Role::SendRecv)
        .sender_video_track(video_track2)
        .sender_audio_track(audio_track2)
        .data_channel_signaling(true);
    if let Some(token) = secret_key() {
        builder2 = builder2.metadata(build_metadata_with_access_token(&token));
    }
    let mut client2 = builder2
        .connect()
        .expect("SoraTestConnection 2 の作成に失敗しました");
    client2
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の接続待機がタイムアウトしました");

    client2
        .wait_for_track_kind("video", Duration::from_secs(15))
        .await
        .expect("クライアント 2 が video トラックを受信できませんでした");
    client2
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "framesDecoded")
            },
            Duration::from_secs(10),
        )
        .await
        .expect("クライアント 2 の受信 stats が条件を満たしませんでした");

    // 書き換え (set_metadata) がフレームごとに実行されたことを確認する。
    assert!(
        write_count.load(Ordering::Relaxed) > 0,
        "メタデータ書き換え transform が呼び出されませんでした"
    );

    client1
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の切断に失敗しました");
    client2
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の切断に失敗しました");
}
