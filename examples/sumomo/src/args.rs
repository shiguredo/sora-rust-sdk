use std::collections::HashSet;
use std::io;

use shiguredo_webrtc::rtc_log_info;
use sora_sdk::Role;

use crate::error::Result;

pub(crate) struct Args {
    pub(crate) signaling_urls: Vec<String>,
    pub(crate) channel_id: String,
    pub(crate) role: Role,
    pub(crate) audio: Option<bool>,
    pub(crate) video: Option<bool>,
    pub(crate) video_codec_type: Option<String>,
    pub(crate) video_codec_implementation: VideoCodecImplementationSelections,
    pub(crate) video_bit_rate: Option<u32>,
    pub(crate) input_mp4: Option<String>,
    pub(crate) openh264_path: Option<String>,
    pub(crate) video_codec_list: bool,
    pub(crate) data_channel_signaling: Option<bool>,
    pub(crate) ignore_disconnect_websocket: Option<bool>,
    pub(crate) simulcast: Option<bool>,
    pub(crate) insecure: bool,
    pub(crate) client_cert: Option<String>,
    pub(crate) client_key: Option<String>,
    pub(crate) ca_cert: Option<String>,
    pub(crate) duration: Option<u64>,
    pub(crate) turn_tls_insecure: bool,
    pub(crate) turn_tls_ca_cert: Option<String>,
    pub(crate) use_libcamera: bool,
    pub(crate) libcamera_controls: Vec<(String, String)>,
    #[cfg(feature = "raw-player")]
    pub(crate) use_raw_player: bool,
    #[cfg(feature = "media-device")]
    pub(crate) video_input_device: Option<String>,
    #[cfg(feature = "media-device")]
    pub(crate) audio_input_device: Option<String>,
    #[cfg(feature = "media-device")]
    pub(crate) list_devices: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum VideoCodecImplementationSelection {
    Internal,
    InternalApple,
    Amf,
    Nvcodec,
    Vpl,
    V4l2,
    Openh264,
}

impl VideoCodecImplementationSelection {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "internal" => Some(Self::Internal),
            "internal-apple" => Some(Self::InternalApple),
            "amf" => Some(Self::Amf),
            "nvcodec" => Some(Self::Nvcodec),
            "vpl" => Some(Self::Vpl),
            "v4l2" => Some(Self::V4l2),
            "openh264" => Some(Self::Openh264),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Internal => "internal",
            Self::InternalApple => "internal-apple",
            Self::Amf => "amf",
            Self::Nvcodec => "nvcodec",
            Self::Vpl => "vpl",
            Self::V4l2 => "v4l2",
            Self::Openh264 => "openh264",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum VideoCodecImplementationSelections {
    #[default]
    Auto,
    Manual(Vec<VideoCodecImplementationSelection>),
}

impl VideoCodecImplementationSelections {
    pub(crate) fn parse(value: &str) -> std::result::Result<Self, &'static str> {
        let values: Vec<&str> = value.split(',').map(|v| v.trim()).collect();
        if values.len() == 1 && values[0] == "auto" {
            return Ok(Self::Auto);
        }
        if values.iter().any(|v| v.is_empty()) {
            return Err("video-codec-implementation must not contain empty entries");
        }
        if values.contains(&"auto") {
            return Err(
                "video-codec-implementation auto cannot be combined with other implementations",
            );
        }

        let mut seen = HashSet::new();
        let mut selections = Vec::new();
        for value in values {
            let selection = VideoCodecImplementationSelection::parse(value).ok_or(
                "video-codec-implementation must be auto/internal/internal-apple/amf/nvcodec/vpl/v4l2/openh264",
            )?;
            if !seen.insert(selection) {
                return Err(
                    "video-codec-implementation must not contain duplicate implementations",
                );
            }
            selections.push(selection);
        }
        Ok(Self::Manual(selections))
    }

    pub(crate) fn contains(&self, selection: VideoCodecImplementationSelection) -> bool {
        match self {
            Self::Auto => false,
            Self::Manual(selections) => selections.contains(&selection),
        }
    }
}

pub(crate) fn parse_args(mut args: noargs::RawArgs) -> Result<Args> {
    args.metadata_mut().app_name = env!("CARGO_PKG_NAME");
    args.metadata_mut().app_description = "Sora WebSocket シグナリングの最小サンプル";

    if noargs::VERSION_FLAG.take(&mut args).is_present() {
        rtc_log_info!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }

    noargs::HELP_FLAG.take_help(&mut args);

    // codec list モードは接続処理を行わず、表示用オプションだけを解釈して即終了する。
    let video_codec_list = noargs::flag("video-codec-list")
        .doc("利用可能な映像コーデック実装と選択優先順位を表示して終了する")
        .take(&mut args)
        .is_present();

    if video_codec_list {
        // preference 計算に必要な実装優先順だけ先に解釈する。
        let video_codec_implementation: Option<VideoCodecImplementationSelections> =
            noargs::opt("video-codec-implementation")
                .doc("映像コーデック実装 (auto または internal/internal-apple/amf/nvcodec/vpl/v4l2/openh264 のカンマ区切り)")
                .take(&mut args)
                .present_and_then(|o| VideoCodecImplementationSelections::parse(o.value()))?;

        // OpenH264 の可用性判定に必要なパスを受け取る。
        let openh264_path: Option<String> = noargs::opt("openh264-path")
            .doc("OpenH264 の動的ライブラリパス")
            .take(&mut args)
            .present_and_then(|o| Ok::<_, &str>(o.value().to_string()))?;

        let _ = args.finish();
        return Ok(Args {
            signaling_urls: Vec::new(),
            channel_id: String::new(),
            role: Role::RecvOnly,
            audio: None,
            video: None,
            video_codec_type: None,
            video_codec_implementation: video_codec_implementation.unwrap_or_default(),
            video_bit_rate: None,
            input_mp4: None,
            openh264_path,
            video_codec_list: true,
            data_channel_signaling: None,
            ignore_disconnect_websocket: None,
            simulcast: None,
            insecure: false,
            client_cert: None,
            client_key: None,
            ca_cert: None,
            duration: None,
            turn_tls_insecure: false,
            turn_tls_ca_cert: None,
            use_libcamera: false,
            libcamera_controls: Vec::new(),
            #[cfg(feature = "raw-player")]
            use_raw_player: false,
            #[cfg(feature = "media-device")]
            video_input_device: None,
            #[cfg(feature = "media-device")]
            audio_input_device: None,
            #[cfg(feature = "media-device")]
            list_devices: false,
        });
    }

    // --list-devices は他のオプションなしで使用できる
    #[cfg(feature = "media-device")]
    let list_devices = noargs::flag("list-devices")
        .doc("利用可能なデバイス一覧を表示して終了する")
        .take(&mut args)
        .is_present();

    #[cfg(feature = "media-device")]
    if list_devices {
        // list-devices モードでは他のオプションは不要
        let _ = args.finish();
        return Ok(Args {
            signaling_urls: Vec::new(),
            channel_id: String::new(),
            role: Role::RecvOnly,
            audio: None,
            video: None,
            video_codec_type: None,
            video_codec_implementation: VideoCodecImplementationSelections::Auto,
            video_bit_rate: None,
            data_channel_signaling: None,
            ignore_disconnect_websocket: None,
            simulcast: None,
            insecure: false,
            client_cert: None,
            client_key: None,
            ca_cert: None,
            duration: None,
            turn_tls_insecure: false,
            turn_tls_ca_cert: None,
            input_mp4: None,
            openh264_path: None,
            video_codec_list: false,
            use_libcamera: false,
            libcamera_controls: Vec::new(),
            #[cfg(feature = "raw-player")]
            use_raw_player: false,
            video_input_device: None,
            audio_input_device: None,
            list_devices: true,
        });
    }

    let signaling_urls: Vec<String> = noargs::opt("signaling-url")
        .doc("Sora の WebSocket シグナリング URL (カンマ区切りで複数指定可)")
        .example("wss://sora.example.com/signaling")
        .take(&mut args)
        .then(|o| Ok::<_, &str>(o.value().split(',').map(|s| s.trim().to_string()).collect()))?;

    let channel_id: String = noargs::opt("channel-id")
        .doc("Sora のチャネル ID")
        .example("sora")
        .take(&mut args)
        .then(|o| Ok::<_, &str>(o.value().to_string()))?;

    let role: String = noargs::opt("role")
        .doc("Sora のロール (sendonly, recvonly, sendrecv)")
        .example("recvonly")
        .take(&mut args)
        .then(|o| Ok::<_, &str>(o.value().to_string()))?;

    let audio: Option<bool> = noargs::opt("audio")
        .doc("音声の有効/無効 (true/false)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("audio は true または false で指定してください"),
        })?;

    let video: Option<bool> = noargs::opt("video")
        .doc("映像の有効/無効 (true/false)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("video は true または false で指定してください"),
        })?;

    let video_codec_type: Option<String> = noargs::opt("video-codec-type")
        .doc("映像コーデック (vp8/vp9/av1/h264/h265)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "vp8" | "vp9" | "av1" | "h264" | "h265" => Ok(o.value().to_string()),
            _ => Err("video-codec-type は vp8/vp9/av1/h264/h265 で指定してください"),
        })?;

    let video_codec_implementation: Option<VideoCodecImplementationSelections> =
        noargs::opt("video-codec-implementation")
            .doc("映像コーデック実装 (auto または internal/internal-apple/amf/nvcodec/vpl/v4l2/openh264 のカンマ区切り)")
            .take(&mut args)
            .present_and_then(|o| VideoCodecImplementationSelections::parse(o.value()))?;

    let video_bit_rate: Option<u32> = noargs::opt("video-bit-rate")
        .doc("映像ビットレート (kbps)")
        .take(&mut args)
        .present_and_then(|o| {
            o.value()
                .parse::<u32>()
                .map_err(|_| "video-bit-rate は数値で指定してください")
        })?;

    let input_mp4: Option<String> = noargs::opt("input-mp4")
        .doc("MP4 ファイルからエンコード済み映像をそのまま送信する")
        .take(&mut args)
        .present_and_then(|o| Ok::<_, &str>(o.value().to_string()))?;

    let openh264_path: Option<String> = noargs::opt("openh264-path")
        .doc("OpenH264 の動的ライブラリパス")
        .take(&mut args)
        .present_and_then(|o| Ok::<_, &str>(o.value().to_string()))?;

    let data_channel_signaling: Option<bool> = noargs::opt("data-channel-signaling")
        .doc("DataChannel 経由でシグナリングを行う (true/false)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("data-channel-signaling は true または false で指定してください"),
        })?;

    let ignore_disconnect_websocket: Option<bool> = noargs::opt("ignore-disconnect-websocket")
        .doc("DataChannel 使用時に WebSocket 切断を無視する (true/false)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("ignore-disconnect-websocket は true または false で指定してください"),
        })?;

    let simulcast: Option<bool> = noargs::opt("simulcast")
        .doc("サイマルキャストを有効にする (true/false)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("simulcast は true または false で指定してください"),
        })?;

    let insecure = noargs::flag("insecure")
        .doc("サーバー証明書の検証をスキップする")
        .take(&mut args)
        .is_present();

    let client_cert: Option<String> = noargs::opt("client-cert")
        .doc("クライアント証明書の PEM ファイルパス")
        .take(&mut args)
        .present_and_then(|o| {
            std::fs::read_to_string(o.value())
                .map_err(|e| format!("クライアント証明書の読み込みに失敗しました: {e}"))
        })?;

    let client_key: Option<String> = noargs::opt("client-key")
        .doc("クライアント秘密鍵の PEM ファイルパス")
        .take(&mut args)
        .present_and_then(|o| {
            std::fs::read_to_string(o.value())
                .map_err(|e| format!("クライアント秘密鍵の読み込みに失敗しました: {e}"))
        })?;

    let ca_cert: Option<String> = noargs::opt("ca-cert")
        .doc("CA 証明書の PEM ファイルパス")
        .take(&mut args)
        .present_and_then(|o| {
            std::fs::read_to_string(o.value())
                .map_err(|e| format!("CA 証明書の読み込みに失敗しました: {e}"))
        })?;

    let duration: Option<u64> = noargs::opt("duration")
        .doc("接続を維持する秒数 (省略時は無制限)")
        .take(&mut args)
        .present_and_then(|o| o.value().parse::<u64>())?;

    let turn_tls_insecure = noargs::flag("turn-tls-insecure")
        .doc("TURN-TLS の証明書検証をスキップする")
        .take(&mut args)
        .is_present();

    let turn_tls_ca_cert: Option<String> = noargs::opt("turn-tls-ca-cert")
        .doc("TURN-TLS の CA 証明書ファイル (PEM 形式)")
        .take(&mut args)
        .present_and_then(|o| Ok::<_, &str>(o.value().to_string()))?;

    let use_libcamera = noargs::flag("libcamera")
        .doc("Use libcamera video capturer")
        .take(&mut args)
        .is_present();

    // --libcamera-control KEY=VALUE (複数回指定可能)
    let mut libcamera_controls = Vec::new();
    loop {
        let opt = noargs::opt("libcamera-control")
            .ty("KEY=VALUE")
            .doc("Set libcamera control (repeatable)")
            .take(&mut args);
        if !opt.is_present() {
            break;
        }
        let raw = opt.value().to_string();
        match raw.split_once('=') {
            Some((key, value)) => {
                libcamera_controls.push((key.to_string(), value.to_string()));
            }
            None => {
                return Err(noargs::Error::other(
                    &args,
                    format!("--libcamera-control must be KEY=VALUE: {raw}"),
                )
                .into());
            }
        }
    }

    #[cfg(feature = "raw-player")]
    let use_raw_player = noargs::flag("raw-player")
        .doc("raw-player でビデオを表示する")
        .take(&mut args)
        .is_present();

    #[cfg(feature = "media-device")]
    let video_input_device: Option<String> = noargs::opt("video-input-device")
        .doc("使用するビデオ入力デバイスの ID（省略時は FakeVideoCapturer を使用）")
        .take(&mut args)
        .present_and_then(|o| Ok::<_, &str>(o.value().to_string()))?;

    #[cfg(feature = "media-device")]
    let audio_input_device: Option<String> = noargs::opt("audio-input-device")
        .doc("使用するオーディオ入力デバイスの名前または ID")
        .take(&mut args)
        .present_and_then(|o| Ok::<_, &str>(o.value().to_string()))?;

    if let Some(help) = args.finish()? {
        print!("{}", help);
        std::process::exit(0);
    }

    let role = Role::parse(&role)?;

    Ok(Args {
        signaling_urls,
        channel_id,
        role,
        audio,
        video,
        video_codec_type,
        video_codec_implementation: video_codec_implementation.unwrap_or_default(),
        video_bit_rate,
        input_mp4,
        openh264_path,
        video_codec_list: false,
        data_channel_signaling,
        ignore_disconnect_websocket,
        simulcast,
        insecure,
        client_cert,
        client_key,
        ca_cert,
        duration,
        turn_tls_insecure,
        turn_tls_ca_cert,
        use_libcamera,
        libcamera_controls,
        #[cfg(feature = "raw-player")]
        use_raw_player,
        #[cfg(feature = "media-device")]
        video_input_device,
        #[cfg(feature = "media-device")]
        audio_input_device,
        #[cfg(feature = "media-device")]
        list_devices: false,
    })
}

pub(crate) fn validate_args(args: &Args) -> Result<()> {
    // mp4 passthrough と OpenH264 ライブラリ指定は排他的。
    if args.input_mp4.is_some() && args.openh264_path.is_some() {
        return Err(
            io::Error::other("--input-mp4 and --openh264-path cannot be used together").into(),
        );
    }

    // OpenH264 は実装選択とライブラリパスがセットで必要。
    let openh264_selected = args
        .video_codec_implementation
        .contains(VideoCodecImplementationSelection::Openh264);
    if openh264_selected && args.openh264_path.is_none() {
        return Err(io::Error::other(
            "--video-codec-implementation openh264 requires --openh264-path",
        )
        .into());
    }
    if !openh264_selected && args.openh264_path.is_some() {
        return Err(io::Error::other(
            "--openh264-path requires --video-codec-implementation to include openh264",
        )
        .into());
    }

    if !args.use_libcamera && !args.libcamera_controls.is_empty() {
        return Err(io::Error::other("--libcamera-control requires --libcamera").into());
    }

    #[cfg(not(feature = "libcamera"))]
    if args.use_libcamera {
        return Err(io::Error::other(
            "libcamera is not enabled in this build. Rebuild sumomo with --features libcamera",
        )
        .into());
    }

    #[cfg(feature = "media-device")]
    if args.use_libcamera && args.video_input_device.is_some() {
        return Err(io::Error::other(
            "--libcamera and --video-input-device cannot be used together",
        )
        .into());
    }

    #[cfg(not(feature = "amf"))]
    if args
        .video_codec_implementation
        .contains(VideoCodecImplementationSelection::Amf)
    {
        return Err(io::Error::other(
            "AMF is not enabled in this build. Rebuild sumomo with --features amf",
        )
        .into());
    }

    #[cfg(not(feature = "nvcodec"))]
    if args
        .video_codec_implementation
        .contains(VideoCodecImplementationSelection::Nvcodec)
    {
        return Err(io::Error::other(
            "NVCodec is not enabled in this build. Rebuild sumomo with --features nvcodec",
        )
        .into());
    }

    #[cfg(not(feature = "vpl"))]
    if args
        .video_codec_implementation
        .contains(VideoCodecImplementationSelection::Vpl)
    {
        return Err(io::Error::other(
            "VPL is not enabled in this build. Rebuild sumomo with --features vpl",
        )
        .into());
    }

    #[cfg(not(feature = "v4l2"))]
    if args
        .video_codec_implementation
        .contains(VideoCodecImplementationSelection::V4l2)
    {
        return Err(io::Error::other(
            "V4L2 is not enabled in this build. Rebuild sumomo with --features v4l2",
        )
        .into());
    }

    Ok(())
}
