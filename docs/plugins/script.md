# 脚本开发

脚本插件适用于“需要登录、动态刷新 token、需要流程编排”的来源。

## 插件目录结构

```text
my-plugin/
  plugin.json
  schema.json
  scripts/
    fetch.lua
    login.lua
    refresh.lua
```

最小要求：`type = "script"` 且包含 `fetch.lua`。

## plugin.json 关键字段

```json
{
  "plugin_id": "vendor.example.dynamic-sub",
  "spec_version": "1.0",
  "type": "script",
  "entrypoints": {
    "login": "scripts/login.lua",
    "refresh": "scripts/refresh.lua",
    "fetch": "scripts/fetch.lua"
  },
  "secret_fields": ["password"],
  "network_profile": "browser_chrome"
}
```

- `spec_version`：当前支持 `1.x`（主版本号为 `1`）。
- `secret_fields`：必须是 schema 中定义过的字段。
- `network_profile`：`standard / browser_chrome / browser_firefox / webview_assisted`。

## 入口函数契约

- `login(ctx, config, state) -> { ok, state?, error? }`
- `refresh(ctx, config, state) -> { ok, state?, error? }`
- `fetch(ctx, config, state) -> { ok, subscription?, state?, error? }`

`subscription` 支持两种返回：
- `{ url = "https://..." }`
- `{ content = "base64 or uri lines text" }`

推荐约定：
- 失败统一 `ok = false` 并返回结构化 `error`。
- 非敏感上下文写入 `state`，敏感值写入 `secret` API。

## Runtime API 白名单

- `http.request({ method, url, headers, body, timeout_ms })`
- `cookie.get(name)` / `cookie.set(name, value, attrs)`
- `json.parse(str)` / `json.stringify(obj)`
- `html.query(html, selector)`
- `base64.encode(str)` / `base64.decode(str)`
- `secret.get(key)` / `secret.set(key, value)`
- `log.info(msg)` / `log.warn(msg)` / `log.error(msg)`
- `time.now()`

`http.request` 额外约定：
- `method` 可省略，默认 `GET`。
- 返回状态码非 2xx 时会直接抛出运行时错误（不会返回 `resp.status` 让脚本自行判断）。

不允许：
- 系统命令
- 文件系统访问
- 任意 socket
- 动态模块加载

## 运行限制（MVP）

- 单次脚本执行超时：`20s`
- 单次 `http.request` 超时：`15s`
- 单次执行最大 HTTP 请求数：`20`
- 单次执行内存上限：`64MB`
- 单次 HTTP 响应体上限：`5MB`

常见错误码：
- `E_SCRIPT_TIMEOUT`
- `E_SCRIPT_LIMIT`
- `E_SCRIPT_RUNTIME`

## SSRF 与安全边界

`http.request` 会在 DNS 解析后做 IP 校验，命中内网/回环网段会直接拒绝。

示例（会被拒绝）：

```lua
http.request({ method = "GET", url = "http://127.0.0.1:18118/health" })
```

此外，插件只能访问自身 `plugin_id` 的 secret 命名空间，不能读取其他插件或系统密钥。

## 示例：最小 fetch.lua

```lua
function fetch(ctx, config, state)
  local ok, resp = pcall(http.request, {
    method = "GET",
    url = config.subscription_url,
    timeout_ms = 10000
  })

  if not ok then
    return { ok = false, error = { code = "E_HTTP", message = tostring(resp) } }
  end

  return {
    ok = true,
    subscription = { content = resp.body },
    state = state
  }
end
```

## 调试建议

- 先在 mock 服务上验证 `login -> refresh -> fetch`。
- `log.*` 只记录必要上下文，避免输出敏感字段。
- 对 429/403 场景设计可恢复重试，不要写死无限循环。
