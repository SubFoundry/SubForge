use std::net::{IpAddr, ToSocketAddrs};

use mlua::Error as LuaError;
use reqwest::Url;

pub(super) fn ensure_allowed_target(url: &Url) -> Result<(), LuaError> {
    match url.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(LuaError::runtime(format!(
                "http.request 仅支持 http/https，当前为：{scheme}"
            )));
        }
    }

    let host = url
        .host_str()
        .ok_or_else(|| LuaError::runtime("http.request 缺少 host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| LuaError::runtime("http.request 端口无效"))?;
    let addresses = resolve_host_ips(host, port)?;
    if addresses.is_empty() {
        return Err(LuaError::runtime("http.request 无法解析目标地址"));
    }

    for ip in addresses {
        if is_forbidden_ip(ip) {
            return Err(LuaError::runtime(format!(
                "http.request 目标地址不允许（内网/保留地址）：{}",
                ip
            )));
        }
    }

    Ok(())
}

fn resolve_host_ips(host: &str, port: u16) -> Result<Vec<IpAddr>, LuaError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }

    let socket_address = format!("{host}:{port}");
    socket_address
        .to_socket_addrs()
        .map(|iter| iter.map(|addr| addr.ip()).collect::<Vec<_>>())
        .map_err(|error| LuaError::runtime(format!("http.request DNS 解析失败：{error}")))
}

fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            octets[0] == 127
                || octets[0] == 0
                || octets[0] == 10
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
                || (octets[0] == 169 && octets[1] == 254)
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return true;
            }
            let first_segment = v6.segments()[0];
            (first_segment & 0xfe00) == 0xfc00 || (first_segment & 0xffc0) == 0xfe80
        }
    }
}
