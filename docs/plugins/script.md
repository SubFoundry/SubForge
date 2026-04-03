# 脚本插件

脚本插件用于需要登录、动态 token、风控适配的数据来源。

## 入口契约

- `login(ctx, config)`
- `refresh(ctx, config, state)`
- `fetch(ctx, config, state)`

## 可用 API（白名单）

- `http.request`
- `cookie.get/set`
- `json.parse/stringify`
- `html.query`
- `base64.encode/decode`
- `secret.get/set`
- `time.now`
- `log.info/warn/error`

## 设计建议

- 非敏感状态放 `state`
- 密钥与 token 通过 `secret` API 存取
- 失败信息返回结构化错误，便于 Runs 页面追踪
