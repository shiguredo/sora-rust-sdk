//! `SoraConnectionContextConfig::environment` がエンコーダー生成まで届くことを検証するためのヘルパー。
//!
//! `create_video_encoder()` に渡された `Environment` が上位レイヤーで作成した `Environment` と一致
//! しているかを確認するために、カスタムエンコーダーを定義する。
use std::sync::mpsc::{Sender, channel};

use shiguredo_webrtc::{
    Environment, EnvironmentRef, SdpVideoFormat, SdpVideoFormatRef, VideoDecoder, VideoEncoder,
};
use sora_sdk::{CodecDirection, VideoCodecCapability, VideoCodecImplementation};

/// 別の [VideoCodecCapability] に委譲しつつ、`create_video_encoder` に渡された
/// [EnvironmentRef] を [EnvironmentRef::to_owned] でコピーして記録する capability。
///
/// `create_video_encoder` は libwebrtc のスレッドから呼ばれるため、呼び出しの場で
/// 検証せずに送信だけを行い、送信した [Environment] の検証はテスト本体で行う。
struct VideoCodecEnvironmentRecorder {
    /// 実際にエンコーダー/デコーダーを生成する capability。
    inner: Box<dyn VideoCodecCapability>,
    /// 記録した [Environment] をテスト本体へ送るチャネル。
    recorded: Sender<Environment>,
}

/// 委譲先の capability を指定して [VideoCodecCapability] と記録を受け取るチャネルを生成する。
pub fn recording_video_codec_capability(
    inner: Box<dyn VideoCodecCapability>,
) -> (
    Box<dyn VideoCodecCapability>,
    std::sync::mpsc::Receiver<Environment>,
) {
    let (recorded, receiver) = channel();
    let capability = VideoCodecEnvironmentRecorder { inner, recorded };
    (Box::new(capability), receiver)
}

impl VideoCodecCapability for VideoCodecEnvironmentRecorder {
    fn get_implementation(&self) -> VideoCodecImplementation {
        self.inner.get_implementation()
    }

    fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat> {
        self.inner.get_supported_formats(direction)
    }

    fn create_video_encoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoEncoder> {
        let _ = self.recorded.send(env.to_owned());
        self.inner.create_video_encoder(env, format)
    }

    fn create_video_decoder(
        &self,
        env: EnvironmentRef<'_>,
        format: SdpVideoFormatRef<'_>,
    ) -> Option<VideoDecoder> {
        self.inner.create_video_decoder(env, format)
    }
}
