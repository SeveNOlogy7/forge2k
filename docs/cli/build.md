# build — 构建 CP2K 镜像

别名 `b`。以 CLI 模式构建 CP2K Docker 镜像；`native` 方法不经 Docker，直接在主机上构建。

```bash
forge2k build [选项]
```

## 全部选项

| # | 选项 | 默认值 | 允许值 | 行为 |
|---|---|---|---|---|
| 1 | `-m, --method <METHOD>` | `spack` | `spack` / `toolchain` / `native`（白名单校验） | 构建方法：spack（Docker，ubuntu:24.04）、toolchain（Docker，ubuntu:22.04）、native（主机直连） |
| 2 | `-v, --version <VERSION>` | `2026.1` | 自由字符串；广告值 `2026.2` / `2026.1` / `2025.2` / `2023.2` / `master` | CP2K 版本；`master` 构建最新 HEAD。**不做取值校验**（见下方"自由字符串说明"） |
| 3 | `--mpi <MPI>` | `mpich` | `mpich` / `openmpi`（白名单校验） | MPI 实现 |
| 4 | `--cpu <CPU>` | `x86_64` | 自由字符串；广告值 `x86_64` / `generic` / `cascadelake` / `haswell` / `skylake-avx512` | CPU 目标。**不做取值校验**（见下方"自由字符串说明"） |
| 5 | `--cuda <CUDA>` | `none` | `none` / `P100` / `V100`（白名单校验） | CUDA GPU 型号；spack 方法仅支持 `none`（见 [spack 方法](../methods/spack.md)） |
| 6 | `--variant <VARIANT>` | `psmp` | `psmp` / `ssmp` / `pdbg` / `sdbg`（白名单校验） | CP2K 二进制变体 |
| 7 | `--features <FEATURES>` | 空 | 逗号分隔；当前可用 `deepmd`（白名单校验） | 功能预设，仅 toolchain 方法 2026.x release-cmake 臂支持（详见下文） |
| 8 | `-j, --jobs <JOBS>` | `0` | 非负整数 | 并行构建任务数；`0` = 自动探测（CPU 核心数） |
| 9 | `-t, --tag <TAG>` | 空 | 任意字符串 | 自定义镜像 tag；缺省按规则自动命名（见下文） |
| 10 | `--no-cache` | 关 | 开关 | 禁用 Docker 层缓存（`docker build --no-cache`） |
| 11 | `-f, --dockerfile <PATH>` | 空 | 路径 | 指定自定义 Dockerfile，覆盖自动选择；与 `--features` 互斥 |
| 12 | `--shm-size <SIZE>` | `1g` | Docker 尺寸串 | Docker 构建共享内存（OpenMPI 多 rank 场景需要） |
| 13 | `--force` | 关 | 开关 | 跳过 Docker 引擎检查 |
| 14 | `--generate-only` | 关 | 开关 | 仅解析 / 合成 Dockerfile 并打印路径：跳过 Docker 引擎检查与构建确认提示，退出码 0，不执行构建 |

## tag 自动命名规则

未指定 `-t/--tag` 时按配置生成：

```text
cp2k/cp2k:{version}_{mpi}_{cpu}[_cuda_{GPU}]_{variant}[_{features}]
```

- CUDA 非 `none` 时插入 `_cuda_{GPU}` 段，例如 `--cpu generic --cuda P100` → `cp2k/cp2k:2026.2_mpich_generic_cuda_P100_psmp`
- 带 `--features` 时在 variant 之后追加预设名，例如 `--features deepmd` → `cp2k/cp2k:2026.2_mpich_generic_psmp_deepmd`
- `master` 分支使用构建当日日期戳：`cp2k/cp2k:master_YYYYMMDD`

## --generate-only

跳过 Docker 引擎检查与 `Start build?` 确认提示，只完成 Dockerfile 的解析（命中捆绑文件）或合成（按模板现场生成），打印其路径后以退出码 0 结束。合成文件写入系统临时目录下的 `forge2k/` 子目录（如 Windows 的 `%TEMP%\forge2k\`）。适合在没有 Docker 的环境预览将要构建的内容。

## --features deepmd

`--features`（逗号分隔）按需开启镜像功能，当前可用 preset 为 `deepmd`：

```bash
forge2k build -m toolchain -v 2026.2 --features deepmd
```

- **支持臂（硬性约束）**：仅 `toolchain` 方法、版本 ≥ 2026.1 且非 `master`（release-cmake 构建臂）。在 spack / native / master / 2023.2 make 臂上携带 `--features` 会直接报错退出，不会静默忽略
- **与 `-f/--dockerfile` 互斥**：显式 Dockerfile 与功能预设同启时报错（预设只影响合成 Dockerfile）；带 `--features` 时即使存在同配置的捆绑 Dockerfile 也强制走合成路径
- **未知 / 重复取值报错**：clap 拒绝未知 feature，重复（如 `deepmd,deepmd`）在构建前的校验层报错
- **语义**：翻转 `-DCP2K_USE_DEEPMD=ON` 与 `-DCP2K_USE_LIBTORCH=ON` 两个 cmake 旗标（上游语义成对）；工具链 `setup` 已把依赖导出进 `CMAKE_PREFIX_PATH`，无需提供额外路径或环境变量
- **OpenBLAS 补丁**：`deepmd` 预设渲染时在工具链安装前注入一个 RUN，把工具链 `install_openblas.sh` 两处 make 调用改为 `SMALL_MATRIX_OPT=0`——规避 OpenBLAS 0.3.33 的小矩阵批处理 GEMM 缺陷（deepmd 计算期 SIGSEGV；上游修复 PR #5980 只在 develop，v0.3.33 / v0.3.34 未含）。不带 `--features` 的所有臂渲染逐字节不变
- **tag 后缀**：`_deepmd`（见上节）

## 自由字符串说明（--version / --cpu）

clap 只对 `method` / `mpi` / `cuda` / `variant` / `features` 做取值白名单；`--version` 与 `--cpu` 是自由字符串，拼写错误要到 Dockerfile 合成或源码 clone 阶段才失败。命中捆绑 Dockerfile 用捆绑文件，未命中按模板现场合成。GUI 的版本 / CPU 下拉刻意收窄为已知配置（与 `forge2k list` 所列一致），从源头避免这类拼写错误。

## 退出码与取消

| 退出码 | 含义 |
|---|---|
| `0` | 构建成功；或"温和退出"——确认提示处回答否（Docker 未运行时的 `Continue anyway?`、构建前的 `Start build?`），以及 Docker 未安装时 |
| `1` | 构建失败（含构建进程异常退出 / 基础设施错误） |
| `130` | 构建被用户取消 |

- **取消词**：构建过程中输入 `q` / `quit` 或直接按回车即可取消（大小写不敏感）；取消会终止正在运行的 docker 进程。GUI 的 CANCEL BUILD 按钮等价
- **Docker 检查**：默认构建前探测 Docker 引擎；未安装时打印分平台安装指引后退出（0）；已安装未运行时询问 `Continue anyway?`。`--force` 跳过该检查

## 构建缓存（BuildKit cache mounts）

重复构建同版本镜像时，捆绑与合成 Dockerfile 通过 BuildKit cache mounts 复用下载：

| 下载向量 | cache id | 共享范围 |
|---|---|---|
| apt 包 | `apt-2404`（spack 系）/ `apt-2204`（toolchain 系） | 同基础发行版的全部构建 |
| CP2K 源码 | `cp2k-src-<版本>` | 同版本全部变体；重建时 `git fetch + reset + submodule update` 增量刷新 |
| Spack 包源码 | `spack-src` | 全部 spack 变体（含 CUDA） |
| toolchain tarballs | `toolchain-tarballs` | 全部 toolchain 变体（含 CUDA） |
| Spack 二进制包 | `spack-buildcache` | spack 系依赖编译产物（buildcache mirror，命中整包跳过） |
| ccache | `spack-ccache` | spack 系 C/C++ 对象（buildcache miss 时的二级加速） |

要点：

- cache mount 由 BuildKit 管理，**独立于镜像层缓存**：Dockerfile 变更导致的层失效不影响下载缓存；编译段仍会重跑
- **toolchain 系（v2023.2）不支持编译级缓存**（上游 `install_cp2k_toolchain.sh` 写死编译器绝对路径且 ccache 不支持 Fortran），仅享受下载段缓存——上游设计而非本工具限制
- **`--no-cache` 注意**：实测（Docker Desktop 29.x）`--no-cache` 构建中 cache mount 视图为空、不命中既有缓存——只想利用下载缓存时应通过修改 Dockerfile 制造层失效，而非 `--no-cache`
- toolchain 安装带自愈重试：下载中断留下的残缺 tar 包会被自动清理并重下
- 清空缓存：`docker builder prune --filter type=exec.cachemount`；缓存磁盘占用约 5-6 GB（以 `docker system df` 实测为准）
- 镜像 pull 加速是另一层，走 [mirror](mirror.md) 子命令

## 相关页面

- [spack 方法](../methods/spack.md) / [toolchain 方法](../methods/toolchain.md) / [native 方法](../methods/native.md)
- [list 参考](list.md)：查看全部预置配置
- [快速开始](../quick-start.md)
