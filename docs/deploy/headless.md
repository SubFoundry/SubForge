# 无头部署

## Docker 运行思路

推荐仅运行 `subforge-core`，将配置与数据目录通过挂载注入。

```bash
subforge-core run -c /etc/subforge/config.toml
```

## 基本建议

- 容器内使用非 root 用户运行
- `config.toml`、`admin_token`、数据库文件权限收敛
- 对外暴露前优先保持监听在回环地址并通过反向代理控制访问

## 运维检查

- `/health` 健康检查
- 定时验证刷新任务状态与错误日志
- 轮换导出 token 并回收旧链接
