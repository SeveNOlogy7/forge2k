# toolchain 方法

`forge2k build -m toolchain`。基于 CP2K 官方工具链脚本在 Docker 内构建，CPU 基础镜像 `ubuntu:22.04`，CUDA 变体基于 `nvidia/cuda:12.2.0-devel-ubuntu22.04`。支持 v2023.2 起的版本与 master 分支。

## 构建原理

两段式 Dockerfile（多阶段构建）：

1. **构建阶段**（`build`）：安装基础编译包后，克隆 CP2K 源码，运行官方 `install_cp2k_toolchain.sh --install-all` 安装全部依赖（带 3 次自愈重试：失败清理 build 目录后重下重跑），随后构建 CP2K 本体
2. **安装阶段**（`install`）：只拷贝 CP2K 产物、基准数据（`/opt/cp2k/data`）与工具链安装树，创建 entrypoint（source 工具链 `setup` + 设置 `OMP_STACKSIZE`）

工具链脚本关键参数（由合成模板 / 捆绑 Dockerfile 注入）：

- `--install-all`：安装全量依赖集
- `--enable-cuda=no`，或 CUDA 变体的 `--enable-cuda=yes --gpu-ver=<P100|V100> --with-libtorch=no`
- `--target-cpu=<CPU>`、`--with-cusolvermp=no`、`--with-gcc=system`、`--with-mpich=system`

## 版本与构建臂（make / cmake）

| 版本 | 构建臂 | 说明 |
|---|---|---|
| v2023.2（以及 2025.2 等旧 release） | `make` + arch 文件 | 上游 v2025.2 是**最后一个仍带根 Makefile / arch 树的 release**；工具链生成 `local.psmp` arch 文件后 `make ARCH=local VERSION=psmp` |
| v2026.1 / v2026.2 | `cmake -GNinja`（release-cmake 臂） | 上游 ≥2026.1 已删除根 Makefile / arch 树，改走 CMake + Ninja；CUDA 变体追加 `-DCP2K_USE_ACCEL=CUDA -DCP2K_WITH_GPU=<GPU> -DCMAKE_CUDA_ARCHITECTURES=<sm>` 三旗标 |
| master | `cmake -GNinja` | 同上（字节冻结的回归守护臂） |

CUDA 产物目录：CPU 变体 arch 名为 `local`，CUDA 变体为 `local_cuda`。

## CPU 目标：预置配置仅 generic

捆绑与广告的 toolchain 配置（见 [list](../cli/list.md)）**全部为 `--cpu generic`**：

- `generic` 是可移植的基线目标，对所有 x86_64 主机有效
- 字面量 `x86_64` 会被工具链脚本内的 OpenBLAS getarch（`TARGET not supported`）与 gfortran（`bad value for -mtune=`）拒绝——native 路径因此在传参前把 `x86_64` 翻译为 `generic`（见 [native 方法](native.md)）
- GUI 的 CPU 下拉对 toolchain 方法**刻意收窄为 generic**，避免拼出不可用目标；CLI 的 `--cpu` 是自由字符串（见 [build 参考](../cli/build.md)）

## CUDA：P100 / V100（sm_60 / sm_70）

CUDA 变体按上游 make_cp2k.sh 的 GPU 表映射计算能力：**P100 → sm_60，V100 → sm_70**。P100/V100 变体对 v2026.2 同样可用（2026-09-29 已在 GitHub Actions 完成全量构建验证）。注意容器内运行 `cp2k --version` 需要 NVIDIA 驱动——无 GPU 主机因缺 `libcuda.so.1` 无法运行，属环境边界而非镜像缺陷。

## 功能预设 --features deepmd

`deepmd` 预设仅在 toolchain 方法的 **release-cmake 臂**（版本 ≥ 2026.1 且非 master）生效：

- 翻转 `-DCP2K_USE_DEEPMD=ON` / `-DCP2K_USE_LIBTORCH=ON`（上游语义成对），tag 追加 `_deepmd` 后缀
- **OpenBLAS SMALL_MATRIX_OPT=0 补丁的背景**：工具链固定的 OpenBLAS 0.3.33 在 x86_64 上默认启用 SMALL_MATRIX_OPT，其小矩阵 `gemm_batch` 调度把表偏移（实测 `0x428`）当函数指针传给工作线程，deepmd NNP 求值的首个小矩阵批处理 GEMM 即 SIGSEGV；上游修复（OpenBLAS PR #5980）只在 develop，v0.3.33 / v0.3.34 均未包含。`deepmd` 预设因此在工具链安装**前**注入一个 RUN，把 `install_openblas.sh` 两处 make 调用改写为 `SMALL_MATRIX_OPT=0`（make 命令行变量覆盖 makefile 默认值并经 MAKEFLAGS 传入递归子 make），镜像以关闭该优化的 OpenBLAS 构建。补丁后的计算级测试已通过（能量值与上游参考值逐位一致）
- 不带 `--features` 的所有臂渲染逐字节不变；完整约束（互斥、fail-loud）见 [build 参考](../cli/build.md)

## v2026.2 的两处 deliberate workaround

- **dbcsr config 版本补丁**：release-cmake 臂内以 sed 修正 tarball 构建产出的 `DBCSRConfigVersion.cmake`——其 `git describe` 爬到外层 cp2k tag 产生畸形版本串（`2026.2.v2026.2v2026.2`），被 SameMajorVersion 语义拒绝导致 `find_package(DBCSR 2.8)` 失败
- **openPMD OFF**：工具链 `--install-all` 依赖集不含 openPMD，经 41 项组件对照矩阵确认其为唯一额外 OFF，故追加 `-DCP2K_USE_OPENPMD=OFF`

## 典型命令

```bash
forge2k build -m toolchain -v 2023.2 --mpi mpich --cpu generic --variant psmp
forge2k build -m toolchain -v 2026.2 --cuda P100          # CUDA 变体
forge2k build -m toolchain -v 2026.2 --features deepmd    # DeePMD 支持
forge2k build -m toolchain -v master                      # master，cmake 臂
```

## 相关页面

- [build 参考](../cli/build.md)：全部选项、`--features` 约束、构建缓存
- [spack 方法](spack.md) / [native 方法](native.md)
