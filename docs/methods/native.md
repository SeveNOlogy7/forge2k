# native 方法

`forge2k build -m native`。**不经 Docker**，直接在主机上克隆 CP2K 源码、安装工具链依赖并构建。**Linux / WSL 专用——Windows 原生主机不可用**。

!!! warning "Windows 不可用"

    native 流程依赖 bash、apt 与 Linux 路径（如 `/opt/cp2k_build`）。Windows 用户请在 WSL 内使用，或在 Docker 里走 [toolchain](toolchain.md) / [spack](spack.md) 方法。

## 6 组合白名单矩阵

native 构建前先做组合校验，只支持以下 **6 种组合**（其余组合显式报错并提示改用 Docker 方法）：

| | CUDA none | CUDA P100 | CUDA V100 |
|---|---|---|---|
| **CPU generic** | 支持 | 支持 | 支持 |
| **CPU x86_64** | 支持 | 支持 | 支持 |

6 组合全开适用于 make 臂版本（2025.2 / 2024.3）；cmake 臂版本（master / 2026.1 / 2026.2）仅支持 CUDA none 两列。

- MPI 锁定 **system mpich**、变体锁定 **psmp**（与捆绑矩阵对齐），其他取值被拒绝
- **cmake 臂（master、2026.1、2026.2）锁 cuda=none**：这些版本走 CMake+Ninja 路径，该路径不带 CUDA 旗标，`cmake 臂版本 + CUDA` 组合被显式拒绝
- CPU 取 `x86_64` 时，传给工具链脚本前会**自动翻译为 `generic`**（脚本的 CPU 词表不含字面量 `x86_64`：OpenBLAS getarch 与 gfortran `-mtune` 都会拒绝）

## 依赖自动补装

**Step 1** 逐个探测 9 个工具（`<tool> --version`）：

```text
gcc, g++, gfortran, git, make, cmake, wget, bunzip2, mpicc
```

有缺失时自动尝试 apt 补装（`apt-get update` 失败则回退 `sudo apt-get update`；安装同理）：

- **探测名 → 包名映射**：`mpicc` → `libmpich-dev`，`bunzip2` → `bzip2`（其余探测名即包名）
- **固定追加的 15 个 extras**：`autoconf`、`autogen`、`automake`、`libtool`、`libtool-bin`、`ninja-build`、`pkg-config`、`python3-dev`、`python3-pip`、`xxd`、`xz-utils`、`zlib1g-dev`、`mpich`、`libmpich-dev`、`bzip2`（与捆绑 toolchain Dockerfile 的 MPI 供给一致，`libmpich-dev` 提供 `mpicc`、`bzip2` 提供 `bunzip2`）

全部去重后一次 `apt-get install -qq --no-install-recommends -y` 安装。

## 构建流程（6 步）

| 步骤 | 内容 |
|---|---|
| Step 1 | 检查系统前提（上节的 9 工具探测与自动补装） |
| Step 2 | 建立工作目录 **`/opt/cp2k_build`**（源码位于 `/opt/cp2k_build/cp2k`） |
| Step 3 | 克隆 CP2K 源码（`--recursive`；非 master 检出 `support/v<版本>` 分支；目录已存在则 `git pull` 增量更新） |
| Step 4 | 安装 CP2K 工具链依赖（`install_cp2k_toolchain.sh --install-all`，预计 30-60 分钟；参数与 [toolchain 方法](toolchain.md)对齐：`--enable-cuda`、`--gpu-ver`、`--target-cpu`、`--with-cusolvermp=no`、`--with-gcc=system`、`--with-mpich=system`；固定 `UCX_TLS=tcp,self,sm` 规避 UCX InfiniBand 探测误报） |
| Step 5 | 构建 CP2K：**master 与 2026.1 / 2026.2 走 CMake + Ninja**（≥2026.1 cmake 臂，source 工具链 setup 后 `cmake -GNinja` + `ninja` + `cmake --install`）；**2025.2 / 2024.3 走 make**（拷贝工具链生成的 `<local|local_cuda>.psmp` arch 文件后 `make -j <jobs> ARCH=<arch> VERSION=psmp`） |
| Step 6 | 验证安装 |

## 验证与 PATH 提示

Step 6 检查产物二进制并打印大小与 PATH 提示：

- cmake 路径（master、2026.1、2026.2）：`/opt/cp2k/install/bin/cp2k.psmp`
- make 路径：`/opt/cp2k_build/cp2k/exe/<local|local_cuda>/cp2k.psmp`

```text
🎉 To use CP2K, add to your PATH:
   export PATH=/opt/cp2k/install/bin:$PATH
```

## 已知限制

- **构建臂分界与 toolchain Docker 路径不同**：native 按 `master || >= 2026.1` 分派——master / 2026.1 / 2026.2 走 CMake+Ninja（cmake 迁移已落地；GUI native 版本下拉已恢复五项 master / 2026.1 / 2026.2 / 2025.2 / 2024.3），2025.2 / 2024.3 走 make
- 已在 GitHub Actions ubuntu runner 上验证 x86_64 / generic 两组合全链路构建 + 产物 `--version` 冒烟，并完成 `native_2026.2_x86_64_none` e2e（run 41454929016：cmake 臂产物路径断言 `/opt/cp2k/install/bin/cp2k.psmp` + smoke `grep 'CP2K version 2026.2'` 版本硬门）；master 与 2026.1 走同一 cmake 臂代码路径但无独立 e2e run；CUDA 两组合为构建面支持，未经镜像级验证（runner 无 NVIDIA 驱动 / CUDA toolkit）
- **cmake 臂的 feature 面与旧行为有差异**：master 的 native cmake 臂同样启用 LIBTORCH/OPENPMD=OFF（对齐 toolchain release-cmake 全套旗标；旧 master cmake 臂未带这两项 OFF），master 依赖面以 `execute.rs` 注释为准
- cmake 臂（master、2026.1、2026.2）+ CUDA 组合被显式拒绝（见白名单矩阵）

## 典型命令

```bash
forge2k build -m native -v 2025.2 --cpu generic          # 主机直连构建
forge2k build -m native -v 2026.2 --cpu x86_64           # cmake 臂（e2e 已验证）
forge2k build -m native -v master                        # master，CMake+Ninja
```

## 相关页面

- [build 参考](../cli/build.md)：全部选项与退出码（构建过程中同样支持 `q` / `quit` / 回车取消）
- [toolchain 方法](toolchain.md)：工具链参数语义
