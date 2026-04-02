use std::time::Duration;

use reqwest::Client;
use reqwest::StatusCode;
use reqwest::redirect::Policy;

use crate::error::TransportResult;

const STANDARD_TIMEOUT_SEC: u64 = 30;
const STANDARD_MAX_REDIRECTS: usize = 10;
const STANDARD_DEFAULT_USER_AGENT: &str = "SubForge/0.1.0 (standard)";
const BROWSER_CHROME_TIMEOUT_SEC: u64 = 30;
const BROWSER_CHROME_MAX_REDIRECTS: usize = 10;
const BROWSER_CHROME_REQUEST_DELAY_MS: u64 = 500;
const BROWSER_CHROME_MAX_RETRIES: usize = 3;
const BROWSER_CHROME_DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const EMPTY_HEADER_TEMPLATE: [(&str, &str); 0] = [];
const BROWSER_CHROME_HEADER_TEMPLATE: [(&str, &str); 11] = [
    (
        "sec-ch-ua",
        "\"Chromium\";v=\"124\", \"Google Chrome\";v=\"124\", \"Not-A.Brand\";v=\"99\"",
    ),
    ("sec-ch-ua-mobile", "?0"),
    ("sec-ch-ua-platform", "\"Windows\""),
    ("upgrade-insecure-requests", "1"),
    (
        "accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
    ),
    ("sec-fetch-site", "none"),
    ("sec-fetch-mode", "navigate"),
    ("sec-fetch-user", "?1"),
    ("sec-fetch-dest", "document"),
    ("accept-encoding", "gzip, deflate, br"),
    ("accept-language", "zh-CN,zh;q=0.9,en;q=0.8"),
];

pub trait TransportProfile: Send + Sync + std::fmt::Debug {
    fn profile_name(&self) -> &'static str;
    fn timeout(&self) -> Duration;
    fn max_redirects(&self) -> usize;
    fn default_user_agent(&self) -> &'static str;
    fn uses_cookie_store(&self) -> bool {
        false
    }
    fn build_client(&self) -> TransportResult<Client> {
        self.build_client_with_limits(self.timeout(), self.max_redirects())
    }
    fn build_client_with_limits(
        &self,
        timeout: Duration,
        max_redirects: usize,
    ) -> TransportResult<Client> {
        build_client_with_settings(
            timeout,
            max_redirects,
            self.default_user_agent(),
            self.uses_cookie_store(),
        )
    }
    fn request_delay(&self) -> Duration;
    fn default_headers(&self) -> &[(&'static str, &'static str)] {
        &EMPTY_HEADER_TEMPLATE
    }
    fn max_retries(&self) -> usize {
        0
    }
    fn is_retryable_status(&self, status_code: StatusCode) -> bool {
        let _ = status_code;
        false
    }
}

#[derive(Debug, Clone)]
pub struct StandardProfile {
    timeout: Duration,
    max_redirects: usize,
    request_delay: Duration,
    default_user_agent: &'static str,
}

impl Default for StandardProfile {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(STANDARD_TIMEOUT_SEC),
            max_redirects: STANDARD_MAX_REDIRECTS,
            request_delay: Duration::from_millis(0),
            default_user_agent: STANDARD_DEFAULT_USER_AGENT,
        }
    }
}

impl TransportProfile for StandardProfile {
    fn profile_name(&self) -> &'static str {
        "standard"
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    fn max_redirects(&self) -> usize {
        self.max_redirects
    }

    fn default_user_agent(&self) -> &'static str {
        self.default_user_agent
    }

    fn request_delay(&self) -> Duration {
        self.request_delay
    }
}

#[derive(Debug, Clone)]
pub struct BrowserChromeProfile {
    timeout: Duration,
    max_redirects: usize,
    request_delay: Duration,
    max_retries: usize,
    default_user_agent: &'static str,
}

impl Default for BrowserChromeProfile {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(BROWSER_CHROME_TIMEOUT_SEC),
            max_redirects: BROWSER_CHROME_MAX_REDIRECTS,
            request_delay: Duration::from_millis(BROWSER_CHROME_REQUEST_DELAY_MS),
            max_retries: BROWSER_CHROME_MAX_RETRIES,
            default_user_agent: BROWSER_CHROME_DEFAULT_USER_AGENT,
        }
    }
}

impl TransportProfile for BrowserChromeProfile {
    fn profile_name(&self) -> &'static str {
        "browser_chrome"
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    fn max_redirects(&self) -> usize {
        self.max_redirects
    }

    fn default_user_agent(&self) -> &'static str {
        self.default_user_agent
    }

    fn uses_cookie_store(&self) -> bool {
        true
    }

    fn request_delay(&self) -> Duration {
        self.request_delay
    }

    fn default_headers(&self) -> &[(&'static str, &'static str)] {
        &BROWSER_CHROME_HEADER_TEMPLATE
    }

    fn max_retries(&self) -> usize {
        self.max_retries
    }

    fn is_retryable_status(&self, status_code: StatusCode) -> bool {
        matches!(
            status_code,
            StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
        )
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NetworkProfileFactory;

impl NetworkProfileFactory {
    pub fn create(profile: &str) -> crate::TransportResult<Box<dyn TransportProfile>> {
        let profile = profile.trim();
        match profile {
            "" | "standard" => Ok(Box::new(StandardProfile::default())),
            "browser_chrome" => Ok(Box::new(BrowserChromeProfile::default())),
            _ => Err(crate::TransportError::UnsupportedProfile(
                profile.to_string(),
            )),
        }
    }
}

fn build_client_with_settings(
    timeout: Duration,
    max_redirects: usize,
    user_agent: &'static str,
    use_cookie_store: bool,
) -> TransportResult<Client> {
    let mut builder = Client::builder()
        .redirect(Policy::limited(max_redirects))
        .timeout(timeout)
        .user_agent(user_agent)
        .danger_accept_invalid_certs(false);
    if use_cookie_store {
        builder = builder.cookie_store(true);
    }
    let client = builder.build()?;
    Ok(client)
}
