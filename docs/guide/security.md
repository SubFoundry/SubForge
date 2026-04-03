# 安全模型

## HTTP 安全基线

- Host Header 白名单校验（`127.0.0.1` / `localhost` / `[::1]`）
- 默认拒绝跨域（不返回 `Access-Control-Allow-Origin`）
- 双 token 隔离：
  - `admin_token`：管理接口（Header `Authorization: Bearer`）
  - `export_token`：配置读取接口（Query `?token=`）

## 脚本沙箱

- 禁用 `os` / `io` / `debug` / `require` 等危险能力
- SSRF 保护：拒绝访问保留地址与内网网段
- 资源限制：执行时长、内存、请求次数、响应体大小、指令数

## Secret 处理

- 普通配置入 SQLite
- 敏感字段进入 SecretStore（keyring/env/file/memory）
- 敏感日志脱敏：仅保留前缀 + `***`
