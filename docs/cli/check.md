# check — 系统诊断

别名 `c`。逐项检查构建环境并打印结果（GUI 的 System 标签页与之同源）。

```bash
forge2k check
```

全程只读探测、不做任何修改；**无论各项结果如何，退出码始终为 0**。

## 五项检测详解

| # | 检测项 | 探测方式 |
|---|---|---|
| 1 | Docker Engine | `docker version` |
| 2 | Docker BuildKit | `docker buildx version` |
| 3 | Docker Registry | 实际 `docker pull alpine:latest` 实测拉取 |
| 4 | 当前 mirror | 读 daemon.json 的 `registry-mirrors` |
| 5 | Docker Storage | `docker info` 取根目录 + 所在卷磁盘空间 |

### 1. Docker Engine

`docker version` 探测安装与运行状态，并显示引擎版本。四种结果：已安装且运行、已安装未运行（提示启动 Docker Desktop 或 `sudo systemctl start docker`）、未安装（打印分平台的安装指引：Windows 的 Docker Desktop 步骤 / WSL 2 内 get.docker.com 脚本 / Linux 官方 apt 源步骤 / macOS 的 Docker Desktop 步骤）、daemon 不可达。

### 2. Docker BuildKit

`docker buildx version` 探测 buildx 插件；未检出时提示构建将回退 legacy builder（**信息级提示，非错误**）。cache mount 加速依赖 BuildKit，建议保持可用。

### 3. Docker Registry

真实执行 `docker pull alpine:latest`（拉完即 `docker rmi` 清理），按耗时与错误判级：

| 结果 | 判据 | 提示 |
|---|---|---|
| Good（绿色） | 拉取成功且 **10 秒内**完成 | registry 访问良好 |
| Slow（黄色） | 拉取成功但耗时 ≥ 10 秒（显示实际耗时） | 可考虑镜像源 |
| Blocked（红色） | 拉取失败，且 stderr 含 `timeout` / `refused` / `no route` | 建议执行 `forge2k mirror` 自动探测（见 [mirror 参考](mirror.md)） |
| Unknown（黄色 `?`） | 其他失败原因 | 显示原始 stderr |

### 4. 当前 mirror

读取 daemon.json 的 `registry-mirrors` 键并显示其**首个条目**；未配置镜像源时不显示该行（不视为错误）。

### 5. Docker Storage

两段信息：

- **根目录**：`docker info --format {{.DockerRootDir}}`；daemon 不可达时显示未知
- **磁盘空间**：检测 Docker 根目录所在卷的剩余 / 已用 / 总量。Windows 用 PowerShell `Get-CimInstance Win32_LogicalDisk` 探测（**不再使用 wmic**——wmic 已在 Windows 11 24H2+ 被移除，旧探测方式在新系统必然空输出）；根目录无盘符（如 Docker Desktop WSL 后端的 `/var/lib/docker`）时回退枚举全部固定磁盘。Unix 用 `df -h <根目录>`

## Summary

最后汇总：Docker 就绪则提示系统可以开始构建（`Run 'forge2k build' or 'forge2k gui' to get started.`）；否则提示先解决上述问题。

## 相关页面

- [mirror 参考](mirror.md)：Registry 项 Blocked 时的处理手段
- [故障排查](../troubleshooting.md)
