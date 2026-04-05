use super::*;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;

#[test]
fn uri_list_parser_supports_base64_and_skips_invalid_lines() {
    let parser = UriListParser;
    let nodes = parser
        .parse("source-fixture", BASE64_SUBSCRIPTION_FIXTURE)
        .expect("解析 fixture 应成功");

    assert_eq!(nodes.len(), 3);
    let protocols = nodes
        .iter()
        .map(|node| node.protocol.clone())
        .collect::<HashSet<_>>();
    assert!(protocols.contains(&ProxyProtocol::Ss));
    assert!(protocols.contains(&ProxyProtocol::Vmess));
    assert!(protocols.contains(&ProxyProtocol::Trojan));
}

#[test]
fn uri_list_parser_handles_invalid_protocol_lines_without_failing() {
    let parser = UriListParser;
    let payload = "not-uri\nvmess://invalid\nss://invalid\nvless://missing-port";
    let nodes = parser
        .parse("source-invalid", payload)
        .expect("解析过程应不中断");

    assert!(nodes.is_empty());
}

#[test]
fn uri_list_parser_decodes_percent_encoded_node_names() {
    let parser = UriListParser;
    let vmess_payload = BASE64_STANDARD.encode(
        r#"{"v":"2","ps":"%E6%97%A5%E6%9C%AC-%E4%B8%9C%E4%BA%AC","add":"example.com","port":"443","id":"11111111-1111-1111-1111-111111111111","aid":"0","net":"tcp","type":"none","host":"","path":"","tls":"tls"}"#,
    );
    let payload = format!(
        "ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ=@example.com:443#%E9%A6%99%E6%B8%AF-01\n\
vless://11111111-1111-1111-1111-111111111111@example.com:443?type=tcp#%E7%BE%8E%E5%9B%BD-02\n\
trojan://password@example.com:443#%E5%8F%B0%E6%B9%BE-03\n\
vmess://{vmess_payload}"
    );
    let nodes = parser
        .parse("source-name-decode", &payload)
        .expect("解析 percent-encoded 名称应成功");

    assert_eq!(nodes.len(), 4);
    let names = nodes
        .into_iter()
        .map(|node| node.name)
        .collect::<HashSet<_>>();
    assert!(names.contains("香港-01"));
    assert!(names.contains("美国-02"));
    assert!(names.contains("台湾-03"));
    assert!(names.contains("日本-东京"));
}
