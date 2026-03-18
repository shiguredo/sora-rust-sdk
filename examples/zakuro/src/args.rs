use shiguredo_webrtc::rtc_log_info;
use sora_sdk::Role;

use crate::error::{ErrorMessage, Result};

pub(crate) struct Args {
    pub(crate) signaling_urls: Vec<String>,
    pub(crate) channel_id: String,
    pub(crate) role: Role,
    pub(crate) vcs: u32,
    pub(crate) vcs_hatch_rate: f64,
    pub(crate) duration: Option<f64>,
    pub(crate) repeat_interval: Option<f64>,
    pub(crate) max_retry: u32,
    pub(crate) retry_interval: f64,
    pub(crate) no_video_device: bool,
    pub(crate) no_audio_device: bool,
    pub(crate) resolution: (i32, i32),
    pub(crate) framerate: u32,
    pub(crate) sandstorm: bool,
    pub(crate) video_codec_type: Option<String>,
    pub(crate) video_bit_rate: Option<u32>,
    pub(crate) audio: bool,
    pub(crate) audio_codec_type: Option<String>,
    pub(crate) audio_bit_rate: Option<u32>,
    pub(crate) data_channel_signaling: Option<bool>,
    pub(crate) ignore_disconnect_websocket: Option<bool>,
}

fn parse_resolution(s: &str) -> Result<(i32, i32)> {
    match s {
        "QVGA" => Ok((320, 240)),
        "VGA" => Ok((640, 480)),
        "HD" => Ok((1280, 720)),
        "FHD" => Ok((1920, 1080)),
        "4K" => Ok((3840, 2160)),
        _ => {
            let parts: Vec<&str> = s.split('x').collect();
            if parts.len() != 2 {
                return Err(ErrorMessage::new(format!(
                    "resolution は QVGA/VGA/HD/FHD/4K または WxH で指定してください: {s}"
                ))
                .into());
            }
            let width: i32 = parts[0].parse().map_err(|_| {
                ErrorMessage::new(format!("resolution の幅が不正です: {}", parts[0]))
            })?;
            let height: i32 = parts[1].parse().map_err(|_| {
                ErrorMessage::new(format!("resolution の高さが不正です: {}", parts[1]))
            })?;
            if width <= 0 || height <= 0 {
                return Err(
                    ErrorMessage::new("resolution の幅と高さは正の整数で指定してください").into(),
                );
            }
            Ok((width, height))
        }
    }
}

pub(crate) fn parse_args() -> Result<Args> {
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = env!("CARGO_PKG_NAME");
    args.metadata_mut().app_description = "Sora WebRTC SFU 負荷試験ツール";

    if noargs::VERSION_FLAG.take(&mut args).is_present() {
        rtc_log_info!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }

    noargs::HELP_FLAG.take_help(&mut args);

    let signaling_urls: Vec<String> = noargs::opt("sora-signaling-url")
        .doc("Sora の WebSocket シグナリング URL (カンマ区切りで複数指定可)")
        .example("wss://sora.example.com/signaling")
        .take(&mut args)
        .then(|o| Ok::<_, &str>(o.value().split(',').map(|s| s.trim().to_string()).collect()))?;

    let channel_id: String = noargs::opt("sora-channel-id")
        .doc("Sora のチャネル ID")
        .example("zakuro-test")
        .take(&mut args)
        .then(|o| Ok::<_, &str>(o.value().to_string()))?;

    let role: String = noargs::opt("sora-role")
        .doc("Sora のロール (sendonly, recvonly, sendrecv)")
        .example("sendonly")
        .take(&mut args)
        .then(|o| Ok::<_, &str>(o.value().to_string()))?;

    let vcs: u32 = noargs::opt("vcs")
        .doc("仮想クライアント数 (1-1000, デフォルト: 1)")
        .take(&mut args)
        .present_and_then(|o| o.value().parse::<u32>())?
        .unwrap_or(1);

    let vcs_hatch_rate: f64 = noargs::opt("vcs-hatch-rate")
        .doc("仮想クライアントの起動レート (秒あたりの起動数, デフォルト: 1.0)")
        .take(&mut args)
        .present_and_then(|o| o.value().parse::<f64>())?
        .unwrap_or(1.0);

    let duration: Option<f64> = noargs::opt("duration")
        .doc("仮想クライアントの接続維持秒数 (省略時は無制限)")
        .take(&mut args)
        .present_and_then(|o| o.value().parse::<f64>())?;

    let repeat_interval: Option<f64> = noargs::opt("repeat-interval")
        .doc("duration 経過後の再接続間隔 (秒)")
        .take(&mut args)
        .present_and_then(|o| o.value().parse::<f64>())?;

    let max_retry: u32 = noargs::opt("max-retry")
        .doc("接続失敗時の最大リトライ回数 (デフォルト: 0)")
        .take(&mut args)
        .present_and_then(|o| o.value().parse::<u32>())?
        .unwrap_or(0);

    let retry_interval: f64 = noargs::opt("retry-interval")
        .doc("リトライ間隔 (秒, デフォルト: 60.0)")
        .take(&mut args)
        .present_and_then(|o| o.value().parse::<f64>())?
        .unwrap_or(60.0);

    let no_video_device = noargs::flag("no-video-device")
        .doc("映像デバイスを使用しない")
        .take(&mut args)
        .is_present();

    let no_audio_device = noargs::flag("no-audio-device")
        .doc("音声デバイスを使用しない")
        .take(&mut args)
        .is_present();

    let resolution: (i32, i32) = noargs::opt("resolution")
        .doc("映像解像度 (QVGA/VGA/HD/FHD/4K または WxH, デフォルト: VGA)")
        .take(&mut args)
        .present_and_then(|o| parse_resolution(o.value()))?
        .unwrap_or((640, 480));

    let framerate: u32 = noargs::opt("framerate")
        .doc("映像フレームレート (1-60, デフォルト: 30)")
        .take(&mut args)
        .present_and_then(|o| o.value().parse::<u32>())?
        .unwrap_or(30);

    let sandstorm = noargs::flag("sandstorm")
        .doc("砂嵐映像を生成する")
        .take(&mut args)
        .is_present();

    let video_codec_type: Option<String> = noargs::opt("sora-video-codec-type")
        .doc("映像コーデック (vp8/vp9/av1/h264/h265)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "vp8" | "vp9" | "av1" | "h264" | "h265" => Ok(o.value().to_string()),
            _ => Err("sora-video-codec-type は vp8/vp9/av1/h264/h265 で指定してください"),
        })?;

    let video_bit_rate: Option<u32> = noargs::opt("sora-video-bit-rate")
        .doc("映像ビットレート (kbps)")
        .take(&mut args)
        .present_and_then(|o| o.value().parse::<u32>())?;

    let audio: bool = noargs::opt("sora-audio")
        .doc("音声の有効/無効 (true/false, デフォルト: true)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("sora-audio は true または false で指定してください"),
        })?
        .unwrap_or(true);

    let audio_codec_type: Option<String> = noargs::opt("sora-audio-codec-type")
        .doc("音声コーデック (opus)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "opus" => Ok(o.value().to_string()),
            _ => Err("sora-audio-codec-type は opus で指定してください"),
        })?;

    let audio_bit_rate: Option<u32> = noargs::opt("sora-audio-bit-rate")
        .doc("音声ビットレート (kbps)")
        .take(&mut args)
        .present_and_then(|o| o.value().parse::<u32>())?;

    let data_channel_signaling: Option<bool> = noargs::opt("sora-data-channel-signaling")
        .doc("DataChannel 経由でシグナリングを行う (true/false)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("sora-data-channel-signaling は true または false で指定してください"),
        })?;

    let ignore_disconnect_websocket: Option<bool> = noargs::opt("sora-ignore-disconnect-websocket")
        .doc("DataChannel 使用時に WebSocket 切断を無視する (true/false)")
        .take(&mut args)
        .present_and_then(|o| match o.value() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("sora-ignore-disconnect-websocket は true または false で指定してください"),
        })?;

    if let Some(help) = args.finish()? {
        print!("{}", help);
        std::process::exit(0);
    }

    let role = Role::parse(&role)?;

    // バリデーション
    if vcs == 0 || vcs > 1000 {
        return Err(ErrorMessage::new("vcs は 1 から 1000 の範囲で指定してください").into());
    }
    if vcs_hatch_rate <= 0.0 {
        return Err(ErrorMessage::new("vcs-hatch-rate は正の数で指定してください").into());
    }
    if framerate == 0 || framerate > 60 {
        return Err(ErrorMessage::new("framerate は 1 から 60 の範囲で指定してください").into());
    }

    Ok(Args {
        signaling_urls,
        channel_id,
        role,
        vcs,
        vcs_hatch_rate,
        duration,
        repeat_interval,
        max_retry,
        retry_interval,
        no_video_device,
        no_audio_device,
        resolution,
        framerate,
        sandstorm,
        video_codec_type,
        video_bit_rate,
        audio,
        audio_codec_type,
        audio_bit_rate,
        data_channel_signaling,
        ignore_disconnect_websocket,
    })
}
