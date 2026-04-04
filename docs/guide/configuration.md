# 配置文件说明

SubForge 无头模式通过 TOML 配置驱动，推荐从仓库根目录的 `subforge.example.toml` 开始。

## 最小可运行示例

```toml
[server]
listen = "127.0.0.1:18118"

[secrets]
backend = "env"

[[sources]]
name = "static-source"
plugin = "subforge.builtin.static"
[sources.config]
url = "https://example.com/subscription.txt"

[[profiles]]
name = "default"
sources = ["static-source"]
```

## 顶层结构

配置由这些主要段组成：

- `server`：HTTP 服务监听与管理 token。
- `log`：日志级别、目录与保留策略。
- `storage`：SQLite 路径。
- `secrets`：密钥后端与密钥文件。
- `refresh`：自动刷新与默认间隔。
- `plugins`：插件目录列表。
- `sources`：来源实例（可多个）。
- `profiles`：聚合导出配置（可多个）。

## server

```toml
[server]
listen = "127.0.0.1:18118"
# admin_token = "replace-with-your-admin-token"
```

- `listen`：监听地址，默认建议回环地址。
- `admin_token`：可选；不填时启动自动生成并写入 `data_dir/admin_token`。

## log

```toml
[log]
level = "info"
dir = "./logs"
retention_days = 7
```

- `level`：`trace/debug/info/warn/error`。
- `dir`：日志目录。
- `retention_days`：日志清理保留天数。

## storage

```toml
[storage]
db_path = "./data/subforge.db"
```

- `db_path`：SQLite 文件路径。

## secrets

```toml
[secrets]
backend = "env"
# backend = "file"
# file_path = "./data/secrets.enc"
```

- `backend`：`env` 或 `file`（MVP 常用）。
- `file_path`：`file` 后端密文文件路径。

## refresh

```toml
[refresh]
auto_on_start = true
default_interval_sec = 1800
```

- `auto_on_start`：Core 启动后是否自动刷新来源。
- `default_interval_sec`：默认刷新间隔（秒）。

## plugins

```toml
[plugins]
dirs = [
  "./plugins/builtins/static",
  "./plugins/examples/script-mock",
]
```

- `dirs`：插件搜索目录，按顺序加载。

## sources

每个 `[[sources]]` 对应一个来源实例。

```toml
[[sources]]
name = "script-source"
plugin = "vendor.example.script-mock"
[sources.config]
subscription_url = "https://example.com/mock-subscription.txt"
username = "demo-user"
[sources.secrets]
password = { env = "SUBFORGE_SCRIPT_PASSWORD" }
```

- `name`：来源实例名，需唯一。
- `plugin`：目标插件 `plugin_id`。
- `[sources.config]`：普通配置字段（落库）。
- `[sources.secrets]`：密钥字段（进入 SecretStore），常见写法：
  - `{ env = "ENV_NAME" }`
  - `{ value = "plaintext-for-testing" }`（仅本地调试）

## profiles

每个 `[[profiles]]` 对应一个导出订阅视图。

```toml
[[profiles]]
name = "default"
sources = ["static-source", "script-source"]
# export_token = "replace-with-readonly-token"
```

- `name`：Profile 名称。
- `sources`：要聚合的来源名列表。
- `export_token`：可选；不填时由系统生成。

## 运行与校验

```bash
subforge-core check -c subforge.example.toml
subforge-core run -c subforge.example.toml
```

建议先执行 `check`，再执行 `run`。

## 常见问题

- `plugin not found`：确认 `plugins.dirs` 已包含对应插件目录。
- `secret missing`：确认环境变量已导出，或 `file` 后端主密码已配置。
- 无法访问管理 API：确认 `Authorization: Bearer <admin_token>` 与 Host 头合法。
