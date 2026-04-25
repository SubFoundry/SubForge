use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::header::{CONTENT_TYPE, COOKIE, SET_COOKIE};
use axum::routing::get;
use reqwest::StatusCode;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::{
    BrowserChromeProfile, BrowserFirefoxProfile, NetworkProfileFactory, StandardProfile,
    TransportError, TransportProfile, WebviewAssistedProfile,
};

#[test]
fn standard_profile_builds_https_request() {
    let profile = StandardProfile::default();
    let client = profile.build_client().expect("标准档位构建客户端失败");
    let request = client
        .get("https://example.com")
        .build()
        .expect("构建 HTTPS 请求失败");
    assert_eq!(request.url().scheme(), "https");
}

#[test]
fn standard_profile_uses_clash_meta_user_agent() {
    let profile = StandardProfile::default();
    assert_eq!(profile.default_user_agent(), "clash.meta");
}

#[test]
fn browser_chrome_profile_exposes_chrome_headers_and_retry_policy() {
    let profile = BrowserChromeProfile::default();
    let headers = profile.default_headers();
    assert_eq!(headers.first().map(|(name, _)| *name), Some("sec-ch-ua"));
    assert_eq!(
        headers.get(3).map(|(name, _)| *name),
        Some("upgrade-insecure-requests")
    );
    assert_eq!(
        headers.last().map(|(name, _)| *name),
        Some("accept-language")
    );
    assert_eq!(profile.request_delay(), Duration::from_millis(500));
    assert_eq!(profile.max_retries(), 3);
    assert!(profile.is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
    assert!(profile.is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
}

#[tokio::test]
async fn browser_chrome_profile_persists_cookies() {
    #[derive(Clone)]
    struct CookieState {
        visits: Arc<AtomicUsize>,
    }

    let visits = Arc::new(AtomicUsize::new(0));
    let state = CookieState {
        visits: visits.clone(),
    };
    let app = Router::new()
        .route(
            "/cookie",
            get(
                |State(state): State<CookieState>, headers: HeaderMap| async move {
                    let current = state.visits.fetch_add(1, Ordering::SeqCst);
                    let has_cookie = headers
                        .get(COOKIE)
                        .and_then(|value| value.to_str().ok())
                        .map(|value| value.contains("subforge_sid=abc123"))
                        .unwrap_or(false);
                    if current == 0 {
                        (
                            StatusCode::OK,
                            [
                                (SET_COOKIE, "subforge_sid=abc123; Path=/; HttpOnly"),
                                (CONTENT_TYPE, "text/plain"),
                            ],
                            "issued".to_string(),
                        )
                    } else if has_cookie {
                        (
                            StatusCode::OK,
                            [
                                (SET_COOKIE, "subforge_sid=abc123; Path=/; HttpOnly"),
                                (CONTENT_TYPE, "text/plain"),
                            ],
                            "reused".to_string(),
                        )
                    } else {
                        (
                            StatusCode::BAD_REQUEST,
                            [
                                (SET_COOKIE, "subforge_sid=abc123; Path=/; HttpOnly"),
                                (CONTENT_TYPE, "text/plain"),
                            ],
                            "cookie missing".to_string(),
                        )
                    }
                },
            ),
        )
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动测试 HTTP 服务失败");
    let address: SocketAddr = listener.local_addr().expect("读取监听地址失败");
    let base_url = format!("http://{}", address);
    let server: JoinHandle<()> = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("测试 HTTP 服务运行失败");
    });

    let profile = BrowserChromeProfile::default();
    let client = profile
        .build_client()
        .expect("browser_chrome 客户端构建失败");
    let first = client
        .get(format!("{base_url}/cookie"))
        .send()
        .await
        .expect("第一次请求失败");
    assert_eq!(first.status(), StatusCode::OK);

    let second = client
        .get(format!("{base_url}/cookie"))
        .send()
        .await
        .expect("第二次请求失败");
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(visits.load(Ordering::SeqCst), 2);

    server.abort();
}

#[test]
fn factory_resolves_browser_chrome_profile() {
    let profile =
        NetworkProfileFactory::create("browser_chrome").expect("browser_chrome 档位应可创建");
    assert_eq!(profile.request_delay(), Duration::from_millis(500));
    assert_eq!(profile.max_retries(), 3);
}

#[test]
fn browser_firefox_profile_uses_cookie_store_and_retry_policy() {
    let profile = BrowserFirefoxProfile::default();
    assert!(profile.uses_cookie_store());
    assert_eq!(profile.request_delay(), Duration::from_millis(500));
    assert_eq!(profile.max_retries(), 3);
    assert!(profile.is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
    assert!(profile.is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
}

#[test]
fn webview_assisted_profile_uses_cookie_store() {
    let profile = WebviewAssistedProfile::default();
    assert!(profile.uses_cookie_store());
    assert_eq!(profile.request_delay(), Duration::from_millis(0));
    assert_eq!(profile.max_retries(), 0);
}

#[test]
fn factory_resolves_browser_firefox_profile() {
    let profile =
        NetworkProfileFactory::create("browser_firefox").expect("browser_firefox 档位应可创建");
    assert_eq!(profile.request_delay(), Duration::from_millis(500));
    assert_eq!(profile.max_retries(), 3);
}

#[test]
fn factory_resolves_webview_assisted_profile() {
    let profile =
        NetworkProfileFactory::create("webview_assisted").expect("webview_assisted 档位应可创建");
    assert_eq!(profile.request_delay(), Duration::from_millis(0));
    assert_eq!(profile.max_retries(), 0);
}

#[test]
fn factory_returns_error_for_unknown_profile() {
    match NetworkProfileFactory::create("unknown-profile") {
        Ok(_) => panic!("未知档位必须返回错误"),
        Err(error) => {
            assert!(matches!(error, TransportError::UnsupportedProfile(_)));
        }
    }
}
