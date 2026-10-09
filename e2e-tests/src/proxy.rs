//! E2E テスト用の TCP プロキシ。
//!
//! `CONNECT` メソッドによるトンネルを確立して通信を中継する。
//! テストから任意のタイミングで接続を切断できるため、WebSocket の Close フレームを
//! 伴わない切断などの、通常の接続操作では再現できない状況を再現するために使う。

use std::io;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use shiguredo_http11::{RequestDecoder, Response, host::Host, uri::Uri};
use sora_sdk::ProxyInfo;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// プロキシの接続先。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectTarget {
    host: String,
    port: u16,
}

#[derive(Debug, Default)]
struct ProxyTrafficStats {
    downstream_to_upstream: AtomicU64,
    upstream_to_downstream: AtomicU64,
}

impl ProxyTrafficStats {
    fn add(&self, downstream_to_upstream: u64, upstream_to_downstream: u64) {
        self.downstream_to_upstream
            .fetch_add(downstream_to_upstream, Ordering::SeqCst);
        self.upstream_to_downstream
            .fetch_add(upstream_to_downstream, Ordering::SeqCst);
    }

    fn snapshot(&self) -> (u64, u64) {
        (
            self.downstream_to_upstream.load(Ordering::SeqCst),
            self.upstream_to_downstream.load(Ordering::SeqCst),
        )
    }
}

type Result<T> = std::result::Result<T, ProxyHarnessError>;

#[derive(Debug)]
enum ProxyHarnessError {
    Io(io::Error),
    Http11(shiguredo_http11::Error),
    UnsupportedMethod(String),
    InvalidAuthority,
    PrematureClose,
}

impl ProxyHarnessError {
    fn io_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Io(err) => Some(err.kind()),
            _ => None,
        }
    }
}

impl std::fmt::Display for ProxyHarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxyHarnessError::Io(err) => write!(f, "I/O エラー: {err}"),
            ProxyHarnessError::Http11(err) => write!(f, "HTTP 解析エラー: {err}"),
            ProxyHarnessError::UnsupportedMethod(method) => {
                write!(f, "CONNECT 以外のメソッドです: {method}")
            }
            ProxyHarnessError::InvalidAuthority => {
                write!(f, "CONNECT の authority を解析できません")
            }
            ProxyHarnessError::PrematureClose => {
                write!(f, "CONNECT 要求の受信前に接続が閉じられました")
            }
        }
    }
}

impl std::error::Error for ProxyHarnessError {}

impl From<io::Error> for ProxyHarnessError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<shiguredo_http11::Error> for ProxyHarnessError {
    fn from(err: shiguredo_http11::Error) -> Self {
        Self::Http11(err)
    }
}

fn parse_authority(authority: &str, default_port: Option<u16>) -> Option<ConnectTarget> {
    let host = Host::parse(authority).ok()?;
    let port = host.port().or(default_port)?;
    let host = host
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    if host.is_empty() {
        return None;
    }
    Some(ConnectTarget { host, port })
}

/// シグナリング URL からプロキシの接続先を取り出す。
///
/// `ws` / `wss` 以外のスキームや、ホストを持たない URL の場合は `None` を返す。
pub fn parse_signaling_target(url: &str) -> Option<ConnectTarget> {
    let uri = Uri::parse(url).ok()?;
    let scheme = uri.scheme()?;
    let default_port = if scheme.eq_ignore_ascii_case("wss") {
        443
    } else if scheme.eq_ignore_ascii_case("ws") {
        80
    } else {
        return None;
    };
    let host = uri.host()?;
    let port = uri.port().unwrap_or(default_port);
    if host.is_empty() || port == 0 {
        return None;
    }
    Some(ConnectTarget {
        host: host.to_string(),
        port,
    })
}

async fn decode_connect_request(stream: &mut TcpStream) -> Result<ConnectTarget> {
    let mut decoder = RequestDecoder::new();
    let mut buf = [0u8; 2048];
    loop {
        if let Some((head, _body_kind)) = decoder.decode_headers()? {
            if !head.method().eq_ignore_ascii_case("CONNECT") {
                return Err(ProxyHarnessError::UnsupportedMethod(
                    head.method().to_string(),
                ));
            }
            return parse_authority(head.uri(), None).ok_or(ProxyHarnessError::InvalidAuthority);
        }

        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Err(ProxyHarnessError::PrematureClose);
        }
        decoder.feed(&buf[..n])?;
    }
}

async fn handle_proxy_connection(
    mut downstream: TcpStream,
    connect_log: Arc<Mutex<Vec<ConnectTarget>>>,
    traffic_stats: Arc<ProxyTrafficStats>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<()> {
    let target = decode_connect_request(&mut downstream).await?;
    connect_log
        .lock()
        .expect("connect_log のロックを取得できませんでした")
        .push(target.clone());

    let mut upstream = TcpStream::connect((target.host.as_str(), target.port)).await?;
    let response = Response::new(200, "Connection Established")
        .expect("固定値の Response::new に失敗しました");
    let encoded = response
        .encode()
        .expect("固定値の Response::encode に失敗しました");
    downstream.write_all(&encoded).await?;
    let (mut downstream_reader, mut downstream_writer) = downstream.split();
    let (mut upstream_reader, mut upstream_writer) = upstream.split();

    // 切断要求を受けたら、上流 (接続先) からの転送を止めてクライアント側の書き込みを
    // 閉じる。WebSocket の Close フレームを送らずに TCP を切断するために必要。
    // クライアントからの読み取りは、ソケットを閉じる時点で受信バッファにデータが残って
    // RST が送られないよう、クライアントが接続を閉じるまで継続する。
    let upstream_to_downstream = async {
        tokio::select! {
            result = relay_proxy_traffic(&mut upstream_reader, &mut downstream_writer) => result,
            _ = wait_for_shutdown(&mut shutdown_rx) => {
                let _ = downstream_writer.shutdown().await;
                Ok(0)
            }
        }
    };
    let (downstream_to_upstream, upstream_to_downstream) = tokio::join!(
        relay_proxy_traffic(&mut downstream_reader, &mut upstream_writer),
        upstream_to_downstream,
    );
    let downstream_to_upstream = downstream_to_upstream?;
    let upstream_to_downstream = upstream_to_downstream?;
    traffic_stats.add(downstream_to_upstream, upstream_to_downstream);
    Ok(())
}

/// 切断要求が送られるまで待つ。
///
/// 送信側が破棄された場合も、プロキシが終了して中継を続けられないため切断要求として扱う。
async fn wait_for_shutdown(shutdown_rx: &mut watch::Receiver<bool>) {
    if *shutdown_rx.borrow() {
        return;
    }
    let _ = shutdown_rx.changed().await;
}

fn is_connection_closed_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::BrokenPipe
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::NotConnected
    )
}

async fn relay_proxy_traffic<R, W>(reader: &mut R, writer: &mut W) -> io::Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut total = 0u64;
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) => {
                let _ = writer.shutdown().await;
                break;
            }
            Ok(n) => n,
            Err(err) if is_connection_closed_error(&err) => {
                let _ = writer.shutdown().await;
                break;
            }
            Err(err) => return Err(err),
        };

        if let Err(err) = writer.write_all(&buf[..n]).await {
            if is_connection_closed_error(&err) {
                break;
            }
            return Err(err);
        }
        total += n as u64;
    }
    Ok(total)
}

/// E2E テストで利用する TCP プロキシ。
///
/// 起動時に `0.0.0.0` の任意ポートで待ち受け、[`ProxyHarness::proxy_info`] が返す
/// `ProxyInfo` を接続に設定することで、シグナリングと TURN の TCP 通信を中継する。
pub struct ProxyHarness {
    proxy_url: String,
    connect_log: Arc<Mutex<Vec<ConnectTarget>>>,
    traffic_stats: Arc<ProxyTrafficStats>,
    active_connection_count: Arc<AtomicUsize>,
    shutdown_tx: watch::Sender<bool>,
    accept_task: JoinHandle<()>,
}

impl ProxyHarness {
    /// プロキシを起動する。
    ///
    /// `signaling_urls` は `ProxyInfo` に設定するホスト IP の決定に使う。
    pub async fn start(signaling_urls: &[String]) -> io::Result<Self> {
        let listener = TcpListener::bind("0.0.0.0:0").await?;
        let addr = listener.local_addr()?;
        let proxy_host = detect_proxy_host(signaling_urls).await;
        let connect_log = Arc::new(Mutex::new(Vec::new()));
        let traffic_stats = Arc::new(ProxyTrafficStats::default());
        let active_connection_count = Arc::new(AtomicUsize::new(0));
        let connect_log_for_task = connect_log.clone();
        let traffic_stats_for_task = traffic_stats.clone();
        let active_connection_count_for_task = active_connection_count.clone();
        let (shutdown_tx, _) = watch::channel(false);
        let shutdown_tx_for_task = shutdown_tx.clone();

        let accept_task = tokio::spawn(async move {
            loop {
                let Ok((stream, _peer_addr)) = listener.accept().await else {
                    break;
                };
                active_connection_count_for_task.fetch_add(1, Ordering::SeqCst);
                let connect_log = connect_log_for_task.clone();
                let traffic_stats = traffic_stats_for_task.clone();
                let active_connection_count = active_connection_count_for_task.clone();
                let shutdown_rx = shutdown_tx_for_task.subscribe();
                tokio::spawn(async move {
                    let result =
                        handle_proxy_connection(stream, connect_log, traffic_stats, shutdown_rx)
                            .await;
                    if let Err(err) = result
                        && !matches!(
                            err.io_kind(),
                            Some(io::ErrorKind::BrokenPipe)
                                | Some(io::ErrorKind::UnexpectedEof)
                                | Some(io::ErrorKind::ConnectionReset)
                                | Some(io::ErrorKind::ConnectionAborted)
                        )
                    {
                        eprintln!("proxy 接続エラー: {err}");
                    }
                    active_connection_count.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });

        Ok(Self {
            proxy_url: format!("http://{}:{}", proxy_host, addr.port()),
            connect_log,
            traffic_stats,
            active_connection_count,
            shutdown_tx,
            accept_task,
        })
    }

    /// 接続に設定する `ProxyInfo` を返す。
    pub fn proxy_info(&self) -> ProxyInfo {
        ProxyInfo {
            url: self.proxy_url.clone(),
            ..Default::default()
        }
    }

    /// これまでに受け付けた `CONNECT` の接続先の一覧を返す。
    pub fn connect_targets(&self) -> Vec<ConnectTarget> {
        self.connect_log
            .lock()
            .expect("connect_log のロックを取得できませんでした")
            .clone()
    }

    /// 下流から上流、上流から下流へ中継したバイト数を返す。
    pub fn transferred_bytes(&self) -> (u64, u64) {
        self.traffic_stats.snapshot()
    }

    /// 現在中継中の接続数を返す。
    pub fn active_connection_count(&self) -> usize {
        self.active_connection_count.load(Ordering::SeqCst)
    }

    /// 中継中の接続がすべて閉じるまで待つ。
    ///
    /// タイムアウト内に閉じた場合は true を返す。
    pub async fn wait_for_all_connections_closed(&self, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, async {
            loop {
                if self.active_connection_count() == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .is_ok()
    }

    /// 中継中の接続をすべて切断する。
    ///
    /// 上流からの転送を止めてクライアント側の書き込みを閉じるため、
    /// クライアントは WebSocket の Close フレームを伴わない TCP の切断を観測する。
    pub fn drop_connections(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}

impl Drop for ProxyHarness {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

/// Proxy 用 URL に設定するホスト IP を推定する。
///
/// `signaling_urls` の宛先へ送信するときに OS が選ぶローカル IP を取得し、loopback
/// 以外の IP が得られたらそれを返す。いずれかの手順が失敗した候補は判定できない
/// ものとして次の候補へ進み、どの候補でも得られない場合は `127.0.0.1` を返す。
async fn detect_proxy_host(signaling_urls: &[String]) -> String {
    // `127.0.0.1` ではなく loopback 以外のローカル IP を選ぶ理由:
    // `libwebrtc` に HTTP Proxy を設定して通信させた場合、Windows では non-loopback の
    // ローカル IP に bind したソケットで `127.0.0.1` へ connect しようとすると失敗する
    // ことがある (`WSAEADDRNOTAVAIL / 10049`)。
    // proxy URL を常に `127.0.0.1` に固定すると `CONNECT` が proxy まで到達せずテストが
    // 不安定になる環境があるため、実際に使われる経路に合わせて IP を選ぶ。
    // libwebrtc が bind している場所:
    // https://source.chromium.org/chromium/chromium/src/+/main:third_party/webrtc/p2p/base/basic_packet_socket_factory.cc;l=156;drc=61721239a70cffde6dd7b56241f1e3360fb3d6ee
    for url in signaling_urls {
        // `ws://` / `wss://` を `host:port` へ変換できない URL は、
        // 経路判定の入力として使えないためスキップする。
        let Some(target) = parse_signaling_target(url) else {
            continue;
        };

        // `0.0.0.0:0` は「任意インターフェース + 任意空きポート」で bind する指定。
        // ここで重要なのは、特定 NIC を固定せず OS の経路選択に任せること。
        let Ok(socket) = UdpSocket::bind("0.0.0.0:0").await else {
            continue;
        };

        // signaling 宛先へ UDP connect して、OS に送信経路を選ばせる。
        // ここで得たいのは通信成功ではなく「どのローカル IP が選ばれるか」。
        if socket
            .connect((target.host.as_str(), target.port))
            .await
            .is_err()
        {
            continue;
        }

        // 上記 connect の結果、OS が決めたローカル側の `IP:port` を取得する。
        let Ok(addr) = socket.local_addr() else {
            continue;
        };

        // loopback (`127.0.0.1` / `::1`) は Windows の connect 問題を避けるため意図的に除外する。
        match addr.ip() {
            std::net::IpAddr::V4(ip) if !ip.is_loopback() => return ip.to_string(),
            std::net::IpAddr::V6(ip) if !ip.is_loopback() => return ip.to_string(),
            _ => {}
        }
    }

    // 最後のフォールバック。最悪でもローカルだけで動かすための保険であり、
    // 上記の Windows の connect 問題を完全に回避する保証はない。
    "127.0.0.1".to_string()
}
