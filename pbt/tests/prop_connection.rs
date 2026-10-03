//! `src/connection.rs` の `ParsedProxyInfo` に対する PBT。
//!
//! プロキシ URL のパースが受理・拒否すべき入力クラスを網羅する。

use std::cell::Cell;

use sora_sdk::{Error, ParsedProxyInfo, ProxyInfo};

/// noprop の seed を指定する環境変数名。
const SEED_ENV_VAR: &str = "SORA_RUST_SDK_PBT_SEED";

fn proxy_info_with_url(url: String) -> ProxyInfo {
    ProxyInfo {
        url,
        ..Default::default()
    }
}

/// プロキシホスト名として有効な `[a-z][a-z0-9]{0,15}` 形式の文字列を生成する。
///
/// 長さの境界 (1 と 16) を有意な確率で含める。
fn sample_proxy_host(ctx: &mut noprop::TestCaseContext) -> String {
    let len =
        noprop::sample_with_boundaries(ctx, &[1usize, 16], noprop::Ratio::one_nth(4), |ctx| {
            noprop::sample_usize_in(ctx, 1..=16)
        });
    let mut host = String::new();
    for i in 0..len {
        if i == 0 {
            // 先頭は英小文字
            host.push((b'a' + noprop::sample_usize_in(ctx, 0..26) as u8) as char);
        } else {
            // 2 文字目以降は英小文字または数字
            let c = noprop::sample_usize_in(ctx, 0..36);
            host.push(if c < 26 {
                (b'a' + c as u8) as char
            } else {
                (b'0' + (c - 26) as u8) as char
            });
        }
    }
    host
}

/// プロキシポートとして有効な 1..=65535 の値を生成する。
///
/// 境界 (1 と 65535) を有意な確率で含める。
fn sample_proxy_port(ctx: &mut noprop::TestCaseContext) -> u16 {
    noprop::sample_with_boundaries(ctx, &[1usize, 65535], noprop::Ratio::one_nth(4), |ctx| {
        noprop::sample_usize_in(ctx, 1..=65535)
    }) as u16
}

#[test]
fn parse_proxy_url_accepts_http() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time(SEED_ENV_VAR)?;
    // 境界値 (ホスト長 1 / 16、ポート 1 / 65535) の到達回数を数える
    let short_host = Cell::new(0usize);
    let long_host = Cell::new(0usize);
    let low_port = Cell::new(0usize);
    let high_port = Cell::new(0usize);
    let mut runner = noprop::Runner::new(seed);

    runner.run(256, |ctx| {
        let host = sample_proxy_host(ctx);
        let port = sample_proxy_port(ctx);
        let proxy = proxy_info_with_url(format!("http://{host}:{port}"));
        let parsed = ParsedProxyInfo::parse(&proxy).expect("http proxy URL の解析に失敗しました");
        assert_eq!(parsed.host(), host, "ホスト名が入力と一致しません");
        assert_eq!(parsed.port(), port, "ポート番号が入力と一致しません");

        // 到達ゲートは不変条件の評価地点で数える
        if host.len() == 1 {
            short_host.set(short_host.get() + 1);
        } else if host.len() == 16 {
            long_host.set(long_host.get() + 1);
        }
        if port == 1 {
            low_port.set(low_port.get() + 1);
        } else if port == 65535 {
            high_port.set(high_port.get() + 1);
        }
        Ok(())
    })?;

    assert!(
        short_host.get() > 0,
        "ホスト長 1 のケースが実行されていません\n{runner}"
    );
    assert!(
        long_host.get() > 0,
        "ホスト長 16 のケースが実行されていません\n{runner}"
    );
    assert!(
        low_port.get() > 0,
        "ポート 1 のケースが実行されていません\n{runner}"
    );
    assert!(
        high_port.get() > 0,
        "ポート 65535 のケースが実行されていません\n{runner}"
    );
    Ok(())
}

#[test]
fn parse_proxy_url_rejects_https() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time(SEED_ENV_VAR)?;
    let mut runner = noprop::Runner::new(seed);

    runner.run(256, |ctx| {
        let host = sample_proxy_host(ctx);
        let port = sample_proxy_port(ctx);
        let proxy = proxy_info_with_url(format!("https://{host}:{port}"));
        let err =
            ParsedProxyInfo::parse(&proxy).expect_err("https proxy URL は拒否される必要があります");
        assert!(
            matches!(err, Error::ProxyUrlUnsupportedScheme { .. }),
            "ProxyUrlUnsupportedScheme 以外のエラーです: {err:?}"
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn parse_proxy_url_rejects_socks() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time(SEED_ENV_VAR)?;
    let mut runner = noprop::Runner::new(seed);

    runner.run(256, |ctx| {
        let host = sample_proxy_host(ctx);
        let port = sample_proxy_port(ctx);
        let scheme = noprop::sample_choice(ctx, &["socks", "socks4", "socks5"]);
        let proxy = proxy_info_with_url(format!("{scheme}://{host}:{port}"));
        let err =
            ParsedProxyInfo::parse(&proxy).expect_err("socks proxy URL は拒否される必要があります");
        assert!(
            matches!(err, Error::ProxyUrlUnsupportedScheme { .. }),
            "ProxyUrlUnsupportedScheme 以外のエラーです: {err:?}"
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn parse_proxy_url_rejects_userinfo() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time(SEED_ENV_VAR)?;
    let mut runner = noprop::Runner::new(seed);

    runner.run(256, |ctx| {
        let host = sample_proxy_host(ctx);
        let port = sample_proxy_port(ctx);
        let proxy = proxy_info_with_url(format!("http://user:pass@{host}:{port}"));
        let err = ParsedProxyInfo::parse(&proxy)
            .expect_err("userinfo 付き proxy URL は拒否される必要があります");
        assert!(
            matches!(err, Error::ProxyUrlUserinfoNotSupported),
            "ProxyUrlUserinfoNotSupported 以外のエラーです: {err:?}"
        );
        Ok(())
    })?;
    Ok(())
}
