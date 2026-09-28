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

构建过程中按 `q` 或回车可取消（GUI 的 CANCEL 按钮等价）。取消会终止 docker 进程并返回非零退出码；构建失败同样以非零退出码结束，只有成功才打印完成提示。

### 镜像 tag 命名

`cp2k/cp2k:{version}_{mpi}_{cpu}[_cuda_{GPU}]_{variant}`，例如 `cp2k/cp2k:2025.2_mpich_x86_64_psmp`；master 分支使用构建当日日期戳（`master_YYYYMMDD`）。

## GUI

`forge2k gui` 或直接运行，四个标签页：

- **Build**——选择方法/版本/MPI/CPU/CUDA/变体，启动、取消构建，实时滚动日志；构建结果以状态条显示（绿=成功 / 红=失败 / 黄=已取消）；CUDA 选项按所选方法联动过滤（spack 仅 none，toolchain 可选 P100/V100）
- **System**——Docker 引擎与系统信息
- **Settings**——registry 镜像源管理（与 `mirror` 子命令同源）；**设置会持久化**到 `~/.forge2k/settings.json`，重启后保留
- **About**——项目信息与已知限制

## 构建方法

- **spack**（默认）：基于 `ubuntu:24.04`，两段式构建——Spack 环境解析 CP2K 依赖后安装，运行时镜像只保留安装产物。支持 v2025.2+ 与 master。
- **toolchain**：基于 `ubuntu:22.04`（CUDA 变体基于 `nvidia/cuda:12.2.0-devel`），运行 CP2K 官方 `install_cp2k_toolchain.sh` 后 `make`。支持 v2023.2 与 CUDA（P100/V100）。
- **native**：不经 Docker，直接在主机上克隆 CP2K 源码、跑 toolchain 安装、CMake/make 构建。当前仅支持默认组合（x86_64 / system mpich / psmp / cuda none），其他组合会显式报错。

## Dockerfile 捆绑清单

| 文件 | 说明 |
|---|---|
| `dockerfiles/spack/2025.2_mpich_cascadelake_psmp.Dockerfile` | Spack + mpich，cascadelake 优化 |
| `dockerfiles/spack/2025.2_mpich_x86_64_psmp.Dockerfile` | Spack + mpich，x86_64 |
| `dockerfiles/spack/2025.2_openmpi_cascadelake_psmp.Dockerfile` | Spack + openmpi，cascadelake |
| `dockerfiles/toolchain/2023.2_mpich_generic_psmp.Dockerfile` | Toolchain + mpich，generic |
| `dockerfiles/toolchain/2023.2_mpich_generic_cuda_P100_psmp.Dockerfile` | Toolchain + CUDA P100 |
| `dockerfiles/toolchain/2023.2_mpich_generic_cuda_V100_psmp.Dockerfile` | Toolchain + CUDA V100 |

**默认版本 2026.1（与 master）没有捆绑 Dockerfile**：构建时 Forge2K 会按 `src/templates/` 模板现场合成（日志中会显式提示 synthesized），功能等价但未经镜像级端到端验证。捆绑的 2025.2 spack 镜像与 2023.2 toolchain 镜像已做过全量构建与运行冒烟验证。

## 发布

推送 `v*` tag（如 `v1.0.1`）会触发 `.github/workflows/release.yml`：先跑 fmt/clippy/test 门禁，再双平台 release 构建、产物冒烟，最后把 zip（二进制 + README）上传为 **GitHub Release 草稿**，由维护者手动 publish。

## 构建缓存

重复构建同版本镜像时，六份捆绑 Dockerfile 与合成 Dockerfile 通过 **BuildKit cache mounts** 复用下载，`--no-cache` 重建或层缓存被逐出时依然命中：

| 下载向量 | cache id | 共享范围 |
|---|---|---|
| apt 包 | `apt-2404`（spack 系）/ `apt-2204`（toolchain 系） | 同基础发行版的全部构建 |
| CP2K 源码 | `cp2k-src-<版本>`（如 `cp2k-src-2025.2`） | 同版本全部变体；重建时 `git fetch+reset+submodule update` 增量刷新 |
| Spack 包源码 | `spack-src` | 全部 spack 变体（含 CUDA） |
| toolchain tarballs | `toolchain-tarballs` | 全部 toolchain 变体（含 CUDA） |

要点：

- cache mount 由 BuildKit 管理，**独立于镜像层缓存**：Dockerfile 变更导致的层失效不影响下载缓存。二次构建的提速幅度 = 下载段（实测：apt ~26 倍、CP2K 源码 clone 5-8 分钟 → fetch 秒级、toolchain tarball 直接跳过），编译段仍会重跑。
- **`--no-cache` 注意**：实测（Docker Desktop 29.x）`--no-cache` 构建中 cache mount 视图为空、不命中既有缓存——如只想利用下载缓存，请通过修改 Dockerfile 制造层失效，而非 `--no-cache`。
- toolchain 安装带**自愈重试**：下载中断留下的残缺 tar 包会被自动清理并重下（skip-if-exists 只信完好文件）。
- 清空缓存：`docker builder prune --filter type=exec.cachemount`
- 缓存磁盘占用约 5-6GB（估计值，以 `docker system df` 实测为准）。
- 注意 named volume（`docker run -v`）与这里的 cache mount 无关：前者是运行期数据卷，后者是构建期缓存，由 BuildKit 自动管理、无需手工挂载。
- 镜像 pull 加速是另一层，走 `forge2k mirror` 子命令（改 daemon.json 的 registry mirror）。

## 项目结构

```
src/
├── main.rs          # clap CLI（build/list/gui/check/mirror）+ stdin 取消
├── gui.rs           # egui 四标签界面
├── settings.rs      # 设置持久化（serde + ~/.forge2k/settings.json）
├── mirrors.rs       # registry 镜像源单一来源
├── templates/       # 合成 Dockerfile 模板（include_str!）
└── build/
    ├── mod.rs       # 共享类型（BuildConfig/BuildOutcome/LogLine）+ re-export
    ├── execute.rs   # docker/native 构建执行与取消
    ├── registry.rs  # registry 检测与镜像源配置
    ├── catalog.rs   # 捆绑 Dockerfile 目录扫描与配置列表
    ├── generate.rs  # 合成 Dockerfile 生成
    ├── docker.rs    # Docker 引擎探测与重启
    └── tests.rs     # 测试套件
```

## 开发与测试

```bash
cargo check                  # 编译检查
cargo test                   # 单元 + 行为测试（28 个，不依赖 Docker daemon）
cargo clippy --all-targets -- -D warnings  # lint 门禁（CI 同款口径）
cargo fmt --check            # 格式门禁
```

CI（`.github/workflows/ci.yml`）在 push 与 PR 时于 ubuntu / windows 双平台跑上述四道门禁。

## 已知限制

- Windows release 构建是 GUI 子系统，CLI 无 stdout（退出码正常）；debug 构建不受影响
- native 构建仅支持默认参数组合，GUI 中选择不支持组合会显式报错
- CUDA P100/V100 变体已在 GitHub Actions 完成全量构建验证（2026-09-29，均成功）；容器内 `cp2k --version` 需 NVIDIA 驱动（无 GPU 主机因缺 libcuda.so.1 无法运行，属环境边界非镜像缺陷）
- 2026.1 / master 走 synthesized 合成路径（见上）
- `mirror --restart` 在 Windows 上经 `cmd /C start` 重启 Docker Desktop，未做破坏性端到端验证
