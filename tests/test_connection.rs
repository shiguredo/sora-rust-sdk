use sora_sdk::{ParsedProxyInfo, ProxyInfo};

fn proxy_info_with_url(url: String) -> ProxyInfo {
    ProxyInfo {
        url,
        ..Default::default()
    }
}

#[test]
fn parsed_proxy_info_accessors_all_fields() {
    let proxy = proxy_info_with_url("http://proxy.example.com:8080".to_string());
    let parsed = ParsedProxyInfo::parse(&proxy).expect("proxy URL の解析に失敗しました");
    assert_eq!(parsed.host(), "proxy.example.com");
    assert_eq!(parsed.port(), 8080);
    assert!(parsed.username().is_none());
    assert!(parsed.password().is_none());
    assert!(!parsed.user_agent().is_empty());
}

#[test]
fn parsed_proxy_info_username_password_none() {
    let proxy = proxy_info_with_url("http://host:1234".to_string());
    let parsed = ParsedProxyInfo::parse(&proxy).expect("proxy URL の解析に失敗しました");
    assert!(parsed.username().is_none());
    assert!(parsed.password().is_none());
}
