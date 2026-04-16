use std::collections::HashSet;
use std::io;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use e2e_tests::{
    FakeVideoCapturer, FakeVideoCapturerConfig, SoraTestConnection,
    build_metadata_with_access_token, build_sender_tracks, generate_channel_id, load_env,
    secret_key, signaling_urls, sum_video_stats_field_for_type, verify_video_stats_field_positive,
};
use shiguredo_http11::{RequestDecoder, Response, host::Host, uri::Uri};
use sora_sdk::{ProxyInfo, Role, SoraConnectionContext};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::task::JoinHandle;

fn test_channel_id(suffix: &str) -> String {
    let base = generate_channel_id();
    format!("{}-{}", base, suffix)
}

const MIN_PROXY_TRANSFER_BYTES_PER_DIRECTION: u64 = 4 * 1024;
const MIN_PROXY_TRANSFER_BYTES_TOTAL: u64 = 12 * 1024;
const MIN_RTP_BYTES_PER_CLIENT: u64 = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConnectTarget {
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
                write!(f, "CONNECT 以外のメソッドはサポートしません: {method}")
            }
            ProxyHarnessError::InvalidAuthority => {
                f.write_str("CONNECT リクエストの authority が不正です")
            }
            ProxyHarnessError::PrematureClose => {
                f.write_str("CONNECT リクエスト受信前に接続が閉じられました")
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

fn parse_signaling_target(url: &str) -> Option<ConnectTarget> {
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
            if !head.method.eq_ignore_ascii_case("CONNECT") {
                return Err(ProxyHarnessError::UnsupportedMethod(head.method));
            }
            return parse_authority(&head.uri, None).ok_or(ProxyHarnessError::InvalidAuthority);
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
) -> Result<()> {
    let target = decode_connect_request(&mut downstream).await?;
    connect_log.lock().unwrap().push(target.clone());

    let mut upstream = TcpStream::connect((target.host.as_str(), target.port)).await?;
    let response = Response::new(200, "Connection Established");
    downstream.write_all(&response.encode()).await?;
    let (mut downstream_reader, mut downstream_writer) = downstream.split();
    let (mut upstream_reader, mut upstream_writer) = upstream.split();
    let (downstream_to_upstream, upstream_to_downstream) = tokio::join!(
        relay_proxy_traffic(&mut downstream_reader, &mut upstream_writer),
        relay_proxy_traffic(&mut upstream_reader, &mut downstream_writer),
    );
    let downstream_to_upstream = downstream_to_upstream?;
    let upstream_to_downstream = upstream_to_downstream?;
    traffic_stats.add(downstream_to_upstream, upstream_to_downstream);
    Ok(())
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

struct ProxyHarness {
    proxy_url: String,
    connect_log: Arc<Mutex<Vec<ConnectTarget>>>,
    traffic_stats: Arc<ProxyTrafficStats>,
    active_connection_count: Arc<AtomicUsize>,
    accept_task: JoinHandle<()>,
}

impl ProxyHarness {
    async fn start(signaling_urls: &[String]) -> Result<Self> {
        let listener = TcpListener::bind("0.0.0.0:0").await?;
        let addr = listener.local_addr()?;
        let proxy_host = detect_proxy_host(signaling_urls).await;
        let connect_log = Arc::new(Mutex::new(Vec::new()));
        let traffic_stats = Arc::new(ProxyTrafficStats::default());
        let active_connection_count = Arc::new(AtomicUsize::new(0));
        let connect_log_for_task = connect_log.clone();
        let traffic_stats_for_task = traffic_stats.clone();
        let active_connection_count_for_task = active_connection_count.clone();

        let accept_task = tokio::spawn(async move {
            loop {
                let Ok((stream, _peer_addr)) = listener.accept().await else {
                    break;
                };
                active_connection_count_for_task.fetch_add(1, Ordering::SeqCst);
                let connect_log = connect_log_for_task.clone();
                let traffic_stats = traffic_stats_for_task.clone();
                let active_connection_count = active_connection_count_for_task.clone();
                tokio::spawn(async move {
                    let result = handle_proxy_connection(stream, connect_log, traffic_stats).await;
                    if let Err(err) = result
                        && !matches!(
                            err.io_kind(),
                            Some(io::ErrorKind::BrokenPipe)
                                | Some(io::ErrorKind::UnexpectedEof)
                                | Some(io::ErrorKind::ConnectionReset)
                                | Some(io::ErrorKind::ConnectionAborted)
                        )
                    {
                        eprintln!("proxy connection error: {err}");
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
            accept_task,
        })
    }

    fn proxy_info(&self) -> ProxyInfo {
        ProxyInfo {
            url: self.proxy_url.clone(),
            ..Default::default()
        }
    }

    fn connect_targets(&self) -> Vec<ConnectTarget> {
        self.connect_log.lock().unwrap().clone()
    }

    fn transferred_bytes(&self) -> (u64, u64) {
        self.traffic_stats.snapshot()
    }

    fn active_connection_count(&self) -> usize {
        self.active_connection_count.load(Ordering::SeqCst)
    }

    async fn wait_for_all_connections_closed(&self, timeout: Duration) -> bool {
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
}

impl Drop for ProxyHarness {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

/// Proxy 用 URL に設定するホスト IP を推定する。
///
/// 背景:
/// - このテストは `libwebrtc` に HTTP Proxy を設定して通信させる。
/// - Windows 環境では、`libwebrtc` 側が non-loopback のローカル IP に bind した
///   ソケットで `127.0.0.1` へ connect しようとすると失敗するケースがある
///   (`WSAEADDRNOTAVAIL / 10049`)。
///   - 実際に bind している場所: https://source.chromium.org/chromium/chromium/src/+/main:third_party/webrtc/p2p/base/basic_packet_socket_factory.cc;l=156;drc=61721239a70cffde6dd7b56241f1e3360fb3d6ee
/// - そのため proxy URL を常に `127.0.0.1` に固定すると、環境によっては
///   `CONNECT` が proxy まで到達せず、テストが不安定になる。
///
/// 目的:
/// - `libwebrtc` が実際に使いそうな経路に合わせて、proxy URL に使うホスト IP を
///   できるだけ妥当に選ぶ。
///
/// 方式:
/// - 各 signaling URL から `host:port` を取り出す。
/// - `UdpSocket::bind("0.0.0.0:0")` でローカル UDP ソケットを作成し、
///   その宛先へ `connect` する。
/// - UDP の `connect` は TCP のような接続確立ではなく、主に「その宛先へ送るなら
///   どのローカル IP / NIC を使うか」を OS に選ばせるために使う。
/// - 直後に `local_addr()` を読むと、OS が選んだ送信元ローカル IP が取れる。
/// - loopback ではない IP が得られたら、それを proxy URL の host として返す。
///
/// 失敗時方針:
/// - URL 解析失敗、ソケット作成失敗、`connect` 失敗、`local_addr` 取得失敗は
///   すべて「その URL では判定できない」とみなして次候補へ進む。
/// - 最後まで有効な候補が得られない場合のみ `127.0.0.1` にフォールバックする。
///   これは「最悪でもローカルだけで動かす」という保険であり、上記 Windows 問題を
///   完全回避する保証ではない。
async fn detect_proxy_host(signaling_urls: &[String]) -> String {
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

        // loopback 以外のアドレスが得られた場合、その IP を proxy URL に採用する。
        // - v4 / v6 の両方に対応する
        // - loopback (`127.0.0.1` / `::1`) は意図的に除外する
        //   (Windows の `non-loopback bind -> loopback connect` 問題を避けるため)
        match addr.ip() {
            std::net::IpAddr::V4(ip) if !ip.is_loopback() => return ip.to_string(),
            std::net::IpAddr::V6(ip) if !ip.is_loopback() => return ip.to_string(),
            _ => {}
        }
    }

    // 候補が 1 つも得られない場合の最終フォールバック。
    "127.0.0.1".to_string()
}

#[tokio::test]
async fn test_sendrecv_bidirectional_via_proxy() {
    load_env();

    let urls = signaling_urls().expect("TEST_SIGNALING_URLS が必要");
    let expected_signaling_targets: HashSet<ConnectTarget> = urls
        .iter()
        .filter_map(|url| parse_signaling_target(url))
        .collect();
    assert!(
        !expected_signaling_targets.is_empty(),
        "TEST_SIGNALING_URLS の解析に失敗しました"
    );
    let proxy = ProxyHarness::start(&urls)
        .await
        .expect("テスト用 Proxy の起動に失敗しました");
    let proxy_info = proxy.proxy_info();
    let channel_id = test_channel_id("sendrecv-via-proxy");

    let context1 = SoraConnectionContext::new().expect("クライアント 1 コンテキスト作成失敗");
    let mut capturer1 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 1 作成失敗");
    let (video_track1, audio_track1) =
        build_sender_tracks(&context1, &mut capturer1).expect("送信用トラック作成失敗");

    let mut builder1 =
        SoraTestConnection::builder(context1, urls.clone(), channel_id.clone(), Role::SendRecv)
            .sender_video_track(video_track1)
            .sender_audio_track(audio_track1)
            .proxy(proxy_info.clone())
            .data_channel_signaling(true)
            .disconnect_wait_timeout(Duration::from_secs(1));

    if let Some(token) = secret_key() {
        builder1 = builder1.metadata(build_metadata_with_access_token(&token));
    }

    let mut client1 = builder1
        .connect()
        .expect("SoraTestConnection 1 の作成に失敗しました");
    client1
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の接続がタイムアウトしました");

    let context2 = SoraConnectionContext::new().expect("クライアント 2 コンテキスト作成失敗");
    let mut capturer2 = FakeVideoCapturer::new(FakeVideoCapturerConfig::default())
        .expect("FakeVideoCapturer 2 作成失敗");
    let (video_track2, audio_track2) =
        build_sender_tracks(&context2, &mut capturer2).expect("送信用トラック作成失敗");

    let mut builder2 =
        SoraTestConnection::builder(context2, urls.clone(), channel_id, Role::SendRecv)
            .sender_video_track(video_track2)
            .sender_audio_track(audio_track2)
            .proxy(proxy_info)
            .data_channel_signaling(true)
            .disconnect_wait_timeout(Duration::from_secs(1))
            .ice_server_url_configurer(|server, urls| {
                for url in urls {
                    // 必ず TURN-TCP または TURN-TLS に接続してほしいので、transport=tcp を含む URL のみ追加する
                    if url.contains("transport=tcp") {
                        server.add_url(url);
                    }
                }
            });

    if let Some(token) = secret_key() {
        builder2 = builder2.metadata(build_metadata_with_access_token(&token));
    }

    let mut client2 = builder2
        .connect()
        .expect("SoraTestConnection 2 の作成に失敗しました");
    client2
        .wait_for_connect(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の接続がタイムアウトしました");
    client1
        .wait_for_video_track(Duration::from_secs(15))
        .await
        .expect("クライアント 1 の on_track 受信待機がタイムアウトしました");
    client2
        .wait_for_video_track(Duration::from_secs(15))
        .await
        .expect("クライアント 2 の on_track 受信待機がタイムアウトしました");

    client1
        .wait_video_outbound_packets_sent(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の outbound-rtp packetsSent が 0 より大きくなりませんでした");
    client2
        .wait_video_outbound_packets_sent(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の outbound-rtp packetsSent が 0 より大きくなりませんでした");
    client1
        .wait_video_inbound_packets_received(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の inbound-rtp packetsReceived が 0 より大きくなりませんでした");
    client2
        .wait_video_inbound_packets_received(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の inbound-rtp packetsReceived が 0 より大きくなりませんでした");

    client1
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && sum_video_stats_field_for_type(stats, "outbound-rtp", "bytesSent")
                        >= MIN_RTP_BYTES_PER_CLIENT
                    && sum_video_stats_field_for_type(stats, "inbound-rtp", "bytesReceived")
                        >= MIN_RTP_BYTES_PER_CLIENT
            },
            Duration::from_secs(15),
        )
        .await
        .expect("クライアント 1 の stats が期待値に到達しませんでした");
    client2
        .wait_stats(
            |stats| {
                verify_video_stats_field_positive(stats, "outbound-rtp", "packetsSent")
                    && verify_video_stats_field_positive(stats, "inbound-rtp", "packetsReceived")
                    && sum_video_stats_field_for_type(stats, "outbound-rtp", "bytesSent")
                        >= MIN_RTP_BYTES_PER_CLIENT
                    && sum_video_stats_field_for_type(stats, "inbound-rtp", "bytesReceived")
                        >= MIN_RTP_BYTES_PER_CLIENT
            },
            Duration::from_secs(15),
        )
        .await
        .expect("クライアント 2 の stats が期待値に到達しませんでした");

    client1
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 1 の disconnect に失敗しました");
    client2
        .disconnect_and_wait(Duration::from_secs(10))
        .await
        .expect("クライアント 2 の disconnect に失敗しました");
    assert!(
        proxy
            .wait_for_all_connections_closed(Duration::from_secs(5))
            .await,
        "Proxy 接続がクローズされませんでした: active_connection_count={}",
        proxy.active_connection_count()
    );

    // 2 クライアントが urls.len() + TURN 回以上接続しているはず
    let connect_targets = proxy.connect_targets();
    assert!(
        connect_targets.len() >= (urls.len() + 1) * 2,
        "Proxy の CONNECT 回数が不足しています: actual({}) >= expected({})",
        connect_targets.len(),
        (urls.len() + 1) * 2
    );
    let signaling_connect_count = connect_targets
        .iter()
        .filter(|target| expected_signaling_targets.contains(*target))
        .count();
    assert!(
        signaling_connect_count >= 2,
        "シグナリング宛先への CONNECT 回数が不足しています: signaling_connect_count={}",
        signaling_connect_count
    );
    let (downstream_to_upstream_bytes, upstream_to_downstream_bytes) = proxy.transferred_bytes();
    assert!(
        downstream_to_upstream_bytes >= MIN_PROXY_TRANSFER_BYTES_PER_DIRECTION,
        "Proxy の下流→上流バイト数が不足しています: bytes={}, min={}",
        downstream_to_upstream_bytes,
        MIN_PROXY_TRANSFER_BYTES_PER_DIRECTION
    );
    assert!(
        upstream_to_downstream_bytes >= MIN_PROXY_TRANSFER_BYTES_PER_DIRECTION,
        "Proxy の上流→下流バイト数が不足しています: bytes={}, min={}",
        upstream_to_downstream_bytes,
        MIN_PROXY_TRANSFER_BYTES_PER_DIRECTION
    );
    assert!(
        downstream_to_upstream_bytes + upstream_to_downstream_bytes
            >= MIN_PROXY_TRANSFER_BYTES_TOTAL,
        "Proxy の総転送バイト数が不足しています: downstream_to_upstream={}, upstream_to_downstream={}, total_min={}",
        downstream_to_upstream_bytes,
        upstream_to_downstream_bytes,
        MIN_PROXY_TRANSFER_BYTES_TOTAL
    );
}
