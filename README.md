# Forge2K

一键构建 CP2K Docker 镜像的 GUI + CLI 工具（Rust 编写）。

Forge2K 自动探测 Docker 引擎、选择或生成 Dockerfile、流式输出构建日志，并支持 Windows / Linux / macOS。支持 Spack 构建（CP2K v2025.2+）、Toolchain 构建（v2023.2+）、master 分支构建，以及主机直连的 native 构建。

## 系统需求

- **Docker Desktop**（Windows / macOS）或 **Docker Engine**（Linux）——`spack` / `toolchain` 方法必需
- Rust stable 工具链（仅从源码构建 Forge2K 本身时需要）
- **native 方法**（主机直连构建）：Linux/WSL，需要 `git`、`gfortran`、`make`/`cmake` 等构建工具（缺失时 Forge2K 会尝试用 apt 自动安装）

## 快速开始

```bash
cargo build --release

# 打开图形界面（无参数默认启动 GUI）
forge2k

# CLI 构建（默认版本 2026.1）
forge2k build -m spack -v 2026.1 --mpi mpich --cpu x86_64 --variant psmp

# 查看全部预置配置
forge2k list
```

## CLI 子命令

| 子命令 | 别名 | 说明 |
|---|---|---|
| `build` | `b` | 构建 CP2K Docker 镜像（CLI 模式） |
| `list` | `l` | 列出全部预置构建配置 |
| `gui` | `g` | 启动图形界面 |
| `check` | `c` | 检查系统需求（Docker、网络等） |
| `mirror` | `m` | 配置 Docker registry 镜像源（网络问题时使用） |

无参数运行 `forge2k` 默认启动 GUI。

### build 选项

```text
-m,  --method <METHOD>    构建方法: spack / toolchain / native [默认: spack]
-v,  --version <VERSION>  CP2K 版本: 2026.1 / 2025.2 / 2023.2 / master [默认: 2026.1]
     --mpi <MPI>          MPI 实现: mpich / openmpi [默认: mpich]
     --cpu <CPU>          CPU 目标: x86_64 / generic / cascadelake / haswell / skylake-avx512 [默认: x86_64]
     --cuda <CUDA>        CUDA GPU: none / P100 / V100 [默认: none]
     --variant <VARIANT>  二进制变体: psmp / ssmp / pdbg / sdbg [默认: psmp]
-j,  --jobs <JOBS>        并行构建任务数（默认自动探测）
-t,  --tag <TAG>          自定义镜像 tag
     --no-cache           禁用 Docker 构建缓存
-f,  --dockerfile <PATH>  指定自定义 Dockerfile（覆盖自动选择）
     --shm-size <SIZE>    Docker 构建共享内存 [默认: 1g]
     --force              跳过 Docker 引擎检查
```

构建过程中按 `q` 或回车可取消（取消会终止 docker 进程并返回非零退出码）。构建失败时 CLI 以非零退出码结束，只有成功才打印完成提示。

### 镜像 tag 命名

`cp2k/cp2k:{version}_{mpi}_{cpu}[_cuda_{GPU}]_{variant}`，例如 `cp2k/cp2k:2025.2_mpich_x86_64_psmp`；master 分支使用构建当日日期戳（`master_YYYYMMDD`）。

## GUI

`forge2k gui` 或直接运行，四个标签页：

- **Build**——选择方法/版本/MPI/CPU/CUDA/变体，启动、取消构建，实时滚动日志；构建结果以状态条显示（绿=成功 / 红=失败 / 黄=已取消）
- **System**——Docker 引擎与系统信息
- **Settings**——registry 镜像源管理（与 `mirror` 子命令同源）
- **About**——项目信息

## 构建方法

- **spack**（默认）：基于 `ubuntu:24.04`，两段式构建——Spack 环境解析 CP2K 依赖后安装，运行时镜像只保留安装产物。支持 v2025.2+ 与 master。
- **toolchain**：基于 `ubuntu:22.04`（CUDA 变体基于 `nvidia/cuda:12.2.0-devel`），运行 CP2K 官方 `install_cp2k_toolchain.sh` 后 `make`。支持 v2023.2 与 CUDA（P100/V100）。
- **native**：不经 Docker，直接在主机上克隆 CP2K 源码、跑 toolchain 安装、CMake/make 构建。当前仅支持默认组合（x86_64 / system mpich / psmp），其他组合会显式报错。

## Dockerfile 捆绑清单

| 文件 | 说明 |
|---|---|
| `dockerfiles/spack/2025.2_mpich_cascadelake_psmp.Dockerfile` | Spack + mpich，cascadelake 优化 |
| `dockerfiles/spack/2025.2_mpich_x86_64_psmp.Dockerfile` | Spack + mpich，x86_64 |
| `dockerfiles/spack/2025.2_openmpi_cascadelake_psmp.Dockerfile` | Spack + openmpi，cascadelake |
| `dockerfiles/toolchain/2023.2_mpich_generic_psmp.Dockerfile` | Toolchain + mpich，generic |
| `dockerfiles/toolchain/2023.2_mpich_generic_cuda_P100_psmp.Dockerfile` | Toolchain + CUDA P100 |
| `dockerfiles/toolchain/2023.2_mpich_generic_cuda_V100_psmp.Dockerfile` | Toolchain + CUDA V100 |

**默认版本 2026.1（与 master）没有捆绑 Dockerfile**：构建时 Forge2K 会按 `src/templates/` 模板现场合成（日志中会显式提示 synthesized），功能等价但未经镜像级端到端验证。

## 开发与测试

```bash
cargo check                  # 编译检查
cargo test                   # 单元 + 行为测试（16 个，不依赖 Docker daemon）
cargo clippy -- -D warnings  # lint 门禁
cargo fmt --check            # 格式门禁
```

CI（`.github/workflows/ci.yml`）在 push 与 PR 时于 ubuntu / windows 双平台跑上述四道门禁。

## 已知限制

- CLI 与 GUI 默认版本已统一为 **2026.1**（2026.1 走 synthesized 合成路径，见上）
- native 构建仅支持默认参数组合，GUI 中选择不支持组合会显式报错
- `mirror --restart` 在 Windows 上经 `cmd /C start` 重启 Docker Desktop，未做破坏性端到端验证
