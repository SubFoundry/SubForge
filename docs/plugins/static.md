# 静态插件

静态插件用于固定配置链接场景，配置简洁，刷新链路短。

## 最小目录结构

```text
my-plugin/
  plugin.json
  schema.json
```

## plugin.json 最小可导入示例

```json
{
  "plugin_id": "vendor.example.static",
  "spec_version": "1.0",
  "name": "Static Subscription",
  "version": "1.0.0",
  "type": "static",
  "config_schema": "schema.json",
  "network_profile": "standard"
}
```

必填字段：

- `plugin_id`、`spec_version`、`name`、`version`、`type`、`config_schema`

可选字段：

- `secret_fields`（默认 `[]`）
- `capabilities`（默认 `[]`，若填写仅允许白名单值）
- `network_profile`（默认 `standard`）
- `anti_bot_level`（默认 `low`）
- `description`、`homepage`、`license`

注意：

- `plugin_id` 不能包含 `..`、`/`、`\`
- `spec_version` 仅支持 `1.x`
- `config_schema` 路径必须位于插件目录内

## 典型 schema

- `url`（必填）
- `user_agent`（可选；内置静态插件默认值为 `clash.meta`）
