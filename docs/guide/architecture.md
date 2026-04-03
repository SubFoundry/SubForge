# 架构总览

SubForge 采用 **Core + Desktop** 分离架构：

- `subforge-core`：独立守护进程，承载刷新调度、插件运行、聚合转换、HTTP API。
- `subforge-desktop`：可选 GUI，仅用于管理与观察。

## 通信边界

- 管理与数据接口统一通过 Core HTTP API。
- Desktop 的进程生命周期管理走 Tauri IPC。
- `admin_token` 只在 Rust 侧内存中保存，不落入 WebView JS 上下文。

## 运行模式

- 桌面模式：Desktop 可随时开关，Core 常驻。
- 无头模式：仅运行 Core + TOML 配置，适合服务器与容器部署。
