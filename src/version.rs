//! クライアント情報・環境情報の生成。

/// デフォルトの sora_client 名を返す。
pub(crate) fn get_sora_client_name() -> String {
    format!("Sora Rust SDK {}", env!("CARGO_PKG_VERSION"))
}

/// shiguredo_webrtc のバージョンから libwebrtc 名を生成する。
pub(crate) fn get_libwebrtc_name() -> String {
    let version = shiguredo_webrtc::version();
    // バージョン形式: "0.146.0-canary.0" からマイナーバージョン (146) を取り出す
    let minor = version.split('.').nth(1).unwrap_or("0");
    format!("Shiguredo-Build M{minor}")
}

/// 実行環境の情報を返す。
pub(crate) fn get_environment_name() -> String {
    let arch = std::env::consts::ARCH;
    let os_info = get_os_info();
    format!("[{arch}] {os_info}")
}

#[cfg(target_os = "macos")]
fn get_os_info() -> String {
    let version = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .unwrap_or_default();
    let version = version.trim();
    if version.is_empty() {
        "macOS".to_string()
    } else {
        format!("macOS {version}")
    }
}

#[cfg(target_os = "linux")]
fn get_os_info() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                let line = line.trim();
                line.strip_prefix("PRETTY_NAME=")
                    .map(|value| value.trim_matches('"').to_string())
            })
        })
        .unwrap_or_else(|| "Linux".to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn get_os_info() -> String {
    std::env::consts::OS.to_string()
}
