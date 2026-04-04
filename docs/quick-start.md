# 快速开始

## 合规提示

本文档仅描述工程化配置管理流程，不提供任何规避访问限制的操作指引。

## 环境要求

- Rust stable（>= 1.94.1）
- Node.js 24.x
- pnpm 10.x

## 1. 拉取并安装依赖

```bash
cargo fetch
pnpm install
```

## 2. 启动 Core（无头）

```bash
cargo run -p subforge-core -- run -c subforge.example.toml
```

默认健康检查：

```bash
curl http://127.0.0.1:18118/health
```

## 3. 启动 Desktop（可选）

```bash
pnpm desktop:tauri:dev
```

Desktop 会检测并连接本地 Core；若 Core 未启动，将按配置尝试拉起。

## 4. 最小验证

```bash
cargo check --workspace
cargo test --workspace
```

## 下一步

- 配置文件说明：`/guide/configuration`
- 脚本开发说明：`/plugins/script`
- 无头部署：`/deploy/headless`
