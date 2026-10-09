//! コーデックの方向 (エンコード/デコード) を表す共有型。
use nojson::{JsonParseError, RawJsonValue};

/// コーデックの方向 (エンコード/デコード)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecDirection {
    /// エンコード方向。
    Encoder,
    /// デコード方向。
    Decoder,
}

impl CodecDirection {
    /// デバッグ表示用の文字列表現 (`"Encoder"` / `"Decoder"`) を返す。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Encoder => "Encoder",
            Self::Decoder => "Decoder",
        }
    }

    /// ラベル用の小文字表記 (`"encoder"` / `"decoder"`) を返す。
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Encoder => "encoder",
            Self::Decoder => "decoder",
        }
    }
}

impl<'text, 'raw> TryFrom<RawJsonValue<'text, 'raw>> for CodecDirection {
    type Error = JsonParseError;

    fn try_from(value: RawJsonValue<'text, 'raw>) -> std::result::Result<Self, Self::Error> {
        let direction_text: String = value.try_into()?;
        match direction_text.as_str() {
            "Encoder" => Ok(Self::Encoder),
            "Decoder" => Ok(Self::Decoder),
            _ => Err(value.invalid(format!("unsupported codec direction: {direction_text}"))),
        }
    }
}
