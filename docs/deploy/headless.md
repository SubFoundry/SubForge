# 无头部署

## Docker 运行思路

推荐仅运行 `subforge-core`，将配置与数据目录通过挂载注入。

```bash
subforge-core run -c /etc/subforge/config.toml
```

## Docker 镜像（非 root）

仓库根目录提供了用于无头模式的 `Dockerfile`，容器内默认使用 `subforge` 非 root 用户运行。

### 构建镜像

```bash
docker build -t subforge-core:local .
```

### 快速启动（env secrets 后端）

默认命令不依赖配置文件，可直接用于健康检查与空白实例启动：

```bash
docker run --rm -p 18118:18118 -v "$PWD/.subforge-data:/var/lib/subforge" \
  subforge-core:local
```

### 生产建议（file secrets 后端）

若使用仓库示例配置文件（`subforge.example.toml`），需额外提供 `SUBFORGE_SCRIPT_PASSWORD`。`file` 后端需要主密码，建议通过环境变量注入：

```bash
docker run --rm -p 18118:18118 \
  -e SUBFORGE_SCRIPT_PASSWORD="replace-with-source-password" \
  -e SUBFORGE_SECRET_KEY="replace-with-strong-passphrase" \
  -v "$PWD/subforge.example.toml:/etc/subforge/config.toml:ro" \
  -v "$PWD/.subforge-data:/var/lib/subforge" \
  subforge-core:local run -c /etc/subforge/config.toml --data-dir /var/lib/subforge --secrets-backend file
```

## 基本建议

- 容器内使用非 root 用户运行
- `config.toml`、`admin_token`、数据库文件权限收敛
- 对外暴露前优先保持监听在回环地址并通过反向代理控制访问

## 运维检查

- `/health` 健康检查
- 定时验证刷新任务状态与错误日志
- 轮换导出 token 并回收旧链接
