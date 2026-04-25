use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use app_secrets::{MemorySecretStore, SecretStore};
use serde_json::json;

use super::{LuaSandbox, LuaSandboxConfig};
use crate::PluginRuntimeError;

#[test]
fn executes_basic_arithmetic_script() {
    let script_path = write_temp_script(
        "basic-exec",
        r#"
            function run(a, b)
                return { sum = a + b, product = a * b }
            end
        "#,
    );
    let sandbox = LuaSandbox::new().expect("沙箱初始化应成功");
    let result = sandbox
        .exec_file(&script_path, "run", &[json!(2), json!(3)])
        .expect("脚本应执行成功");

    assert_eq!(result["sum"], json!(5));
    assert_eq!(result["product"], json!(6));
    cleanup_script(&script_path);
}

#[test]
fn disallows_dangerous_lua_capabilities() {
    let script_path = write_temp_script(
        "disabled-capabilities",
        r#"
            function run()
                return {
                    os_execute = pcall(function() return os.execute("echo 1") end),
                    io_open = pcall(function() return io.open("test.txt", "r") end),
                    require_mod = pcall(function() return require("x") end),
                    debug_info = pcall(function() return debug.getinfo(1) end),
                    rawget_call = pcall(function() return rawget({},"k") end)
                }
            end
        "#,
    );
    let sandbox = LuaSandbox::new().expect("沙箱初始化应成功");
    let result = sandbox
        .exec_file(&script_path, "run", &[])
        .expect("脚本应可执行并返回结果");

    assert_eq!(result["os_execute"], json!(false));
    assert_eq!(result["io_open"], json!(false));
    assert_eq!(result["require_mod"], json!(false));
    assert_eq!(result["debug_info"], json!(false));
    assert_eq!(result["rawget_call"], json!(false));
    cleanup_script(&script_path);
}

#[test]
fn returns_script_limit_when_memory_limit_exceeded() {
    let script_path = write_temp_script(
        "memory-limit",
        r#"
            function run()
                local t = {}
                for i = 1, 200000 do
                    t[i] = i
                end
                return #t
            end
        "#,
    );
    let config = LuaSandboxConfig::default()
        .with_memory_limit_bytes(128 * 1024)
        .with_timeout(Duration::from_secs(2))
        .with_instruction_limit(1_000_000_000, 1000);
    let sandbox = LuaSandbox::new_with_config(config).expect("沙箱初始化应成功");
    let error = sandbox
        .exec_file(&script_path, "run", &[])
        .expect_err("应触发内存限制");

    assert!(matches!(error, PluginRuntimeError::ScriptLimit(_)));
    assert_eq!(error.code(), "E_SCRIPT_LIMIT");
    cleanup_script(&script_path);
}

#[test]
fn returns_script_timeout_on_infinite_loop() {
    let script_path = write_temp_script(
        "timeout-limit",
        r#"
            function run()
                while true do
                end
            end
        "#,
    );
    let config = LuaSandboxConfig::default()
        .with_timeout(Duration::from_millis(80))
        .with_instruction_limit(u64::MAX / 2, 1000);
    let sandbox = LuaSandbox::new_with_config(config).expect("沙箱初始化应成功");
    let error = sandbox
        .exec_file(&script_path, "run", &[])
        .expect_err("应触发超时限制");

    assert!(matches!(error, PluginRuntimeError::ScriptTimeout(_)));
    assert_eq!(error.code(), "E_SCRIPT_TIMEOUT");
    cleanup_script(&script_path);
}

#[test]
fn returns_script_limit_on_instruction_budget_exceeded() {
    let script_path = write_temp_script(
        "instruction-limit",
        r#"
            function run()
                local sum = 0
                for i = 1, 10000000 do
                    sum = sum + i
                end
                return sum
            end
        "#,
    );
    let config = LuaSandboxConfig::default()
        .with_timeout(Duration::from_secs(3))
        .with_instruction_limit(10_000, 1000);
    let sandbox = LuaSandbox::new_with_config(config).expect("沙箱初始化应成功");
    let error = sandbox
        .exec_file(&script_path, "run", &[])
        .expect_err("应触发指令预算限制");

    assert!(matches!(error, PluginRuntimeError::ScriptLimit(_)));
    assert_eq!(error.code(), "E_SCRIPT_LIMIT");
    cleanup_script(&script_path);
}

#[test]
fn exposes_json_base64_time_and_log_apis() {
    let script_path = write_temp_script(
        "runtime-apis",
        r#"
            function run()
                local parsed = json.parse("{\"name\":\"subforge\",\"count\":2}")
                local encoded = base64.encode("hello")
                local decoded = base64.decode(encoded)
                local now = time.now()
                log.info("runtime api smoke test")
                return {
                    parsed_name = parsed.name,
                    parsed_count = parsed.count,
                    encoded = encoded,
                    decoded = decoded,
                    now = now
                }
            end
        "#,
    );
    let sandbox = LuaSandbox::new().expect("沙箱初始化应成功");
    let result = sandbox
        .exec_file(&script_path, "run", &[])
        .expect("运行时 API 应可调用");

    assert_eq!(result["parsed_name"], json!("subforge"));
    assert_eq!(result["parsed_count"], json!(2));
    assert_eq!(result["encoded"], json!("aGVsbG8="));
    assert_eq!(result["decoded"], json!("hello"));
    let now = result["now"].as_str().expect("time.now 应返回字符串");
    assert!(
        now.contains('T') && now.ends_with('Z'),
        "time.now 应返回 UTC RFC3339 时间字符串"
    );
    cleanup_script(&script_path);
}

#[test]
fn denies_unlisted_runtime_capability_apis() {
    let script_path = write_temp_script(
        "runtime-capability-deny",
        r#"
            function run()
                return {
                    has_json = pcall(function() return json.parse("{}") end),
                    has_http = pcall(function() return http.request({ url = "http://127.0.0.1:18118/health" }) end),
                    has_secret = pcall(function() return secret.get("password") end),
                    has_log = pcall(function() return log.info("test") end),
                    has_time = pcall(function() return time.now() end)
                }
            end
        "#,
    );
    let config = LuaSandboxConfig::default().with_capabilities(["base64"]);
    let sandbox = LuaSandbox::new_with_config(config).expect("沙箱初始化应成功");
    let result = sandbox
        .exec_file(&script_path, "run", &[])
        .expect("脚本应可执行并返回能力检测结果");

    assert_eq!(result["has_json"], json!(false));
    assert_eq!(result["has_http"], json!(false));
    assert_eq!(result["has_secret"], json!(false));
    assert_eq!(result["has_log"], json!(false));
    assert_eq!(result["has_time"], json!(false));
    cleanup_script(&script_path);
}

#[test]
fn allows_declared_runtime_capability_apis() {
    let script_path = write_temp_script(
        "runtime-capability-allow",
        r#"
            function run()
                local encoded = base64.encode("ok")
                local decoded = base64.decode(encoded)
                return {
                    encoded = encoded,
                    decoded = decoded
                }
            end
        "#,
    );
    let config = LuaSandboxConfig::default().with_capabilities(["base64"]);
    let sandbox = LuaSandbox::new_with_config(config).expect("沙箱初始化应成功");
    let result = sandbox
        .exec_file(&script_path, "run", &[])
        .expect("声明过的 capability API 应可调用");

    assert_eq!(result["encoded"], json!("b2s="));
    assert_eq!(result["decoded"], json!("ok"));
    cleanup_script(&script_path);
}

#[test]
fn exposes_html_and_cookie_apis() {
    let script_path = write_temp_script(
        "html-cookie-apis",
        r#"
            function run()
                cookie.set("session", "token-1", { Path = "/", HttpOnly = true })
                local html_texts = html.query("<ul><li> Alpha </li><li>Beta</li></ul>", "li")
                return {
                    cookie_value = cookie.get("session"),
                    list_count = #html_texts,
                    first_item = html_texts[1],
                    second_item = html_texts[2]
                }
            end
        "#,
    );
    let sandbox = LuaSandbox::new().expect("沙箱初始化应成功");
    let result = sandbox
        .exec_file(&script_path, "run", &[])
        .expect("html/cookie API 应可调用");

    assert_eq!(result["cookie_value"], json!("token-1"));
    assert_eq!(result["list_count"], json!(2));
    assert_eq!(result["first_item"], json!("Alpha"));
    assert_eq!(result["second_item"], json!("Beta"));
    cleanup_script(&script_path);
}

#[test]
fn secret_api_isolation_is_scoped_to_current_plugin() {
    let shared_secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    shared_secret_store
        .set("plugin:plugin.alpha", "password", "alpha-secret")
        .expect("预置插件 A 密钥应成功");
    shared_secret_store
        .set("plugin:plugin.beta", "password", "beta-secret")
        .expect("预置插件 B 密钥应成功");

    let script_path = write_temp_script(
        "secret-namespace",
        r#"
            function run()
                local current = secret.get("password")
                secret.set("session_token", "alpha-token")
                local saved = secret.get("session_token")
                local cross_scope = pcall(function()
                    return secret.get("plugin.beta.password")
                end)
                return {
                    current = current,
                    saved = saved,
                    cross_scope = cross_scope
                }
            end
        "#,
    );

    let config = LuaSandboxConfig::default()
        .with_plugin_id("plugin.alpha")
        .with_secret_store(Arc::clone(&shared_secret_store));
    let sandbox = LuaSandbox::new_with_config(config).expect("沙箱初始化应成功");
    let result = sandbox
        .exec_file(&script_path, "run", &[])
        .expect("secret API 应可调用");

    assert_eq!(result["current"], json!("alpha-secret"));
    assert_eq!(result["saved"], json!("alpha-token"));
    assert_eq!(result["cross_scope"], json!(false));
    let plugin_a_saved = shared_secret_store
        .get("plugin:plugin.alpha", "session_token")
        .expect("插件 A 新密钥应存在");
    assert_eq!(plugin_a_saved.as_str(), "alpha-token");
    let plugin_b_secret = shared_secret_store
        .get("plugin:plugin.beta", "password")
        .expect("插件 B 密钥应保持不变");
    assert_eq!(plugin_b_secret.as_str(), "beta-secret");
    let plugin_b_token = shared_secret_store.get("plugin:plugin.beta", "session_token");
    assert!(
        plugin_b_token.is_err(),
        "插件 A 写入不应污染插件 B 命名空间"
    );
    cleanup_script(&script_path);
}

#[test]
fn enforces_http_request_count_limit() {
    let script_path = write_temp_script(
        "http-limit",
        r#"
            function run()
                for i = 1, 20 do
                    pcall(function()
                        http.request({ url = "http://127.0.0.1:18118/health" })
                    end)
                end
                return http.request({ url = "http://127.0.0.1:18118/health" })
            end
        "#,
    );
    let sandbox = LuaSandbox::new().expect("沙箱初始化应成功");
    let error = sandbox
        .exec_file(&script_path, "run", &[])
        .expect_err("第 21 次请求应触发上限");

    assert!(matches!(error, PluginRuntimeError::ScriptLimit(_)));
    assert_eq!(error.code(), "E_SCRIPT_LIMIT");
    cleanup_script(&script_path);
}

#[test]
fn blocks_loopback_ssrf_target() {
    let script_path = write_temp_script(
        "ssrf-loopback",
        r#"
            function run()
                return http.request({ url = "http://127.0.0.1:18118/health" })
            end
        "#,
    );
    let sandbox = LuaSandbox::new().expect("沙箱初始化应成功");
    let error = sandbox
        .exec_file(&script_path, "run", &[])
        .expect_err("访问 loopback 应被拦截");

    assert!(matches!(error, PluginRuntimeError::ScriptRuntime(_)));
    assert_eq!(error.code(), "E_SCRIPT_RUNTIME");
    cleanup_script(&script_path);
}

#[test]
fn blocks_cloud_metadata_ssrf_target() {
    let script_path = write_temp_script(
        "ssrf-metadata",
        r#"
            function run()
                return http.request({ url = "http://169.254.169.254/latest/meta-data" })
            end
        "#,
    );
    let sandbox = LuaSandbox::new().expect("沙箱初始化应成功");
    let error = sandbox
        .exec_file(&script_path, "run", &[])
        .expect_err("访问云元数据地址应被拦截");

    assert!(matches!(error, PluginRuntimeError::ScriptRuntime(_)));
    assert_eq!(error.code(), "E_SCRIPT_RUNTIME");
    cleanup_script(&script_path);
}

#[test]
fn blocks_dns_rebinding_to_private_ip() {
    let script_path = write_temp_script(
        "dns-rebinding",
        r#"
            function run()
                return http.request({ url = "http://localhost:18118/health" })
            end
        "#,
    );
    let sandbox = LuaSandbox::new().expect("沙箱初始化应成功");
    let error = sandbox
        .exec_file(&script_path, "run", &[])
        .expect_err("域名解析到内网地址时应被拦截");

    assert!(matches!(error, PluginRuntimeError::ScriptRuntime(_)));
    assert_eq!(error.code(), "E_SCRIPT_RUNTIME");
    cleanup_script(&script_path);
}

fn write_temp_script(prefix: &str, content: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间异常")
        .as_nanos();
    let script_path =
        std::env::temp_dir().join(format!("subforge-lua-sandbox-{prefix}-{nanos}.lua"));
    fs::write(&script_path, content).expect("写入脚本文件失败");
    script_path
}

fn cleanup_script(path: &Path) {
    let _ = fs::remove_file(path);
}
