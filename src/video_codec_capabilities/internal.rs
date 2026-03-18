use std::collections::HashMap;

use shiguredo_webrtc::{
    Environment, SdpVideoFormat, VideoCodecType, VideoDecoderFactory, VideoDecoderHandler,
    VideoEncoderFactory, VideoEncoderHandler,
};

use crate::video_codec_capability::{
    CodecDirection, VideoCodecCapability, VideoCodecImplementation,
};

pub struct InternalVideoCodecCapability {
    encoder_factory: VideoEncoderFactory,
    decoder_factory: VideoDecoderFactory,
}

impl InternalVideoCodecCapability {
    pub fn new() -> Self {
        let encoder_factory = VideoEncoderFactory::builtin();
        let decoder_factory = VideoDecoderFactory::builtin();
        Self {
            encoder_factory,
            decoder_factory,
        }
    }

    fn encoder_formats_by_codec_type(&self, codec_type: VideoCodecType) -> Vec<SdpVideoFormat> {
        self.encoder_factory
            .get_supported_formats()
            .into_iter()
            .filter(|format| {
                format
                    .name()
                    .ok()
                    .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())
                    == Some(codec_type)
            })
            .collect()
    }

    fn decoder_formats_by_codec_type(&self, codec_type: VideoCodecType) -> Vec<SdpVideoFormat> {
        self.decoder_factory
            .get_supported_formats()
            .into_iter()
            .filter(|format| {
                format
                    .name()
                    .ok()
                    .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())
                    == Some(codec_type)
            })
            .collect()
    }

    fn matches_requested_format(
        &self,
        format: &mut SdpVideoFormat,
        parameters: &HashMap<String, String>,
        scalability_mode: Option<&str>,
    ) -> bool {
        let format_parameters = format
            .parameters_mut()
            .iter()
            .collect::<HashMap<String, String>>();
        if !parameters
            .iter()
            .all(|(k, v)| format_parameters.get(k).is_some_and(|value| value == v))
        {
            return false;
        }
        let Some(scalability_mode_text) = scalability_mode else {
            return true;
        };
        format.scalability_modes().iter().any(|mode| {
            mode.as_str()
                .is_ok_and(|mode_text| mode_text == scalability_mode_text)
        })
    }

    fn resolve_from_formats(
        &self,
        mut formats: Vec<SdpVideoFormat>,
        parameters: &HashMap<String, String>,
        scalability_mode: Option<&str>,
    ) -> Option<SdpVideoFormat> {
        let fallback = formats.first().cloned()?;
        for format in &mut formats {
            if self.matches_requested_format(format, parameters, scalability_mode) {
                return Some(format.clone());
            }
        }
        Some(fallback)
    }
}

impl Default for InternalVideoCodecCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoCodecCapability for InternalVideoCodecCapability {
    fn get_implementation(&self) -> VideoCodecImplementation {
        VideoCodecImplementation::new("internal", "WebRTC built-in VideoCodecFactory")
    }

    fn is_supported(&self, direction: CodecDirection, codec_type: VideoCodecType) -> bool {
        match direction {
            CodecDirection::Encoder => !self.encoder_formats_by_codec_type(codec_type).is_empty(),
            CodecDirection::Decoder => !self.decoder_formats_by_codec_type(codec_type).is_empty(),
        }
    }

    fn resolve_sdp_format(
        &self,
        direction: CodecDirection,
        codec_type: VideoCodecType,
        parameters: &HashMap<String, String>,
        scalability_mode: Option<&str>,
    ) -> Option<SdpVideoFormat> {
        let formats = match direction {
            CodecDirection::Encoder => self.encoder_formats_by_codec_type(codec_type),
            CodecDirection::Decoder => self.decoder_formats_by_codec_type(codec_type),
        };
        self.resolve_from_formats(formats, parameters, scalability_mode)
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

    #[test]
    fn internal_capability_has_expected_implementation_name() {
        let capability = InternalVideoCodecCapability::new();
        assert_eq!(capability.get_implementation().name(), "internal");
    }

    #[test]
    fn internal_capability_creates_supported_encoder_and_decoder() {
        let capability = InternalVideoCodecCapability::new();
        let encoder_codec_types = capability
            .encoder_factory
            .get_supported_formats()
            .iter()
            .filter_map(|format| {
                format
                    .name()
                    .ok()
                    .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())
            })
            .collect::<Vec<_>>();
        for codec_type in &encoder_codec_types {
            let format = capability
                .resolve_sdp_format(CodecDirection::Encoder, *codec_type, &HashMap::new(), None)
                .expect("encoder format resolve failed");
            assert!(
                capability.create_video_encoder(&format).is_some(),
                "encoder must be created for {codec_type:?}",
            );
        }
        let decoder_codec_types = capability
            .decoder_factory
            .get_supported_formats()
            .iter()
            .filter_map(|format| {
                format
                    .name()
                    .ok()
                    .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())
            })
            .collect::<Vec<_>>();
        for codec_type in &decoder_codec_types {
            let format = capability
                .resolve_sdp_format(CodecDirection::Decoder, *codec_type, &HashMap::new(), None)
                .expect("decoder format resolve failed");
            assert!(
                capability.create_video_decoder(&format).is_some(),
                "decoder must be created for {codec_type:?}",
            );
        }
    }

    #[test]
    fn resolve_sdp_format_with_unknown_scalability_mode_falls_back() {
        let capability = InternalVideoCodecCapability::new();
        let codec_type = capability
            .encoder_factory
            .get_supported_formats()
            .iter()
            .find_map(|format| {
                format
                    .name()
                    .ok()
                    .and_then(|name| VideoCodecType::try_from(name.as_str()).ok())
            })
            .expect("encoder が少なくとも 1 codec をサポートしている必要があります");
        let resolved = capability.resolve_sdp_format(
            CodecDirection::Encoder,
            codec_type,
            &HashMap::new(),
            Some("INVALID_MODE"),
        );
        assert!(resolved.is_some());
    }
}
