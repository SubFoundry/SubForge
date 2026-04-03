use reqwest::header::HeaderMap;

use super::super::{CookieEntry, CookieStore};
use mlua::Error as LuaError;

pub(super) fn compose_cookie_header(cookie_store: CookieStore) -> Result<String, LuaError> {
    let jar = cookie_store
        .lock()
        .map_err(|_| LuaError::runtime("cookie 会话锁已损坏"))?;
    if jar.is_empty() {
        return Ok(String::new());
    }

    let mut pairs = jar
        .iter()
        .map(|(name, entry)| {
            let _attrs = entry.attrs.len();
            format!("{name}={}", entry.value)
        })
        .collect::<Vec<_>>();
    pairs.sort();
    Ok(pairs.join("; "))
}

pub(super) fn apply_response_cookies(
    headers: &HeaderMap,
    cookie_store: CookieStore,
) -> Result<(), LuaError> {
    let mut jar = cookie_store
        .lock()
        .map_err(|_| LuaError::runtime("cookie 会话锁已损坏"))?;
    for value in &headers.get_all("set-cookie") {
        let raw = match value.to_str() {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        if let Some((name, cookie)) = parse_set_cookie_line(raw) {
            jar.insert(name, cookie);
        }
    }
    Ok(())
}

fn parse_set_cookie_line(raw: &str) -> Option<(String, CookieEntry)> {
    let mut segments = raw.split(';');
    let name_value = segments.next()?.trim();
    let (name, value) = name_value.split_once('=')?;
    if name.trim().is_empty() {
        return None;
    }

    let mut attrs = std::collections::BTreeMap::new();
    for segment in segments {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if let Some((attr_name, attr_value)) = segment.split_once('=') {
            attrs.insert(attr_name.trim().to_string(), attr_value.trim().to_string());
        } else {
            attrs.insert(segment.to_string(), "true".to_string());
        }
    }

    Some((
        name.trim().to_string(),
        CookieEntry {
            value: value.trim().to_string(),
            attrs,
        },
    ))
}
