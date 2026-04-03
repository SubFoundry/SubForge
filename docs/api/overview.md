# HTTP API 总览

## 健康检查

- `GET /health`（无需鉴权）

## 管理 API（admin_token）

- `GET /api/system/status`
- `PUT /api/system/settings`
- `GET/POST/PUT/DELETE /api/plugins`
- `GET/POST/PUT/DELETE /api/sources`
- `GET/POST/PUT/DELETE /api/profiles`
- `POST /api/tokens/{profile_id}/rotate`
- `GET /api/events`（SSE）

## 配置读取 API（export_token）

- `GET /api/profiles/{id}/clash?token=...`
- `GET /api/profiles/{id}/sing-box?token=...`
- `GET /api/profiles/{id}/base64?token=...`
- `GET /api/profiles/{id}/raw?token=...`

## 统一错误结构

```json
{
  "code": "E_AUTH",
  "message": "...",
  "retryable": false
}
```
