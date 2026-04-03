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
    login.lua
    refresh.lua
```

更多字段说明见下级文档。
