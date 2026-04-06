use super::*;
use std::io::Write;

use brotli::CompressorWriter;
use flate2::Compression;
use flate2::write::GzEncoder;

#[tokio::test]
async fn static_fetcher_fetches_parses_and_persists_node_cache() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let source_repository = SourceRepository::new(&db);
    source_repository
        .insert(&sample_source("source-fetch-1", "subforge.builtin.static"))
        .expect("写入来源实例失败");

    let (url, server_task) = start_fixture_server(
        "/sub",
        BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
        "text/plain; charset=utf-8",
    )
    .await;

    let fetcher = StaticFetcher::new(&db).expect("初始化 StaticFetcher 失败");
    let nodes = fetcher
        .fetch_and_cache(
            "source-fetch-1",
            &format!("{url}/sub"),
            Some("SubForge-Test/0.1"),
        )
        .await
        .expect("拉取并缓存应成功");
    assert_eq!(nodes.len(), 3);

    let cache_repository = NodeCacheRepository::new(&db);
    let cache = cache_repository
        .get_by_source("source-fetch-1")
        .expect("读取缓存失败")
        .expect("缓存应存在");
    assert_eq!(cache.nodes.len(), 3);

    server_task.abort();
}

#[tokio::test]
async fn static_fetcher_rejects_unsupported_content_type() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let source_repository = SourceRepository::new(&db);
    source_repository
        .insert(&sample_source("source-fetch-2", "subforge.builtin.static"))
        .expect("写入来源实例失败");

    let (url, server_task) =
        start_fixture_server("/sub", "plain text".to_string(), "image/png").await;

    let fetcher = StaticFetcher::new(&db).expect("初始化 StaticFetcher 失败");
    let error = fetcher
        .fetch_and_cache("source-fetch-2", &format!("{url}/sub"), None)
        .await
        .expect_err("非法 Content-Type 应被拒绝");
    assert!(matches!(error, CoreError::SubscriptionFetch(_)));

    server_task.abort();
}

#[tokio::test]
async fn static_fetcher_browser_chrome_retries_on_429_then_succeeds() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let source_repository = SourceRepository::new(&db);
    source_repository
        .insert(&sample_source("source-fetch-3", "subforge.builtin.static"))
        .expect("写入来源实例失败");

    let (url, request_count, server_task) = start_retry_fixture_server(
        "/sub",
        vec![429, 429],
        BASE64_SUBSCRIPTION_FIXTURE.trim().to_string(),
        "text/plain; charset=utf-8",
    )
    .await;

    let fetcher = StaticFetcher::new_with_network_profile(&db, "browser_chrome")
        .expect("初始化 browser_chrome StaticFetcher 失败");
    let started = Instant::now();
    let nodes = fetcher
        .fetch_and_cache("source-fetch-3", &format!("{url}/sub"), None)
        .await
        .expect("429 重试后应成功");

    assert_eq!(nodes.len(), 3);
    assert_eq!(request_count.load(Ordering::SeqCst), 3);
    assert!(
        started.elapsed() >= Duration::from_millis(1400),
        "退避总时长应至少接近 500ms + 1000ms"
    );

    server_task.abort();
}

#[test]
fn static_fetcher_rejects_unknown_network_profile() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let error = StaticFetcher::new_with_network_profile(&db, "unknown-profile")
        .expect_err("未知网络档位必须返回错误");
    assert_eq!(error.code(), "E_CONFIG_INVALID");
}

#[tokio::test]
async fn static_fetcher_decodes_gzip_subscription_payload() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let source_repository = SourceRepository::new(&db);
    source_repository
        .insert(&sample_source(
            "source-fetch-gzip",
            "subforge.builtin.static",
        ))
        .expect("写入来源实例失败");

    let compressed = gzip_encode(BASE64_SUBSCRIPTION_FIXTURE.trim().as_bytes());
    let (url, server_task) = start_encoded_fixture_server("/sub", compressed, "gzip").await;

    let fetcher = StaticFetcher::new_with_network_profile(&db, "browser_chrome")
        .expect("初始化 browser_chrome StaticFetcher 失败");
    let nodes = fetcher
        .fetch_and_cache("source-fetch-gzip", &format!("{url}/sub"), None)
        .await
        .expect("gzip 内容应可正常解析");
    assert_eq!(nodes.len(), 3);

    server_task.abort();
}

#[tokio::test]
async fn static_fetcher_decodes_brotli_subscription_payload() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let source_repository = SourceRepository::new(&db);
    source_repository
        .insert(&sample_source("source-fetch-br", "subforge.builtin.static"))
        .expect("写入来源实例失败");

    let compressed = brotli_encode(BASE64_SUBSCRIPTION_FIXTURE.trim().as_bytes());
    let (url, server_task) = start_encoded_fixture_server("/sub", compressed, "br").await;

    let fetcher = StaticFetcher::new_with_network_profile(&db, "browser_chrome")
        .expect("初始化 browser_chrome StaticFetcher 失败");
    let nodes = fetcher
        .fetch_and_cache("source-fetch-br", &format!("{url}/sub"), None)
        .await
        .expect("brotli 内容应可正常解析");
    assert_eq!(nodes.len(), 3);

    server_task.abort();
}

#[test]
fn static_fetcher_extracts_and_persists_clash_routing_template() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let source_repository = SourceRepository::new(&db);
    source_repository
        .insert(&sample_source(
            "source-fetch-template",
            "subforge.builtin.static",
        ))
        .expect("写入来源实例失败");

    let fetcher = StaticFetcher::new(&db).expect("初始化 StaticFetcher 失败");
    let payload = r#"
proxies:
  - name: old-node-a
    type: ss
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - DIRECT
      - old-node-a
  - name: Auto
    type: url-test
    proxies:
      - old-node-a
    url: http://www.gstatic.com/generate_204
    interval: 300
    tolerance: 50
rules:
  - MATCH,Proxy
"#;
    let nodes = fetcher
        .parse_and_cache_content("source-fetch-template", payload)
        .expect("缓存模板内容不应失败");
    assert!(nodes.is_empty(), "Clash YAML 不应被 URI 解析器误解析为节点");

    let repository = SettingsRepository::new(&db);
    let setting = repository
        .get("source.source-fetch-template.clash_routing_template")
        .expect("读取模板设置失败")
        .expect("应保存 Clash 分流模板");
    let template: app_common::ClashRoutingTemplate =
        serde_json::from_str(&setting.value).expect("模板 JSON 反序列化失败");
    assert_eq!(template.groups.len(), 2);
    assert_eq!(template.groups[0].name, "Proxy");
    assert_eq!(template.rules, vec!["MATCH,Proxy".to_string()]);
}

#[test]
fn static_fetcher_extracts_singbox_template_and_converts_to_clash_semantics() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let source_repository = SourceRepository::new(&db);
    source_repository
        .insert(&sample_source(
            "source-fetch-template-singbox",
            "subforge.builtin.static",
        ))
        .expect("写入来源实例失败");

    let fetcher = StaticFetcher::new(&db).expect("初始化 StaticFetcher 失败");
    let payload = r#"
{
  "outbounds": [
    {
      "type": "selector",
      "tag": "Proxy",
      "outbounds": ["Auto", "DIRECT"]
    },
    {
      "type": "urltest",
      "tag": "Auto",
      "outbounds": ["old-node-a"],
      "url": "http://www.gstatic.com/generate_204",
      "interval": 300
    }
  ],
  "route": {
    "rules": [
      {
        "domain_suffix": ["example.com"],
        "outbound": "Proxy"
      }
    ],
    "final": "Proxy"
  }
}
"#;
    let nodes = fetcher
        .parse_and_cache_content("source-fetch-template-singbox", payload)
        .expect("缓存 sing-box 模板内容不应失败");
    assert!(
        nodes.is_empty(),
        "sing-box 模板不应被 URI 解析器误解析为节点"
    );

    let repository = SettingsRepository::new(&db);
    let setting = repository
        .get("source.source-fetch-template-singbox.clash_routing_template")
        .expect("读取模板设置失败")
        .expect("应保存转换后的 Clash 分流模板");
    let template: app_common::ClashRoutingTemplate =
        serde_json::from_str(&setting.value).expect("模板 JSON 反序列化失败");
    assert_eq!(template.groups.len(), 2);
    assert_eq!(template.groups[0].name, "Proxy");
    assert_eq!(template.groups[1].group_type, "url-test");
    assert_eq!(
        template.rules,
        vec![
            "DOMAIN-SUFFIX,example.com,Proxy".to_string(),
            "MATCH,Proxy".to_string()
        ]
    );
}

#[test]
fn static_fetcher_clears_template_when_payload_is_not_clash_yaml() {
    let db = Database::open_in_memory().expect("内存数据库初始化失败");
    let source_repository = SourceRepository::new(&db);
    source_repository
        .insert(&sample_source(
            "source-fetch-template-clear",
            "subforge.builtin.static",
        ))
        .expect("写入来源实例失败");

    let fetcher = StaticFetcher::new(&db).expect("初始化 StaticFetcher 失败");
    fetcher
        .parse_and_cache_content(
            "source-fetch-template-clear",
            r#"
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - old-node
"#,
        )
        .expect("首次写入模板应成功");
    fetcher
        .parse_and_cache_content(
            "source-fetch-template-clear",
            "ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ=@example.com:443#node-a",
        )
        .expect("写入 URI 内容应成功");

    let repository = SettingsRepository::new(&db);
    assert!(
        repository
            .get("source.source-fetch-template-clear.clash_routing_template")
            .expect("读取模板设置失败")
            .is_none(),
        "非 Clash YAML 内容应清理模板缓存"
    );
}

async fn start_encoded_fixture_server(
    route_path: &'static str,
    body: Vec<u8>,
    content_encoding: &'static str,
) -> (String, JoinHandle<()>) {
    let app = Router::new().route(
        route_path,
        get(move || {
            let body = body.clone();
            async move {
                (
                    [
                        (
                            axum::http::header::CONTENT_TYPE,
                            "text/plain; charset=utf-8",
                        ),
                        (axum::http::header::CONTENT_ENCODING, content_encoding),
                    ],
                    body,
                )
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("启动测试 HTTP 服务失败");
    let address: SocketAddr = listener.local_addr().expect("读取监听地址失败");
    let base_url = format!("http://{}", address);

    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("测试 HTTP 服务运行失败");
    });

    (base_url, task)
}

fn gzip_encode(payload: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(payload).expect("写入 gzip 压缩流失败");
    encoder.finish().expect("完成 gzip 压缩失败")
}

fn brotli_encode(payload: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    {
        let mut encoder = CompressorWriter::new(&mut output, 4096, 5, 22);
        encoder.write_all(payload).expect("写入 br 压缩流失败");
    }
    output
}
