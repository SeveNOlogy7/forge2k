# mirror — 配置 registry 镜像源

别名 `m`。配置 Docker daemon 的 registry 镜像源（拉取镜像慢或被网络阻断时使用）。

```bash
forge2k mirror [url] [-r|--remove] [--restart]
```

## 用法示例

```bash
forge2k mirror                                       # 省略 url：自动探测最佳镜像
forge2k mirror https://docker.mirrors.ustc.edu.cn    # 手动指定镜像 URL
forge2k mirror -r                                    # 移除镜像源配置
forge2k mirror -r --restart                          # 移除配置并重启 Docker（两个动作都执行）
forge2k mirror https://docker.mirrors.ustc.edu.cn --restart   # 写入配置并重启 Docker
```

## 自动探测语义

省略 `url` 位置参数时，按内置已知镜像列表（5 个，见下表）**逐个探测**：请求 `<url>/v2/_catalog`（5 秒超时），HTTP 200 或 401 均视为可用（401 表示 registry 存在但需认证），取**第一个可用者**。全部不可用时打印部分已知镜像示例并提示手动提供 URL。

内置镜像（`src/mirrors.rs`，探测顺序）：

| 名称 | URL | 说明 |
|---|---|---|
| USTC | `https://docker.mirrors.ustc.edu.cn` | 中国 · 中国科学技术大学 |
| Tencent Cloud | `https://mirror.ccs.tencentyun.com` | 中国 · 腾讯云 |
| DaoCloud | `https://2a59f68c.m.daocloud.io` | 中国 · DaoCloud |
| Docker CN | `https://registry.docker-cn.com` | 中国 · Docker 官方 CN |
| DockerHub Proxy | `https://dockerhub.timeweb.cloud` | 俄罗斯 · Timeweb Cloud |

## url 参数

手动指定的 URL **必须以 `http://` 或 `https://` 开头**，否则拒绝写入。

## 写入行为与 daemon.json 路径

镜像源写入 daemon.json 的 `registry-mirrors` 键：

| 平台 | 路径 |
|---|---|
| Windows | `%USERPROFILE%\.docker\daemon.json` |
| Linux / macOS | `/etc/docker/daemon.json` |

- **单元素覆盖**：`registry-mirrors` 键被**整体覆盖为单元素数组**（`[本次写入的 URL]`）；daemon.json 中其余键原样保留。需要配置多个镜像须手动编辑该文件
- 写入后**需重启 Docker 才生效**（见 `--restart`）

## -r / --remove

从 daemon.json 删除 `registry-mirrors` 键（其余键保留）；配置本就不存在时静默成功。

## --restart

重启 Docker 使配置生效，两平台实现不同：

- **Windows**：强制结束 Docker Desktop（`taskkill`）→ 等 3 秒 → 重新拉起（`cmd /C start`）→ 轮询 `docker version` 等待引擎就绪，**最长 60 秒**；超时或拉起失败时给出手动启动指引
- **非 Windows（Linux/macOS）**：执行 `sudo systemctl restart docker`；失败时回退 rootless 的 `systemctl --user restart docker`

## 组合语义

- `-r` 与 `--restart` 可同时使用，**两个动作都执行**（先移除配置、再重启），`--restart` 不会因 `-r` 被忽略
- 不带 `--restart` 时打印对应平台的手动重启指引：Windows——从系统托盘重启 Docker Desktop；Linux——`sudo systemctl restart docker`

## 相关页面

- [check 参考](check.md)：Registry 项判级与当前 mirror 显示
- [故障排查](../troubleshooting.md)
