# 安装

## 方式一：预编译二进制（推荐）

GitHub Releases 提供 **双平台 zip 包**（Windows 与 Linux），每个 zip 内含 `forge2k` 可执行文件与 README，下载后解压即用：

1. 打开 [GitHub Releases](https://github.com/SeveNOlogy7/forge2k/releases) 页面
2. 下载对应平台的 zip 包
3. 解压到任意目录，直接运行其中的 `forge2k`

!!! note "关于 draft 发布"

    当前发布习惯：推送 `v*` tag 后 CI（release workflow）会先跑 fmt / clippy / test 门禁，再双平台构建、产物冒烟，最后把 zip（二进制 + README）上传为 **GitHub Release 草稿（draft）**，由维护者手动发布。若 Releases 列表暂无正式版本，请等待维护者发布对应草稿。

!!! warning "Windows release 构建的 CLI 输出"

    Windows release 版 `forge2k.exe` 为 GUI 子系统构建：双击直接打开图形界面；从 cmd / PowerShell 运行时 CLI 输出可能不回显（退出码正常）。debug 构建不受影响（见 README 已知限制）。

## 方式二：从源码构建

需要 Rust stable 工具链（推荐通过 [rustup](https://rustup.rs/) 安装）：

```bash
git clone https://github.com/SeveNOlogy7/forge2k.git
cd forge2k
cargo build --release
```

产物位于 `target/release/forge2k`（Windows 为 `forge2k.exe`）。

## 系统需求

| 需求 | 说明 |
|---|---|
| Docker Desktop（Windows / macOS）或 Docker Engine（Linux） | `spack` / `toolchain` 方法必需；建议使用自带 BuildKit（buildx）的现代版本 |
| native 方法：Linux / WSL | 需要 `git`、`gfortran`、`make`/`cmake` 等构建工具链；**缺失时 Forge2K 会尝试用 apt 自动补装**（工具与包清单见 [native 方法](methods/native.md)）。Windows 原生主机不可用 |
| Rust stable | 仅从源码构建 Forge2K 本身时需要 |

## 下一步

- [快速开始](quick-start.md)：首次构建与预期时长
- [GUI 完整指南](gui/overview.md)：图形界面逐页说明
- [故障排查](troubleshooting.md)：Docker 未运行、registry 被阻断等常见问题
