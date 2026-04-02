use app_common::ProxyNode;
use app_storage::{Database, NodeCacheRepository};
use app_transport::{NetworkProfileFactory, TransportProfile};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use reqwest::{Client as HttpClient, Url};
use tokio::time::sleep;

use crate::utils::now_rfc3339;
use crate::{CoreError, CoreResult, SubscriptionParser, UriListParser};

const MAX_SUBSCRIPTION_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug)]
pub struct StaticFetcher<'a, P: SubscriptionParser = UriListParser> {
    db: &'a Database,
    parser: P,
    client: HttpClient,
    transport_profile: Box<dyn TransportProfile>,
}

impl<'a> StaticFetcher<'a, UriListParser> {
    pub fn new(db: &'a Database) -> CoreResult<Self> {
        Self::new_with_network_profile(db, "standard")
    }

    pub fn new_with_network_profile(db: &'a Database, network_profile: &str) -> CoreResult<Self> {
        Self::with_parser_and_network_profile(db, UriListParser, network_profile)
    }
}

impl<'a, P> StaticFetcher<'a, P>
where
    P: SubscriptionParser,
{
    pub fn with_parser(db: &'a Database, parser: P) -> CoreResult<Self> {
        Self::with_parser_and_network_profile(db, parser, "standard")
    }

    pub fn with_parser_and_network_profile(
        db: &'a Database,
        parser: P,
        network_profile: &str,
    ) -> CoreResult<Self> {
        let transport_profile = NetworkProfileFactory::create(network_profile)?;
        let client = transport_profile.build_client()?;

        Ok(Self {
            db,
            parser,
            client,
            transport_profile,
        })
    }

    pub async fn fetch_and_cache(
        &self,
        source_instance_id: &str,
        subscription_url: &str,
        user_agent: Option<&str>,
    ) -> CoreResult<Vec<ProxyNode>> {
        let subscription_url = subscription_url.trim();
        if subscription_url.is_empty() {
            return Err(CoreError::ConfigInvalid("订阅 URL 不能为空".to_string()));
        }

        let url = Url::parse(subscription_url)
            .map_err(|error| CoreError::ConfigInvalid(format!("订阅 URL 非法：{error}")))?;
        let headers = self.build_request_headers(user_agent)?;
        let redacted_url = redact_url_for_log(&url);
        let redacted_headers = redact_headers_for_log(&headers);
        let started = std::time::Instant::now();
        let profile_name = self.transport_profile.profile_name();

        let mut retry_attempt = 0usize;
        let response = loop {
            if retry_attempt > 0 {
                let backoff = retry_backoff(self.transport_profile.request_delay(), retry_attempt);
                sleep(backoff).await;
            }
            let response = self
                .client
                .get(url.clone())
                .headers(headers.clone())
                .send()
                .await
                .map_err(|error| {
                    let sanitized = sanitize_reqwest_error(&error, &url);
                    eprintln!(
                        "WARN: 订阅请求失败 source_id={} profile={} url={} elapsed_ms={} request_headers={} error={}",
                        source_instance_id,
                        profile_name,
                        redacted_url,
                        started.elapsed().as_millis(),
                        redacted_headers,
                        sanitized
                    );
                    CoreError::SubscriptionFetch(sanitized)
                })?;

            let status = response.status();
            if status.is_success() {
                eprintln!(
                    "INFO: 订阅请求成功 source_id={} profile={} url={} status={} elapsed_ms={} retries={} request_headers={}",
                    source_instance_id,
                    profile_name,
                    redacted_url,
                    status.as_u16(),
                    started.elapsed().as_millis(),
                    retry_attempt,
                    redacted_headers
                );
                break response;
            }
            if retry_attempt < self.transport_profile.max_retries()
                && self.transport_profile.is_retryable_status(status)
            {
                eprintln!(
                    "WARN: 订阅请求触发重试 source_id={} profile={} url={} status={} elapsed_ms={} retry={}/{} request_headers={}",
                    source_instance_id,
                    profile_name,
                    redacted_url,
                    status.as_u16(),
                    started.elapsed().as_millis(),
                    retry_attempt + 1,
                    self.transport_profile.max_retries(),
                    redacted_headers
                );
                retry_attempt += 1;
                continue;
            }
            eprintln!(
                "WARN: 订阅请求状态异常 source_id={} profile={} url={} status={} elapsed_ms={} retries={} request_headers={}",
                source_instance_id,
                profile_name,
                redacted_url,
                status.as_u16(),
                started.elapsed().as_millis(),
                retry_attempt,
                redacted_headers
            );
            return Err(CoreError::SubscriptionFetch(format!(
                "上游响应状态码异常：{}",
                status.as_u16()
            )));
        };

        validate_content_type(response.headers())?;
        if let Some(content_length) = response.content_length() {
            if content_length > MAX_SUBSCRIPTION_BYTES as u64 {
                return Err(CoreError::SubscriptionFetch(format!(
                    "上游响应体过大：{} bytes（限制 {} bytes）",
                    content_length, MAX_SUBSCRIPTION_BYTES
                )));
            }
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|error| CoreError::SubscriptionFetch(error.to_string()))?;
        if bytes.len() > MAX_SUBSCRIPTION_BYTES {
            return Err(CoreError::SubscriptionFetch(format!(
                "上游响应体过大：{} bytes（限制 {} bytes）",
                bytes.len(),
                MAX_SUBSCRIPTION_BYTES
            )));
        }

        let payload = std::str::from_utf8(&bytes).map_err(|error| {
            CoreError::SubscriptionParse(format!("订阅内容不是 UTF-8：{error}"))
        })?;
        let nodes = self.parser.parse(source_instance_id, payload)?;
        self.cache_nodes(source_instance_id, &nodes)?;

        Ok(nodes)
    }

    pub fn parse_and_cache_content(
        &self,
        source_instance_id: &str,
        payload: &str,
    ) -> CoreResult<Vec<ProxyNode>> {
        let nodes = self.parser.parse(source_instance_id, payload)?;
        self.cache_nodes(source_instance_id, &nodes)?;
        Ok(nodes)
    }

    fn cache_nodes(&self, source_instance_id: &str, nodes: &[ProxyNode]) -> CoreResult<()> {
        let now = now_rfc3339()?;
        let cache_repository = NodeCacheRepository::new(self.db);
        cache_repository.upsert_nodes(source_instance_id, nodes, &now, None)?;
        Ok(())
    }

    fn build_request_headers(&self, user_agent: Option<&str>) -> CoreResult<HeaderMap> {
        let mut headers = HeaderMap::new();
        for (name, value) in self.transport_profile.default_headers() {
            let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                CoreError::ConfigInvalid(format!("传输层默认 Header 名非法（{name}）：{error}"))
            })?;
            let header_value = HeaderValue::from_str(value).map_err(|error| {
                CoreError::ConfigInvalid(format!("传输层默认 Header 值非法（{name}）：{error}"))
            })?;
            headers.insert(header_name, header_value);
        }

        if let Some(user_agent) = user_agent {
            let user_agent = user_agent.trim();
            if !user_agent.is_empty() {
                headers.insert(
                    USER_AGENT,
                    user_agent.parse().map_err(|error| {
                        CoreError::ConfigInvalid(format!("user_agent 非法：{error}"))
                    })?,
                );
            }
        }

        Ok(headers)
    }
}

pub(crate) fn retry_backoff(
    base_delay: std::time::Duration,
    retry_attempt: usize,
) -> std::time::Duration {
    let base_delay = if base_delay.is_zero() {
        std::time::Duration::from_millis(100)
    } else {
        base_delay
    };
    let shift = retry_attempt.saturating_sub(1).min(16);
    base_delay.saturating_mul(1_u32 << shift)
}

pub(crate) fn redact_url_for_log(url: &Url) -> String {
    let mut sanitized = url.clone();
    if url.query().is_none() {
        return sanitized.to_string();
    }
    sanitized.set_query(None);
    {
        let mut query = sanitized.query_pairs_mut();
        for (key, value) in url.query_pairs() {
            if is_sensitive_query_key(key.as_ref()) {
                query.append_pair(key.as_ref(), "***");
            } else {
                query.append_pair(key.as_ref(), value.as_ref());
            }
        }
    }
    sanitized.to_string()
}

pub(crate) fn sanitize_reqwest_error(error: &reqwest::Error, url: &Url) -> String {
    let message = error.to_string();
    let redacted_url = redact_url_for_log(url);
    message.replace(url.as_str(), &redacted_url)
}

pub(crate) fn redact_headers_for_log(headers: &HeaderMap) -> String {
    if headers.is_empty() {
        return "[]".to_string();
    }
    let mut pairs = headers
        .iter()
        .map(|(name, value)| {
            let key = name.as_str().to_ascii_lowercase();
            let value = if is_sensitive_header(&key) {
                "***".to_string()
            } else {
                value.to_str().unwrap_or("<non-utf8>").to_string()
            };
            format!("{key}={value}")
        })
        .collect::<Vec<_>>();
    pairs.sort_unstable();
    format!("[{}]", pairs.join(", "))
}

pub(crate) fn is_sensitive_query_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "token"
            | "access_token"
            | "password"
            | "passwd"
            | "secret"
            | "auth"
            | "authorization"
            | "api_key"
            | "apikey"
            | "cookie"
    )
}

pub(crate) fn is_sensitive_header(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "x-api-key"
            | "x-auth-token"
            | "x-access-token"
    )
}

pub(crate) fn validate_content_type(headers: &HeaderMap) -> CoreResult<()> {
    let Some(content_type) = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return Ok(());
    };

    let normalized = content_type.to_ascii_lowercase();
    let allowed = normalized.starts_with("text/")
        || normalized.starts_with("application/json")
        || normalized.starts_with("application/octet-stream");

    if allowed {
        Ok(())
    } else {
        Err(CoreError::SubscriptionFetch(format!(
            "上游 Content-Type 不受支持：{content_type}"
        )))
    }
}
