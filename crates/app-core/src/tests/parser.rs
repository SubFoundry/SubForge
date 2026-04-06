use super::*;
use app_common::ProxyTransport;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::{Value, json};

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

#[test]
fn uri_list_parser_preserves_transport_tls_and_runtime_fields() {
    let parser = UriListParser;
    let vmess_payload = BASE64_STANDARD.encode(
        r#"{"v":"2","ps":"vmess-node","add":"vmess.example.com","port":"443","id":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa","aid":"2","net":"ws","host":"edge.vmess.example.com","path":"/vmess","tls":"tls","sni":"sni.vmess.example.com","scy":"auto","fp":"chrome","alpn":"h2,http/1.1","allowInsecure":"1"}"#,
    );
    let payload = format!(
        "vmess://{vmess_payload}\n\
vless://bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb@vless.example.com:443?type=grpc&security=tls&sni=sni.vless.example.com&serviceName=vless-grpc&alpn=h2%2Chttp%2F1.1&fp=chrome&allowInsecure=1&flow=xtls-rprx-vision#vless-node\n\
trojan://trojan-pass@trojan.example.com:443?type=ws&sni=sni.trojan.example.com&host=edge.trojan.example.com&path=%2Ftrojan&alpn=h2%2Chttp%2F1.1&allowInsecure=true#trojan-node"
    );

    let nodes = parser
        .parse("source-runtime-fields", &payload)
        .expect("解析带完整参数的节点应成功");
    assert_eq!(nodes.len(), 3);

    let vmess = nodes
        .iter()
        .find(|node| node.protocol == ProxyProtocol::Vmess)
        .expect("应包含 vmess 节点");
    assert_eq!(vmess.transport, ProxyTransport::Ws);
    assert!(vmess.tls.enabled);
    assert_eq!(
        vmess.tls.server_name.as_deref(),
        Some("sni.vmess.example.com")
    );
    assert_eq!(
        vmess.extra.get("host"),
        Some(&Value::String("edge.vmess.example.com".to_string()))
    );
    assert_eq!(
        vmess.extra.get("path"),
        Some(&Value::String("/vmess".to_string()))
    );
    assert_eq!(
        vmess.extra.get("skip_cert_verify"),
        Some(&Value::Bool(true))
    );
    assert_eq!(vmess.extra.get("alpn"), Some(&json!(["h2", "http/1.1"])));

    let vless = nodes
        .iter()
        .find(|node| node.protocol == ProxyProtocol::Vless)
        .expect("应包含 vless 节点");
    assert_eq!(vless.transport, ProxyTransport::Grpc);
    assert!(vless.tls.enabled);
    assert_eq!(
        vless.tls.server_name.as_deref(),
        Some("sni.vless.example.com")
    );
    assert_eq!(
        vless.extra.get("service_name"),
        Some(&Value::String("vless-grpc".to_string()))
    );
    assert_eq!(
        vless.extra.get("flow"),
        Some(&Value::String("xtls-rprx-vision".to_string()))
    );
    assert_eq!(
        vless.extra.get("skip_cert_verify"),
        Some(&Value::Bool(true))
    );

    let trojan = nodes
        .iter()
        .find(|node| node.protocol == ProxyProtocol::Trojan)
        .expect("应包含 trojan 节点");
    assert_eq!(trojan.transport, ProxyTransport::Ws);
    assert!(trojan.tls.enabled);
    assert_eq!(
        trojan.tls.server_name.as_deref(),
        Some("sni.trojan.example.com")
    );
    assert_eq!(
        trojan.extra.get("host"),
        Some(&Value::String("edge.trojan.example.com".to_string()))
    );
    assert_eq!(
        trojan.extra.get("path"),
        Some(&Value::String("/trojan".to_string()))
    );
    assert_eq!(
        trojan.extra.get("password"),
        Some(&Value::String("trojan-pass".to_string()))
    );
}
