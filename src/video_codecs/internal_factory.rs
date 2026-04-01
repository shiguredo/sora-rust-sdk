use std::collections::HashMap;

use shiguredo_webrtc::{
    Environment, SdpVideoFormat, VideoCodecType, VideoDecoderFactory, VideoDecoderHandler,
    VideoEncoderFactory, VideoEncoderHandler, fuzzy_match_sdp_video_format,
};

use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

pub(crate) struct FactoryBackedVideoCodecCapability {
    implementation: VideoCodecImplementation,
    encoder_factory: VideoEncoderFactory,
    decoder_factory: VideoDecoderFactory,
}

impl FactoryBackedVideoCodecCapability {
    pub(crate) fn new(
        implementation: VideoCodecImplementation,
        encoder_factory: VideoEncoderFactory,
        decoder_factory: VideoDecoderFactory,
    ) -> Self {
        Self {
            implementation,
            encoder_factory,
            decoder_factory,
        }
    }

    pub(crate) fn get_supported_formats(&self, direction: CodecDirection) -> Vec<SdpVideoFormat> {
        match direction {
            CodecDirection::Encoder => self.encoder_factory.get_supported_formats(),
            CodecDirection::Decoder => self.decoder_factory.get_supported_formats(),
        }
    }

    fn formats_by_codec_type(
        &self,
        direction: CodecDirection,
        codec_type: VideoCodecType,
    ) -> Vec<SdpVideoFormat> {
        self.get_supported_formats(direction)
            .into_iter()
            .filter(|format| Self::codec_type_from_format(format) == Some(codec_type))
            .collect()
    }

    fn codec_type_from_format(format: &SdpVideoFormat) -> Option<VideoCodecType> {
        format
            .name()
            .ok()
            .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())
    }
}

impl VideoCodecCapability for FactoryBackedVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        self.implementation.clone()
    }

    fn resolve_sdp_format(
        &self,
        direction: CodecDirection,
        codec_type: VideoCodecType,
        parameters: &HashMap<String, String>,
        _scalability_mode: Option<&str>,
    ) -> Option<SdpVideoFormat> {
        let formats = self.formats_by_codec_type(direction, codec_type);
        let requested_name = formats.first()?.name().ok()?;
        let requested =
            SdpVideoFormat::new_with_parameters(requested_name.as_str(), parameters, &[]);
        fuzzy_match_sdp_video_format(&formats, requested.as_ref())
    }

    fn create_video_encoder(
        &self,
        format: &SdpVideoFormat,
    ) -> Option<Box<dyn VideoEncoderHandler>> {
        let env = Environment::new();
        let encoder = self
            .encoder_factory
            .create(env.as_ref(), format.as_ref())
            .map(Box::new)?;
        Some(encoder)
    }

    fn create_video_decoder(
        &self,
        format: &SdpVideoFormat,
    ) -> Option<Box<dyn VideoDecoderHandler>> {
        let env = Environment::new();
        let decoder = self
            .decoder_factory
            .create(env.as_ref(), format.as_ref())
            .map(Box::new)?;
        Some(decoder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiguredo_webrtc::ScalabilityMode;

    fn resolve_from_candidates(
        candidates: Vec<SdpVideoFormat>,
        parameters: &HashMap<String, String>,
        _scalability_mode: Option<&str>,
    ) -> Option<SdpVideoFormat> {
        let requested_name = candidates.first()?.name().ok()?;
        let requested =
            SdpVideoFormat::new_with_parameters(requested_name.as_str(), parameters, &[]);
        fuzzy_match_sdp_video_format(&candidates, requested.as_ref())
    }

    fn h264_format(packetization_mode: &str, profile_level_id: &str) -> SdpVideoFormat {
        SdpVideoFormat::new_with_parameters(
            "H264",
            &HashMap::from([
                (
                    String::from("packetization-mode"),
                    String::from(packetization_mode),
                ),
                (
                    String::from("profile-level-id"),
                    String::from(profile_level_id),
                ),
            ]),
            &[],
        )
    }

    #[test]
    fn resolve_prefers_more_exact_parameter_matches() {
        let candidates = vec![h264_format("1", "42e01f"), h264_format("0", "42e01f")];
        let request_parameters = HashMap::from([
            (String::from("packetization-mode"), String::from("1")),
            (String::from("profile-level-id"), String::from("42e01f")),
            (String::from("x-google-start-bitrate"), String::from("500")),
        ]);
        let mut resolved = resolve_from_candidates(candidates, &request_parameters, None)
            .expect("resolve_sdp_format_from_candidates should return a format");
        let resolved_parameters = resolved
            .parameters_mut()
            .iter()
            .collect::<HashMap<String, String>>();
        assert_eq!(
            resolved_parameters
                .get("packetization-mode")
                .map(String::as_str),
            Some("1"),
        );
    }

    #[test]
    fn resolve_h264_without_parameters_keeps_first_candidate() {
        let candidates = vec![h264_format("0", "640c1f"), h264_format("1", "42e01f")];
        let request_parameters = HashMap::new();
        let mut resolved = resolve_from_candidates(candidates, &request_parameters, None)
            .expect("resolve_sdp_format_from_candidates should return a format");
        let resolved_parameters = resolved
            .parameters_mut()
            .iter()
            .collect::<HashMap<String, String>>();
        assert_eq!(
            resolved_parameters
                .get("packetization-mode")
                .map(String::as_str),
            Some("0"),
        );
        assert_eq!(
            resolved_parameters
                .get("profile-level-id")
                .map(String::as_str),
            Some("640c1f"),
        );
    }

    #[test]
    fn resolve_keeps_first_candidate_on_tie() {
        let candidates = vec![
            SdpVideoFormat::new_with_parameters(
                "VP8",
                &HashMap::from([(String::from("x-google-start-bitrate"), String::from("500"))]),
                &[],
            ),
            SdpVideoFormat::new_with_parameters(
                "VP8",
                &HashMap::from([(String::from("x-google-start-bitrate"), String::from("1000"))]),
                &[],
            ),
        ];
        let request_parameters = HashMap::new();
        let mut resolved = resolve_from_candidates(candidates, &request_parameters, None)
            .expect("resolve_sdp_format_from_candidates should return a format");
        let resolved_parameters = resolved
            .parameters_mut()
            .iter()
            .collect::<HashMap<String, String>>();
        assert_eq!(
            resolved_parameters
                .get("x-google-start-bitrate")
                .map(String::as_str),
            Some("500"),
        );
    }

    #[test]
    fn resolve_ignores_requested_scalability_mode() {
        let candidates = vec![
            SdpVideoFormat::new_with_parameters(
                "VP9",
                &HashMap::from([(String::from("profile-id"), String::from("0"))]),
                &[ScalabilityMode::L1T1],
            ),
            SdpVideoFormat::new_with_parameters(
                "VP9",
                &HashMap::from([(String::from("profile-id"), String::from("1"))]),
                &[ScalabilityMode::L1T2],
            ),
        ];
        let request_parameters = HashMap::from([(String::from("profile-id"), String::from("1"))]);
        let mut resolved = resolve_from_candidates(candidates, &request_parameters, Some("L3T3"))
            .expect("resolve_sdp_format_from_candidates should return a format");
        let resolved_parameters = resolved
            .parameters_mut()
            .iter()
            .collect::<HashMap<String, String>>();
        assert_eq!(
            resolved_parameters.get("profile-id").map(String::as_str),
            Some("1"),
        );
    }
}
