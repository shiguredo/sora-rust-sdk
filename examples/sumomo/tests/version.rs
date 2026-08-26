// sumomo の --version 出力を child process 経由で検証する integration test。
//
// Cargo が提供する実 `CARGO_BIN_EXE_sumomo` を child process として起動し、
// --version が stdout に `sumomo 0.0.0` を末尾改行付きで出力して exit 0 することを
// 検証する。--help と同じくログプレフィックス・タイムスタンプ・スレッド名を
// stdout に含めず、stderr にも出力しないことを確認する。
//
// この test は Sora への接続を伴わないため、シグナリング URL 等の環境変数は不要。

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// sumomo を child process として起動し、10 秒以内の終了を待つ。
///
/// timeout 時は child を kill / wait して test を失敗させる。
/// child が大量の出力で pipe buffer を詰まらせても child がブロックしないよう、
/// stdout / stderr は別 thread で読み取る。
fn run_sumomo(args: &[&str]) -> (std::process::ExitStatus, String, String) {
    let bin = env!("CARGO_BIN_EXE_sumomo");
    let mut child = Command::new(bin)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sumomo の起動に失敗しました");

    let stdout_handle = std::thread::spawn({
        let stdout = child.stdout.take().expect("stdout の取得に失敗しました");
        move || read_to_string(stdout)
    });
    let stderr_handle = std::thread::spawn({
        let stderr = child.stderr.take().expect("stderr の取得に失敗しました");
        move || read_to_string(stderr)
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait().expect("sumomo の終了確認に失敗しました") {
            Some(status) => break status,
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("sumomo が 10 秒以内に終了しませんでした");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };

    let stdout = stdout_handle
        .join()
        .expect("stdout 読み取り thread の join に失敗しました");
    let stderr = stderr_handle
        .join()
        .expect("stderr 読み取り thread の join に失敗しました");

    (status, stdout, stderr)
}

fn read_to_string<R: Read>(mut reader: R) -> String {
    let mut buf = String::new();
    reader
        .read_to_string(&mut buf)
        .expect("child output の読み取りに失敗しました");
    buf
}

/// --version は stdout にバージョン文字列を末尾改行付きで出力し、exit 0 する。
#[test]
fn version_outputs_to_stdout() {
    let (status, stdout, stderr) = run_sumomo(&["--version"]);

    assert!(
        status.success(),
        "--version は exit 0 になる必要があります:\n--- stdout ---\n{}\n--- stderr ---\n{}",
        stdout,
        stderr,
    );

    let expected = format!("{} {}\n", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    assert_eq!(
        stdout,
        expected,
        "--version は stdout に `{}` を出力する必要があります",
        expected.trim_end(),
    );

    assert!(
        stderr.is_empty(),
        "--version は stderr に出力してはいけません:\n{}",
        stderr,
    );
}
