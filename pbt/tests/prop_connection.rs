use proptest::prelude::*;
use sora_sdk::{Error, ParsedProxyInfo, ProxyInfo};

fn proxy_info_with_url(url: String) -> ProxyInfo {
    ProxyInfo {
        url,
        ..Default::default()
    }
}

proptest! {
    #[test]
    fn parse_proxy_url_accepts_http(
        label in "[a-z][a-z0-9]{0,15}",
        port in 1u16..=65535
    ) {
        let proxy = proxy_info_with_url(format!("http://{label}:{port}"));
        let parsed = ParsedProxyInfo::parse(&proxy).expect("http proxy URL の解析に失敗しました");
        prop_assert_eq!(parsed.host(), label);
        prop_assert_eq!(parsed.port(), port);
    }

    #[test]
    fn parse_proxy_url_rejects_https(
        label in "[a-z][a-z0-9]{0,15}",
        port in 1u16..=65535
    ) {
        let proxy = proxy_info_with_url(format!("https://{label}:{port}"));
        let err = ParsedProxyInfo::parse(&proxy).expect_err("https proxy URL は拒否される必要があります");
        match err {
            Error::ProxyUrlUnsupportedScheme { .. } => {}
            _ => prop_assert!(false),
        }
    }

    #[test]
    fn parse_proxy_url_rejects_socks(
        label in "[a-z][a-z0-9]{0,15}",
        port in 1u16..=65535,
        scheme in prop_oneof![Just("socks"), Just("socks4"), Just("socks5")]
    ) {
        let proxy = proxy_info_with_url(format!("{scheme}://{label}:{port}"));
        let err = ParsedProxyInfo::parse(&proxy).expect_err("socks proxy URL は拒否される必要があります");
        match err {
            Error::ProxyUrlUnsupportedScheme { .. } => {}
            _ => prop_assert!(false),
        }
    }

    #[test]
    fn parse_proxy_url_rejects_userinfo(
        label in "[a-z][a-z0-9]{0,15}",
        port in 1u16..=65535
    ) {
        let proxy = proxy_info_with_url(format!("http://user:pass@{label}:{port}"));
        let err = ParsedProxyInfo::parse(&proxy).expect_err("userinfo 付き proxy URL は拒否される必要があります");
        prop_assert!(matches!(err, Error::ProxyUrlUserinfoNotSupported));
    }
}
