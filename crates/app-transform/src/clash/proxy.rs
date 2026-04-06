use app_common::{ProxyNode, ProxyProtocol, ProxyTransport};

use crate::clash::{ClashGrpcOptions, ClashH2Options, ClashProxy, ClashWsOptions};
use crate::shared::{
    clash_network, optional_bool, optional_string, optional_string_list, optional_u32,
    required_string,
};
use crate::{TransformError, TransformResult};

pub(super) fn build_clash_proxy(node: &ProxyNode) -> TransformResult<ClashProxy> {
    let network = Some(clash_network(&node.transport).to_string());
    let ws_opts = build_ws_options(node);
    let grpc_opts = build_grpc_options(node);
    let h2_opts = build_h2_options(node);
    let tls_enabled = Some(node.tls.enabled);
    let servername = node.tls.server_name.clone();
    let sni = node
        .tls
        .server_name
        .clone()
        .or_else(|| optional_string(node, "sni"));
    let skip_cert_verify = optional_bool(node, "skip_cert_verify");

    let mut proxy = ClashProxy {
        name: node.name.clone(),
        proxy_type: String::new(),
        server: node.server.clone(),
        port: node.port,
        cipher: None,
        password: None,
        uuid: None,
        alter_id: None,
        udp: Some(true),
        tls: tls_enabled,
        sni,
        servername,
        network: None,
        flow: None,
        skip_cert_verify,
        client_fingerprint: optional_string(node, "client_fingerprint"),
        ws_opts,
        grpc_opts,
        h2_opts,
        alpn: optional_string_list(node, "alpn"),
        obfs: optional_string(node, "obfs"),
        obfs_password: optional_string(node, "obfs_password"),
        congestion_control: optional_string(node, "congestion_control"),
        udp_relay_mode: optional_string(node, "udp_relay_mode"),
    };

    match node.protocol {
        ProxyProtocol::Ss => {
            proxy.proxy_type = "ss".to_string();
            proxy.cipher = Some(required_string(node, "cipher")?);
            proxy.password = Some(required_string(node, "password")?);
            proxy.network = Some("tcp".to_string());
            proxy.tls = None;
            proxy.sni = None;
            proxy.servername = None;
            proxy.ws_opts = None;
            proxy.grpc_opts = None;
            proxy.h2_opts = None;
            proxy.skip_cert_verify = None;
            proxy.client_fingerprint = None;
            proxy.alpn = None;
            proxy.obfs = None;
            proxy.obfs_password = None;
            proxy.congestion_control = None;
            proxy.udp_relay_mode = None;
        }
        ProxyProtocol::Vmess => {
            proxy.proxy_type = "vmess".to_string();
            proxy.uuid = Some(required_string(node, "uuid")?);
            proxy.alter_id = optional_u32(node, "alter_id").or(Some(0));
            proxy.cipher = optional_string(node, "cipher").or(Some("auto".to_string()));
            proxy.network = network;
            proxy.flow = None;
        }
        ProxyProtocol::Vless => {
            proxy.proxy_type = "vless".to_string();
            proxy.uuid = Some(required_string(node, "uuid")?);
            proxy.network = network;
            proxy.flow = optional_string(node, "flow");
            proxy.alter_id = None;
            proxy.cipher = None;
        }
        ProxyProtocol::Trojan => {
            proxy.proxy_type = "trojan".to_string();
            proxy.password = Some(required_string(node, "password")?);
            proxy.network = network;
            proxy.sni = proxy.servername.clone();
            proxy.alter_id = None;
            proxy.cipher = None;
            proxy.uuid = None;
            proxy.flow = None;
        }
        ProxyProtocol::Hysteria2 => {
            proxy.proxy_type = "hysteria2".to_string();
            proxy.password = Some(
                optional_string(node, "password")
                    .or_else(|| optional_string(node, "auth"))
                    .ok_or_else(|| TransformError::MissingField {
                        node_name: node.name.clone(),
                        field: "password/auth",
                    })?,
            );
            proxy.network = None;
            proxy.uuid = None;
            proxy.flow = None;
            proxy.alter_id = None;
            proxy.cipher = None;
            proxy.grpc_opts = None;
            proxy.h2_opts = None;
            proxy.ws_opts = None;
        }
        ProxyProtocol::Tuic => {
            proxy.proxy_type = "tuic".to_string();
            proxy.uuid = Some(required_string(node, "uuid")?);
            proxy.password = Some(required_string(node, "password")?);
            proxy.network = None;
            proxy.flow = None;
            proxy.alter_id = None;
            proxy.cipher = None;
            proxy.grpc_opts = None;
            proxy.h2_opts = None;
            proxy.ws_opts = None;
        }
    }

    Ok(proxy)
}

fn build_ws_options(node: &ProxyNode) -> Option<ClashWsOptions> {
    if !matches!(node.transport, ProxyTransport::Ws) {
        return None;
    }

    let mut headers = std::collections::BTreeMap::new();
    if let Some(host) = optional_string(node, "host") {
        headers.insert("Host".to_string(), host);
    }

    Some(ClashWsOptions {
        path: optional_string(node, "path").unwrap_or_else(|| "/".to_string()),
        headers: (!headers.is_empty()).then_some(headers),
        max_early_data: optional_u32(node, "max_early_data"),
        early_data_header_name: optional_string(node, "early_data_header_name"),
    })
}

fn build_grpc_options(node: &ProxyNode) -> Option<ClashGrpcOptions> {
    if !matches!(node.transport, ProxyTransport::Grpc) {
        return None;
    }

    Some(ClashGrpcOptions {
        grpc_service_name: optional_string(node, "grpc_service_name")
            .or_else(|| optional_string(node, "service_name"))
            .unwrap_or_else(|| "grpc".to_string()),
    })
}

fn build_h2_options(node: &ProxyNode) -> Option<ClashH2Options> {
    if !matches!(node.transport, ProxyTransport::H2) {
        return None;
    }

    let host = optional_string_list(node, "host");
    Some(ClashH2Options {
        host,
        path: optional_string(node, "path"),
    })
}
