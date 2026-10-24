# Forge2K

一键构建 CP2K Docker 镜像的 **GUI + CLI** 工具（Rust 编写）。

Forge2K 自动探测 Docker 引擎、选择或现场合成 Dockerfile、流式输出构建日志，支持 Windows / Linux / macOS。构建方法覆盖 Spack（CP2K v2025.2+）、Toolchain（v2023.2+）、master 分支，以及不经 Docker 的主机直连 native 构建。

## 特性亮点

- **三种构建方法 + native**：Spack（v2025.2+）、官方 Toolchain（v2023.2+）、master 分支，外加主机直连的 native 构建（不经 Docker，Linux/WSL）
- **GUI 与 CLI 全对齐**：CLI 的全部功能在 egui 图形界面（Build / Configs / System / Settings / About 五标签页）中均有入口
- **网络诊断与镜像源**：实测 Docker registry 拉取连通性，一键探测 / 配置 / 移除镜像源（daemon.json `registry-mirrors`）
- **Dockerfile 自动合成**：无捆绑 Dockerfile 的组合（2026.1 / 2026.2 / master）按模板现场合成；`--generate-only` 只生成不构建
- **功能预设**：`--features deepmd` 一键开启 CP2K 的 DeePMD（机器学习势）支持
- **构建缓存**：BuildKit cache mounts 复用 apt 包、CP2K 源码与 toolchain tarball 下载；Spack 系另有编译级 buildcache 加速
- **预编译发布**：GitHub Releases 提供双平台 zip（二进制 + README），解压即用

## 快速导航

| 页面 | 内容 |
|---|---|
| [安装](install.md) | 预编译二进制、从源码构建、系统需求 |
| [快速开始](quick-start.md) | 5 分钟上手：GUI、首次构建、list、check |
| [build 参考](cli/build.md) | 全部 14 个选项、tag 命名、退出码、构建缓存 |
| [list 参考](cli/list.md) | 预置配置浏览与方法 / 版本过滤 |
| [check 参考](cli/check.md) | 五项系统诊断详解 |
| [mirror 参考](cli/mirror.md) | registry 镜像源配置与重启 |
| [gui 参考](cli/gui.md) | 图形界面子命令 |
| [GUI 完整指南](gui/overview.md) | 五标签页逐页说明 |
| [spack 方法](methods/spack.md) | Spack 构建原理与版本支持 |
| [toolchain 方法](methods/toolchain.md) | Toolchain 构建、CPU/CUDA 矩阵、功能预设 |
| [native 方法](methods/native.md) | 主机直连构建、组合白名单、依赖补装 |
| [故障排查](troubleshooting.md) | 常见问题与解决方案 |
| [FAQ](faq.md) | 高频问答 |
| [开发指南](development.md) | 架构、测试与贡献 |
| [覆盖矩阵](coverage.md) | CLI / GUI 功能覆盖附录 |

## 系统需求摘要

- **Docker Desktop**（Windows / macOS）或 **Docker Engine**（Linux）——`spack` / `toolchain` 方法必需
- **native 方法**（主机直连构建）：Linux / WSL，需要 `git`、`gfortran`、`make`/`cmake` 等构建工具（缺失时 Forge2K 会尝试用 apt 自动补装）；Windows 原生主机不可用
- Rust stable 工具链——仅从源码构建 Forge2K 本身时需要

详见[安装](install.md)。Docker 未安装时 `forge2k check` 会打印分平台安装指引；拉取镜像慢或被阻断时用 `forge2k mirror` 配置镜像源（见 [mirror 参考](cli/mirror.md)）。
