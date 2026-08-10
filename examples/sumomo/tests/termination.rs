// sumomo の終了要求と失敗結果を child process 経由で検証する integration test。
//
// Cargo が提供する実 `CARGO_BIN_EXE_sumomo` を child process として起動する。
// 認証付き Sora へ接続するため、`TEST_SIGNALING_URLS` と `TEST_SECRET_KEY`、
// channel ID の識別用に `TEST_CHANNEL_ID_PREFIX` / `TEST_CHANNEL_ID_SUFFIX` を使う。
// これらは e2e-tests と共有する CI 設定から読む。
//
// `TEST_SIGNALING_URLS` が未設定の場合は skip せず設定不足で失敗させる。
// credential、secret query、certificate を test output に表示しない。
// child output を assertion failure に添付する場合は signaling URL を除去する。

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use aws_lc_rs::hmac;
use base64ct::{Base64UrlUnpadded, Encoding};

/// テスト用の環境変数を e2e-tests/.env から読み込む。
///
/// CI では repository variable / secret が環境変数として設定されるため、
/// 既に設定済みの環境変数は上書きしない。ローカルでは e2e-tests/.env から
/// `TEST_SIGNALING_URLS` 等を読み込めるようにする。
fn load_env() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let env_path = std::path::Path::new(manifest_dir).join("../../e2e-tests/.env");
    let Ok(content) = std::fs::read_to_string(&env_path) else {
        return;
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        if std::env::var(key).is_err() {
            // Safety: テストプロセス内でのみ環境変数を設定する。
            unsafe { std::env::set_var(key, value) };
        }
    }
}

/// シグナリング URL を取得する。
///
/// 未設定なら設定不足として test を失敗させる。
fn signaling_urls() -> Vec<String> {
    let value =
        std::env::var("TEST_SIGNALING_URLS").expect("TEST_SIGNALING_URLS が設定されていません");
    let urls: Vec<String> = value.split(',').map(|s| s.trim().to_string()).collect();
    assert!(!urls.is_empty(), "TEST_SIGNALING_URLS が空です");
    urls
}

/// シークレットキーを取得する (設定されている場合)。
fn secret_key() -> Option<String> {
    std::env::var("TEST_SECRET_KEY").ok()
}

/// channel ID を生成する。
///
/// e2e-tests と同じく、ログや Sora 側でどのテストの接続かを識別できるよう
/// prefix と suffix を channel ID に含める。
fn generate_channel_id() -> String {
    let prefix =
        std::env::var("TEST_CHANNEL_ID_PREFIX").unwrap_or_else(|_| "sumomo-test".to_string());
    let suffix = std::env::var("TEST_CHANNEL_ID_SUFFIX").unwrap_or_default();
    let random = shiguredo_webrtc::random_bytes(8);
    let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
    format!("{prefix}{hex}{suffix}")
}

/// access_token (JWT) を生成する。
///
/// channel_id と exp を含む payload に HMAC-SHA256 で署名する。
/// e2e-tests の `generate_access_token` と同じ形式。
fn generate_access_token(channel_id: &str, secret: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("システム時刻の取得に失敗しました")
        .as_secs();
    let exp = now + 300;

    let payload = nojson::object(|f| {
        f.member("channel_id", channel_id)?;
        f.member("exp", exp)
    })
    .to_string();

    let header = r#"{"alg":"HS256","typ":"JWT"}"#;

    let signing_input = format!(
        "{}.{}",
        Base64UrlUnpadded::encode_string(header.as_bytes()),
        Base64UrlUnpadded::encode_string(payload.as_bytes()),
    );

    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let signature = hmac::sign(&key, signing_input.as_bytes());

    format!(
        "{}.{}",
        signing_input,
        Base64UrlUnpadded::encode_string(signature.as_ref())
    )
}

/// access_token を含む metadata JSON 文字列を生成する。
fn build_metadata_with_access_token(access_token: &str) -> String {
    nojson::object(|f| f.member("access_token", access_token)).to_string()
}

/// sumomo への child process 実行結果。
struct SumomoOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

impl SumomoOutput {
    /// child output から signaling URL を除去した assertion message を作る。
    fn message(&self, urls: &[String]) -> String {
        let stdout = redact_signaling_urls(&self.stdout, urls);
        let stderr = redact_signaling_urls(&self.stderr, urls);
        // access_token を含む行は credential 漏洩の恐れがあるため行ごと除去する。
        let stdout = redact_access_token_lines(&stdout);
        let stderr = redact_access_token_lines(&stderr);
        format!(
            "exit: {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            self.status, stdout, stderr,
        )
    }
}

/// 出力から signaling URL を除去する。
fn redact_signaling_urls(output: &str, urls: &[String]) -> String {
    let mut result = output.to_string();
    for url in urls {
        result = result.replace(url, "<signaling-url>");
    }
    result
}

/// access_token を含む行を除去する。
fn redact_access_token_lines(output: &str) -> String {
    output
        .lines()
        .filter(|line| !line.contains("access_token"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// sumomo を child process として起動し、30 秒以内の終了を待つ。
///
/// timeout 時は child を kill / wait して test を失敗させる。
/// child が大量の出力で pipe buffer を詰まらせても child がブロックしないよう、
/// stdout / stderr は別 thread で読み取る。
fn run_sumomo(args: &[String], envs: &[(&str, &str)]) -> SumomoOutput {
    let bin = env!("CARGO_BIN_EXE_sumomo");
    let mut command = Command::new(bin);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("sumomo の起動に失敗しました");

    let stdout_handle = std::thread::spawn({
        let stdout = child.stdout.take().expect("stdout の取得に失敗しました");
        move || read_to_string(stdout)
    });
    let stderr_handle = std::thread::spawn({
        let stderr = child.stderr.take().expect("stderr の取得に失敗しました");
        move || read_to_string(stderr)
    });

    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("sumomo の終了確認に失敗しました") {
            Some(status) => break status,
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("sumomo が 30 秒以内に終了しませんでした");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };

    // 読み取り thread の join。child は終了済みなので pipe は閉じられ、
    // read_to_string は必ず完了する。
    let stdout = stdout_handle
        .join()
        .expect("stdout 読み取り thread の join に失敗しました");
    let stderr = stderr_handle
        .join()
        .expect("stderr 読み取り thread の join に失敗しました");

    SumomoOutput {
        status,
        stdout,
        stderr,
    }
}

fn read_to_string<R: Read>(mut reader: R) -> String {
    let mut buf = String::new();
    reader
        .read_to_string(&mut buf)
        .expect("child output の読み取りに失敗しました");
    buf
}

/// 通常の CLI 引数を組み立てる。
///
/// `--metadata` は `TEST_SECRET_KEY` が設定されている場合だけ付与する。
fn build_cli_args(urls: &[String], channel_id: &str, extra: &[&str]) -> Vec<String> {
    let mut args = vec![
        "--signaling-url".to_string(),
        urls.join(","),
        "--channel-id".to_string(),
        channel_id.to_string(),
        "--role".to_string(),
        "recvonly".to_string(),
    ];
    if let Some(secret) = secret_key() {
        let access_token = generate_access_token(channel_id, &secret);
        let metadata = build_metadata_with_access_token(&access_token);
        args.push("--metadata".to_string());
        args.push(metadata);
    }
    args.extend(extra.iter().map(|s| s.to_string()));
    args
}

/// 通常表示で `--duration 1` を指定すると、disconnect して 30 秒以内に exit 0 になる。
#[test]
fn normal_display_duration_exits_zero() {
    load_env();
    let urls = signaling_urls();
    let channel_id = generate_channel_id();
    let args = build_cli_args(&urls, &channel_id, &["--duration", "1"]);
    let output = run_sumomo(&args, &[]);
    assert!(
        output.status.success(),
        "通常表示で duration 経過時に exit 0 になる必要があります:\n{}",
        output.message(&urls)
    );
}

/// 構文不正な signaling URL による即時 connection error は exit non-zero になり、hang しない。
#[test]
fn malformed_signaling_url_exits_nonzero() {
    load_env();
    let channel_id = generate_channel_id();
    let malformed = "not-a-url".to_string();
    let urls = vec![malformed.clone()];
    let args = build_cli_args(&urls, &channel_id, &[]);
    let output = run_sumomo(&args, &[]);
    assert!(
        !output.status.success(),
        "構文不正な signaling URL は exit non-zero になる必要があります:\n{}",
        output.message(&urls)
    );
}

/// raw-player で `SDL_VIDEODRIVER=dummy` と `--duration 1` を指定すると、
/// 30 秒以内に exit 0 になる。
#[cfg(feature = "raw-player")]
#[test]
fn raw_player_duration_exits_zero() {
    load_env();
    let urls = signaling_urls();
    let channel_id = generate_channel_id();
    let args = build_cli_args(&urls, &channel_id, &["--duration", "1", "--raw-player"]);
    let output = run_sumomo(&args, &[("SDL_VIDEODRIVER", "dummy")]);
    assert!(
        output.status.success(),
        "raw-player で duration 経過時に exit 0 になる必要があります:\n{}",
        output.message(&urls)
    );
}

/// repository にある実 MP4 fixture と `--input-mp4 --duration 1` を指定すると、
/// connection shutdown、capturer drop、process exit が 30 秒以内に完了する。
#[test]
fn input_mp4_duration_exits_zero() {
    load_env();
    let urls = signaling_urls();
    let channel_id = generate_channel_id();

    // repository の MP4 fixture のパスを解決する。
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let mp4_path = std::path::Path::new(manifest_dir).join("../../testdata/red-320x320-h264.mp4");
    let mp4_path = mp4_path
        .canonicalize()
        .expect("MP4 fixture の解決に失敗しました");
    let mp4_path = mp4_path
        .to_str()
        .expect("MP4 fixture のパスが UTF-8 ではありません");

    let mut args = vec![
        "--signaling-url".to_string(),
        urls.join(","),
        "--channel-id".to_string(),
        channel_id.clone(),
        "--role".to_string(),
        "sendonly".to_string(),
        "--input-mp4".to_string(),
        mp4_path.to_string(),
        "--duration".to_string(),
        "1".to_string(),
    ];
    if let Some(secret) = secret_key() {
        let access_token = generate_access_token(&channel_id, &secret);
        let metadata = build_metadata_with_access_token(&access_token);
        args.push("--metadata".to_string());
        args.push(metadata);
    }

    let output = run_sumomo(&args, &[]);
    assert!(
        output.status.success(),
        "実 MP4 fixture を使った場合に exit 0 になる必要があります:\n{}",
        output.message(&urls)
    );
}

/// 存在しない MP4 path による raw-player worker async setup error は exit non-zero になる。
#[cfg(feature = "raw-player")]
#[test]
fn raw_player_missing_mp4_exits_nonzero() {
    load_env();
    let channel_id = generate_channel_id();
    let urls = vec!["wss://example.invalid/signaling".to_string()];
    let args = build_cli_args(
        &urls,
        &channel_id,
        &[
            "--input-mp4",
            "/nonexistent/path/sumomo-test.mp4",
            "--raw-player",
        ],
    );
    let output = run_sumomo(&args, &[("SDL_VIDEODRIVER", "dummy")]);
    assert!(
        !output.status.success(),
        "存在しない MP4 path は exit non-zero になる必要があります:\n{}",
        output.message(&urls)
    );
}

/// 無効な SDL video driver による renderer setup error は exit non-zero になる。
#[cfg(feature = "raw-player")]
#[test]
fn raw_player_invalid_sdl_driver_exits_nonzero() {
    load_env();
    let channel_id = generate_channel_id();
    let urls = vec!["wss://example.invalid/signaling".to_string()];
    let args = build_cli_args(&urls, &channel_id, &["--raw-player"]);
    let output = run_sumomo(&args, &[("SDL_VIDEODRIVER", "invalid-driver")]);
    assert!(
        !output.status.success(),
        "無効な SDL video driver は exit non-zero になる必要があります:\n{}",
        output.message(&urls)
    );
}
