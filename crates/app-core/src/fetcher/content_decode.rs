use brotli::Decompressor;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use std::io::Read;

pub(super) fn decode_response_body(
    raw: Vec<u8>,
    content_encoding: Option<&str>,
) -> Result<Vec<u8>, String> {
    let Some(content_encoding) = content_encoding
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(raw);
    };

    let mut encodings = content_encoding
        .split(',')
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty() && value != "identity")
        .collect::<Vec<_>>();
    if encodings.is_empty() {
        return Ok(raw);
    }

    let mut payload = raw;
    while let Some(encoding) = encodings.pop() {
        payload = match encoding.as_str() {
            "br" => decode_brotli(&payload)?,
            "gzip" | "x-gzip" => decode_gzip(&payload)?,
            "deflate" => decode_deflate(&payload)?,
            _ => payload,
        };
    }
    Ok(payload)
}

fn decode_brotli(payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = Decompressor::new(payload, 4096);
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|error| format!("br 解压失败：{error}"))?;
    Ok(decoded)
}

fn decode_gzip(payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = GzDecoder::new(payload);
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|error| format!("gzip 解压失败：{error}"))?;
    Ok(decoded)
}

fn decode_deflate(payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut zlib_decoder = ZlibDecoder::new(payload);
    let mut decoded = Vec::new();
    match zlib_decoder.read_to_end(&mut decoded) {
        Ok(_) => Ok(decoded),
        Err(_) => {
            let mut raw_decoder = DeflateDecoder::new(payload);
            let mut fallback = Vec::new();
            raw_decoder
                .read_to_end(&mut fallback)
                .map_err(|error| format!("deflate 解压失败：{error}"))?;
            Ok(fallback)
        }
    }
}
