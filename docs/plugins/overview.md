# 插件体系

SubForge 插件遵循统一 `plugin.json + schema.json` 规范。

## 插件类型

- `static`：直接拉取配置 URL
- `script`：通过登录/刷新/抓取脚本获取配置数据

## 最小目录结构

```text
my-plugin/
  plugin.json
  schema.json
  scripts/
    fetch.lua
    login.lua (可选)
    refresh.lua (可选)
```

更多字段说明见下级文档。

## 推荐阅读顺序

1. `plugins/static`：了解固定 URL 来源。
2. `plugins/script`：了解脚本入口契约、Runtime API、运行限制与安全边界。
