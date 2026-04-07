use std::collections::BTreeMap;

use app_common::{ProxyNode, ProxyProtocol, ProxyTransport};
use serde::Serialize;

use crate::shared::{
    optional_bool, optional_string, optional_string_list, optional_u32, required_string,
};
use crate::{TransformError, TransformResult};

use super::SingboxOutbound;

pub(super) fn build_singbox_node_outbound(node: &ProxyNode) -> TransformResult<SingboxOutbound> {
    let tls = build_singbox_tls(node);
    let transport = build_singbox_transport(node);

    let mut outbound = SingboxOutbound {
        outbound_type: String::new(),
        tag: node.name.clone(),
        outbounds: None,
        default: None,
        url: None,
        interval: None,
        tolerance: None,
        server: Some(node.server.clone()),
        server_port: Some(node.port),
        method: None,
        password: None,
        uuid: None,
        security: None,
        alter_id: None,
        flow: None,
        network: None,
        tls,
        transport: None,
        obfs: None,
        congestion_control: None,
        udp_relay_mode: None,
    };

    match node.protocol {
        ProxyProtocol::Ss => {
            outbound.outbound_type = "shadowsocks".to_string();
            outbound.method = Some(required_string(node, "cipher")?);
            outbound.password = Some(required_string(node, "password")?);
            outbound.tls = None;
            outbound.transport = None;
        }
        ProxyProtocol::Vmess => {
            outbound.outbound_type = "vmess".to_string();
            outbound.uuid = Some(required_string(node, "uuid")?);
            outbound.security = optional_string(node, "security")
                .or_else(|| optional_string(node, "cipher"))
                .or(Some("auto".to_string()));
            outbound.alter_id = optional_u32(node, "alter_id").or(Some(0));
            outbound.network = Some("tcp".to_string());
            outbound.transport = transport;
        }
        ProxyProtocol::Vless => {
            outbound.outbound_type = "vless".to_string();
            outbound.uuid = Some(required_string(node, "uuid")?);
            outbound.flow = optional_string(node, "flow");
            outbound.network = Some("tcp".to_string());
            outbound.transport = transport;
        }
        ProxyProtocol::Trojan => {
            outbound.outbound_type = "trojan".to_string();
            outbound.password = Some(required_string(node, "password")?);
            outbound.network = Some("tcp".to_string());
            outbound.transport = transport;
        }
        ProxyProtocol::Hysteria2 => {
            outbound.outbound_type = "hysteria2".to_string();
            outbound.password = Some(
                optional_string(node, "password")
                    .or_else(|| optional_string(node, "auth"))
                    .ok_or_else(|| TransformError::MissingField {
                        node_name: node.name.clone(),
                        field: "password/auth",
                    })?,
            );
            if let Some(obfs_type) = optional_string(node, "obfs") {
                outbound.obfs = Some(SingboxObfs {
                    obfs_type,
                    password: optional_string(node, "obfs_password"),
                });
            }
            outbound.transport = None;
        }
        ProxyProtocol::Tuic => {
            outbound.outbound_type = "tuic".to_string();
            outbound.uuid = Some(required_string(node, "uuid")?);
            outbound.password = Some(required_string(node, "password")?);
            outbound.congestion_control = optional_string(node, "congestion_control");
            outbound.udp_relay_mode = optional_string(node, "udp_relay_mode");
            outbound.network = Some("tcp".to_string());
            outbound.transport = None;
        }
    }

    Ok(outbound)
}

fn build_singbox_tls(node: &ProxyNode) -> Option<SingboxTls> {
    let server_name = node
        .tls
        .server_name
        .clone()
        .or_else(|| optional_string(node, "sni"));
    let insecure = optional_bool(node, "skip_cert_verify");
    let alpn = optional_string_list(node, "alpn");
    let has_fields =
        server_name.is_some() || insecure.is_some() || alpn.is_some() || node.tls.enabled;
    if !has_fields {
        return None;
    }

    Some(SingboxTls {
        enabled: node.tls.enabled,
        server_name,
        insecure,
        alpn,
    })
}

fn build_singbox_transport(node: &ProxyNode) -> Option<SingboxTransport> {
    match node.transport {
        ProxyTransport::Tcp => None,
        ProxyTransport::Ws => {
            let mut headers = BTreeMap::new();
            if let Some(host) = optional_string(node, "host") {
                headers.insert("Host".to_string(), host);
            }
            Some(SingboxTransport {
                transport_type: "ws".to_string(),
                path: optional_string(node, "path"),
                headers: (!headers.is_empty()).then_some(headers),
                host: None,
                service_name: None,
                max_early_data: optional_u32(node, "max_early_data"),
                early_data_header_name: optional_string(node, "early_data_header_name"),
            })
        }
        ProxyTransport::Grpc => Some(SingboxTransport {
            transport_type: "grpc".to_string(),
            path: None,
            headers: None,
            host: None,
            service_name: optional_string(node, "grpc_service_name")
                .or_else(|| optional_string(node, "service_name")),
            max_early_data: None,
            early_data_header_name: None,
        }),
        ProxyTransport::H2 => Some(SingboxTransport {
            transport_type: "http".to_string(),
            path: optional_string(node, "path"),
            headers: None,
            host: optional_string_list(node, "host"),
            service_name: None,
            max_early_data: None,
            early_data_header_name: None,
        }),
        ProxyTransport::Quic => Some(SingboxTransport {
            transport_type: "quic".to_string(),
            path: None,
            headers: None,
            host: None,
            service_name: None,
            max_early_data: None,
            early_data_header_name: None,
        }),
    }
}

#[derive(Debug, Serialize)]
pub(super) struct SingboxTls {
    pub(super) enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) server_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) insecure: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) alpn: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub(super) struct SingboxTransport {
    #[serde(rename = "type")]
    pub(super) transport_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) host: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) service_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) max_early_data: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) early_data_header_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct SingboxObfs {
    #[serde(rename = "type")]
    pub(super) obfs_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) password: Option<String>,
}
