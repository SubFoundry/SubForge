# 静态插件

静态插件用于固定配置链接场景，配置简洁，刷新链路短。

## 关键字段

- `type = "static"`
- `config_schema` 指向 `schema.json`
- `network_profile` 决定拉取策略（如 `standard`）

## 典型 schema

- `url`（必填）
- `user_agent`（可选）
