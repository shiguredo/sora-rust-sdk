//! SoraConnectionContext と関連リソースの管理。
use std::sync::{Arc, Mutex};

use shiguredo_webrtc::{
    AudioDecoderFactory, AudioDeviceModule, AudioDeviceModuleAudioLayer, AudioEncoderFactory,
    AudioProcessingBuilder, AudioTrack, AudioTrackSource, ConnectionContext, Environment,
    PeerConnectionFactory, PeerConnectionFactoryDependencies, RtcEventLogFactory, Thread,
    VideoDecoderFactory, VideoEncoderFactory, VideoTrack, VideoTrackSource, random_string,
};

use crate::error::Result;
use crate::video_codec::{SoraVideoDecoderFactory, SoraVideoEncoderFactory};
use crate::video_codec_capability::VideoCodecCapability;
use crate::video_codec_preference::{VideoCodecPreference, validate_video_codec_preference};
use crate::video_codecs::internal::InternalVideoCodecCapability;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use crate::video_codecs::internal_apple::InternalAppleVideoCodecCapability;

/// AudioDeviceModule の設定。
#[derive(Clone, Default)]
pub enum AdmConfig {
    /// Dummy の AudioDeviceModule を使用する。
    #[default]
    NoAudioDevice,
    /// PlatformDefault の AudioDeviceModule を使用する。
    UseBuiltIn,
    /// 外部の AudioDeviceModule を使用する。
    UseExternal(shiguredo_webrtc::AudioDeviceModule),
}

/// SoraConnectionContext の設定。
pub struct SoraConnectionContextConfig {
    /// AudioDeviceModule の設定。デフォルトは [AdmConfig::NoAudioDevice]。
    pub adm_config: AdmConfig,
    /// コーデックごとに使用する実装を指定する優先設定。
    ///
    /// 各エントリ（[PreferenceCodec](crate::video_codec_preference::PreferenceCodec)）は、
    /// 特定の方向（エンコード/デコード）とコーデック種別（VP8/VP9/H264/H265/AV1）に対して、
    /// どの [VideoCodecCapability] 実装を使うかを指定する。
    /// [Default::default()] では [InternalVideoCodecCapability] から自動生成され、
    /// macOS/iOS ではさらに `InternalAppleVideoCodecCapability` がマージされる。
    pub video_codec_preference: VideoCodecPreference,
    /// エンコーダー/デコーダーを実際に生成する capability 実装のリスト。
    ///
    /// [Default::default()] では [InternalVideoCodecCapability] が含まれ、
    /// macOS/iOS では `InternalAppleVideoCodecCapability` も追加される。
    pub video_codec_capabilities: Vec<Box<dyn VideoCodecCapability>>,
}

impl Default for SoraConnectionContextConfig {
    fn default() -> Self {
        let mut video_codec_preference = VideoCodecPreference::default();
        let mut video_codec_capabilities = Vec::new();

        let internal_capability: Box<dyn VideoCodecCapability> =
            Box::new(InternalVideoCodecCapability::new());
        video_codec_preference.merge(&VideoCodecPreference::new_from_capability(
            internal_capability.as_ref(),
        ));
        video_codec_capabilities.push(internal_capability);

        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if let Some(internal_apple_capability) = InternalAppleVideoCodecCapability::new() {
            let internal_apple_capability: Box<dyn VideoCodecCapability> =
                Box::new(internal_apple_capability);
            video_codec_preference.merge(&VideoCodecPreference::new_from_capability(
                internal_apple_capability.as_ref(),
            ));
            video_codec_capabilities.push(internal_apple_capability);
        }

        Self {
            adm_config: AdmConfig::default(),
            video_codec_preference,
            video_codec_capabilities,
        }
    }
}

/// PeerConnectionFactory と関連リソースをまとめて管理する。
pub struct SoraConnectionContext {
    // フィールドの宣言順には依存関係があるため、変更してはならない:
    //
    // 1. ConnectionContext::~ConnectionContext() は DCHECK(signaling_thread_IsCurrent()) になっており
    //    シグナリングスレッド以外のスレッドから呼び出すと SIGABRT する。
    //
    // 2. `factory` の実体はメンバーに ConnectionContext を保持している。
    //
    // 3. `factory` (PeerConnectionFactory) の実体は C ラッパー側で PeerConnectionFactoryProxy に
    //    ラップされており、Proxy デストラクタは実体の PeerConnectionFactory の破棄を
    //    signaling スレッドへの BlockingCall で行うようになっている。
    //
    // 4. 2, 3 の理由によって、factory 経由で ConnectionContext を破棄すれば 1 の問題は発生しない。
    //
    // 5. しかし SoraConnectionContext のメンバーとして connection_context を factory より後に
    //    配置すると、factory 破棄時に参照が残ることになるため、Rust 側のスレッド上で
    //    ConnectionContext::~ConnectionContext() が呼ばれ、DCHECK(signaling_thread_->IsCurrent())
    //    に失敗して SIGABRT する。
    //
    // したがって connection_context を factory より前に配置し、
    // Rust 側の参照を先に手放すことで、factory 破棄時の signaling スレッド上での
    // 解放が ConnectionContext の最後の C++ 参照になるようにしている。
    connection_context: ConnectionContext,
    factory: PeerConnectionFactory,
    _network: Thread,
    _worker: Thread,
    _signaling: Thread,
}

impl SoraConnectionContext {
    /// [SoraConnectionContextConfig::default] で [SoraConnectionContext] を生成する。
    ///
    /// [SoraConnectionContext::new_with_config] のショートカット。
    pub fn new() -> Result<Arc<Self>> {
        Self::new_with_config(SoraConnectionContextConfig::default())
    }

    /// 指定した設定でコンテキストを生成する。
    pub fn new_with_config(config: SoraConnectionContextConfig) -> Result<Arc<Self>> {
        let SoraConnectionContextConfig {
            adm_config,
            video_codec_preference,
            video_codec_capabilities,
        } = config;
        validate_video_codec_preference(&video_codec_preference, &video_codec_capabilities)?;

        let shared_video_codec_capabilities = Arc::new(Mutex::new(video_codec_capabilities));
        let video_encoder_factory =
            VideoEncoderFactory::new_with_handler(Box::new(SoraVideoEncoderFactory::new(
                video_codec_preference.clone(),
                shared_video_codec_capabilities.clone(),
            )));
        let video_decoder_factory = VideoDecoderFactory::new_with_handler(Box::new(
            SoraVideoDecoderFactory::new(video_codec_preference, shared_video_codec_capabilities),
        ));

        let env = Environment::new();
        let mut network = Thread::new_with_socket_server();
        let mut worker = Thread::new();
        let mut signaling = Thread::new();
        network.start();
        worker.start();
        signaling.start();

        let mut deps = PeerConnectionFactoryDependencies::new();
        deps.set_network_thread(&network);
        deps.set_worker_thread(&worker);
        deps.set_signaling_thread(&signaling);
        let event_log = RtcEventLogFactory::new();
        deps.set_event_log_factory(event_log);
        match adm_config {
            AdmConfig::NoAudioDevice => {
                let adm = AudioDeviceModule::new(&env, AudioDeviceModuleAudioLayer::Dummy)?;
                deps.set_audio_device_module(&adm);
            }
            AdmConfig::UseBuiltIn => {
                let adm =
                    AudioDeviceModule::new(&env, AudioDeviceModuleAudioLayer::PlatformDefault)?;
                deps.set_audio_device_module(&adm);
            }
            AdmConfig::UseExternal(external_adm) => {
                deps.set_audio_device_module(&external_adm);
            }
        };
        let audio_enc = AudioEncoderFactory::builtin();
        let audio_dec = AudioDecoderFactory::builtin();
        deps.set_audio_encoder_factory(&audio_enc);
        deps.set_audio_decoder_factory(&audio_dec);
        deps.set_video_encoder_factory(video_encoder_factory);
        deps.set_video_decoder_factory(video_decoder_factory);
        let apb = AudioProcessingBuilder::new_builtin();
        deps.set_audio_processing_builder(apb);
        deps.enable_media();

        let (factory, connection_context) =
            PeerConnectionFactory::create_modular_with_context(&mut deps)?;
        Ok(Arc::new(Self {
            factory,
            connection_context,
            _network: network,
            _worker: worker,
            _signaling: signaling,
        }))
    }

    /// AudioTrackSource を作成する。
    pub fn create_audio_source(&self) -> Result<AudioTrackSource> {
        Ok(self.factory.create_audio_source()?)
    }

    /// AudioTrack を作成する。
    pub fn create_audio_track(&self, source: &AudioTrackSource) -> Result<AudioTrack> {
        let track_id = random_string(16);
        Ok(self.factory.create_audio_track(source, &track_id)?)
    }

    /// VideoTrack を作成する。
    pub fn create_video_track(&self, source: &VideoTrackSource) -> Result<VideoTrack> {
        let track_id = random_string(16);
        Ok(self.factory.create_video_track(source, &track_id)?)
    }

    pub(crate) fn factory(&self) -> &PeerConnectionFactory {
        &self.factory
    }

    pub(crate) fn connection_context(&self) -> &ConnectionContext {
        &self.connection_context
    }
}

unsafe impl Send for SoraConnectionContext {}
// SAFETY: PeerConnectionFactoryInterface の実体はシーケンシャルにする Proxy 経由で
// アクセスするためスレッドセーフに使用できる。
// ref: https://source.chromium.org/chromium/chromium/src/+/main:third_party/webrtc/pc/peer_connection_factory_proxy.h;l=32-59;drc=ef55be496e45889ace33ace4b05094ca19cb499b
unsafe impl Sync for SoraConnectionContext {}

#[cfg(all(test, any(target_os = "macos", target_os = "ios")))]
mod tests {
    use super::*;
    use crate::video_codec_capability::CodecDirection;
    use shiguredo_webrtc::VideoCodecType;

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    #[test]
    fn default_config_prefers_internal_apple_for_supported_codecs() {
        let config = SoraConnectionContextConfig::default();
        let Some(internal_apple_capability) = config
            .video_codec_capabilities
            .iter()
            .find(|cap| cap.get_implementation().name() == "internal-apple")
        else {
            return;
        };
        let internal_apple_implementation = internal_apple_capability.get_implementation();

        for codec_type in [
            VideoCodecType::Vp8,
            VideoCodecType::Vp9,
            VideoCodecType::H264,
            VideoCodecType::H265,
            VideoCodecType::Av1,
        ] {
            for direction in [CodecDirection::Encoder, CodecDirection::Decoder] {
                if !internal_apple_capability.is_supported(direction, codec_type) {
                    continue;
                }
                let preference = config
                    .video_codec_preference
                    .find(direction, codec_type)
                    .expect("preference エントリは必ず存在する");
                assert_eq!(
                    preference.implementation(),
                    &internal_apple_implementation,
                    "internal-apple が {direction:?} {codec_type:?} で優先されなければなりません",
                );
            }
        }
    }
}
