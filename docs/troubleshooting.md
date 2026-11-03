# 故障排查

每条按 **症状 → 原因 → 解决** 组织。CLI 与 GUI 的行为差异在各条内分别说明。

## Docker 未运行 / 未安装

**症状**：构建前提示 Docker 引擎不可用；CLI 输出 "Docker Engine is installed but not running!" 或 "Docker Engine is not installed!"；GUI 点 START BUILD 后日志区显示 "Docker is not running. Please start Docker first."。

**原因**：`spack` / `toolchain` 方法需要 Docker daemon；构建前有引擎预检（CLI 与 GUI 均有）。Docker **已安装但 daemon 没起来**是最常见的情形。

**解决**：

1. 启动 Docker：Windows/macOS 打开 Docker Desktop 并等其就绪；Linux 执行 `sudo systemctl start docker`。
2. 用 `forge2k check`（CLI）或 System 标签页的 Check Docker（GUI）确认状态，未安装时两者都会打印分平台安装指引。
3. **CLI 的两种处理方式**：
    - 已安装但未运行：询问 `Continue anyway? [Y/n]`——回答是则带病继续（构建在真正需要 daemon 时仍会失败）；回答否则温和退出（退出码 0）。
    - 根本未安装：直接结束并打印安装指引，不提供"继续"。
    - 想跳过这道询问，用 `forge2k build --force`（跳过 Docker 引擎检查）。
4. **GUI 的处理方式**：没有"继续"询问——Docker 不在运行时 START BUILD 被直接拦截；勾选高级选项区的 **Force build even if Docker is not running (--force)** 即等价 CLI `--force`。Generate Only 不需要 Docker，停机时也能预览 Dockerfile。

## Registry Blocked（registry 连通性被阻断）

**症状**：`forge2k check` 的 Docker Registry 段显示 "Registry access appears blocked"；GUI System 页 Registry 卡片显示 `❌ Blocked - <原因>`；`docker pull` 超时、连接拒绝或无路由。

**原因**：check 会实际执行 `docker pull alpine:latest` 实测连通性——10 秒内成功为 Good，超过 10 秒为 Slow（显示耗时），stderr 含 `timeout` / `refused` / `no route` 判为 Blocked，其余为 Unknown。受限网络直接拉 Docker Hub 常见超时。

**解决**：配置 registry 镜像源（见下一条"镜像拉取慢"），配置后重启 Docker 生效；再用 check 复测。

## 镜像拉取慢

**症状**：构建卡在拉取基础镜像（`ubuntu:24.04`、`nvidia/cuda:...` 等）；Registry 检测显示 Slow。

**原因**：到 Docker Hub 的网络路径慢或不稳定。

**解决**：配置 registry 镜像源。

- **CLI**：
    ```bash
    forge2k mirror                                        # 省略 url：自动探测内置镜像列表
    forge2k mirror https://docker.mirrors.ustc.edu.cn     # 手动指定（必须 http(s):// 开头）
    forge2k mirror https://docker.mirrors.ustc.edu.cn --restart   # 写入并重启 Docker
    ```
- **GUI**：Settings 标签页 → 填 Mirror URL 后 Apply，或点 Auto-detect Best Mirror，再点 Restart Docker。
- **已知镜像**（共 5 个，自动探测按此顺序）：USTC、腾讯云（mirror.ccs.tencentyun.com）、DaoCloud、Docker CN、Timeweb Cloud。自动探测对每个候选请求 `/v2/_catalog`（5 秒超时，HTTP 200 或 401 视为可用），全部失败时 CLI 会打印该列表提示手动提供。
- **注意**：镜像源写入 daemon.json（Windows 为 `%USERPROFILE%\.docker\daemon.json`，Linux/macOS 为 `/etc/docker/daemon.json`）的 `registry-mirrors` 键，**该键被整体覆盖为单元素数组**；daemon.json 中其余键原样保留。需要多个镜像源须手动编辑该文件。写入后必须重启 Docker 才生效。

## native 构建报工具缺失

**症状**：native 构建日志出现 "Missing: gcc, gfortran, ..." 与 "Attempting to install missing packages..."。

**原因**：native 方法直接在主机上编译，Step 1 探测 9 个工具：`gcc`、`g++`、`gfortran`、`git`、`make`、`cmake`、`wget`、`bunzip2`、`mpicc`。

**解决**：

- **自动补装**：Forge2K 会把缺失工具映射为 Ubuntu 包名（`mpicc`→`libmpich-dev`、`bunzip2`→`bzip2`，其余同名），拼接一份固定的补充包清单（autoconf/automake/libtool/ninja-build/pkg-config/python3-dev/zlib1g-dev/mpich/libmpich-dev/bzip2 等），先试 `apt-get install`，失败自动改用 `sudo apt-get install`。整个过程在构建日志可见。
- **手动补装**：自动安装失败（如非 Debian/Ubuntu 系发行版）时，按日志里的缺失清单自行安装，例如：
    ```bash
    sudo apt-get install -y gcc g++ gfortran git make cmake wget bzip2 mpich libmpich-dev
    ```
- **前提**：native 方法仅 Linux/WSL 可用（构建脚本依赖 bash/apt/`/opt/cp2k_build` 路径），Windows 主机请用 Docker 方法或在 WSL 内运行。

## WSL 环境的特殊行为

**症状**：在 WSL 发行版内运行时安装指引/磁盘探测行为与原生 Linux 不同；或磁盘剩余空间显示 "Disk space unknown"。

**原因与行为**：

- **WSL 检测**：工具通过 `/etc/wsl.conf` 存在或 `WSL_DISTRO_NAME` 环境变量识别 WSL 环境，并据此打印对应的安装指引——既给 WSL 内直装 Docker 的步骤，也给 Docker Desktop WSL 2 后端的启用步骤（Windows 装 Docker Desktop → Settings → Resources → WSL Integration → 启用对应发行版）。
- **Docker Desktop WSL 后端的存储探测**：该后端的 `DockerRootDir` 是 `/var/lib/docker`——没有 Windows 盘符。Windows 侧磁盘探测优先取根目录所在盘符，取不到盘符时**回退枚举所有固定磁盘**（DriveType=3），保证仍有输出。
- **WSL 内（非 Windows 二进制）的探测**：`docker info` 取不到根目录（daemon 不可达）时，磁盘项直接判 Unknown 并说明原因，不猜。
- GUI Quick Tips 亦常驻提醒：WSL2 需在 Docker Desktop 中启用对应发行版的 integration。

## Dockerfile 找不到

**症状**：构建或 Generate Only 报 "Custom Dockerfile not found: <路径>"；GUI 高级选项输入路径失焦后显示红字 "❌ File not found: <路径>"。

**原因**：Dockerfile 按 `resolve_dockerfile` 解析：显式 `-f` 优先（必须存在），否则在搜索路径里找捆绑文件，都未命中则现场合成。

**解决**：

1. **自定义路径**：检查路径拼写——CLI 相对当前工作目录解析；GUI 建议填绝对路径（失焦即校验）。
2. **捆绑文件为何没找到**：捆绑 Dockerfile 依序在三个位置搜索：**可执行文件旁的 `dockerfiles/`** → **`./dockerfiles/`**（工作目录）→ **`.`**。zip 发行版里二进制与 dockerfiles 不在同目录、或从别的工作目录启动时，会落到后续路径；三个位置都没有对应捆绑文件时按模板合成（日志显式提示 synthesized），这本身不是错误。
3. 2026.1 / 2026.2 / master 本就没有捆绑 Dockerfile，走合成路径，属预期行为。

## features 与 -f/--dockerfile 互斥报错

**症状**：构建/预览报错 "--features cannot be combined with an explicit Dockerfile (-f/--dockerfile)..."；GUI 中两个选项同启时输入框下方即时红字。

**原因**：feature 预设（如 `deepmd`）只影响**合成** Dockerfile 的构建步骤；指定现成 Dockerfile 时预设会被静默丢弃，因此组合被禁止而不是静默忽略。

**解决**：二选一——取消 Feature Presets 勾选，或清空 Dockerfile 路径让 Forge2K 合成（预设生效的合成臂为 toolchain + CP2K ≥2026.1 非 master，其余臂携带 `--features` 也会直接报错并说明要求）。

## deepmd 构建或运行失败（SMALL_MATRIX_OPT）

**症状**：带 `--features deepmd` 的镜像构建成功，但运行 deepmd 计算时 SIGSEGV；gdb 栈落在 OpenBLAS 的 `gemm_batch_thread.c`。

**原因**：工具链钉死的 OpenBLAS 0.3.33 在 x86_64 默认开启 `SMALL_MATRIX_OPT`，其小矩阵批量 GEMM 调度把表偏移当函数指针传递（上游缺陷；修复 PR #5980 只进 develop，v0.3.33/v0.3.34 均未含），CP2K deepmd NNP 求值的第一个小批量 GEMM 即触发跳转崩溃。

**解决**：**升级/使用当前版本的 deepmd preset 即可**——渲染时会在工具链安装前注入一个 RUN，把 `install_openblas.sh` 的两处 make 调用改为 `SMALL_MATRIX_OPT=0`（make 命令行变量覆盖 makefile 默认值，并经 MAKEFLAGS 传入递归子 make），镜像以关闭该优化的 OpenBLAS 构建。该修复已经过计算级验证：e2e run 39286488384 中 `deepmd-water` np1/np2 均 rc=0，能量与上游参考值逐位一致。不带 `--features` 的所有臂渲染逐字节不变（无此补丁）。

## spack 2026.2 / master：构建臂分界与受支持组合 { #spack-2026-2 }

**现状**：spack 方法**支持** v2026.2 与 master。上游 support/v2026.2 重排了 `tools/spack` 目录（`cp2k_dev_repo` → `spack_repo/cp2k_dev`、deps yaml 改名、spack 包本体移出上游仓库），旧 spack 布局模板对 2026.2+ 已失效，因此构建臂按版本分界：**2025.2 / 2026.1 走旧 spack 布局臂**（引用 `tools/spack` 内部路径）；**2026.2 / master 委托上游仓库根 `make_cp2k.sh` 一体化脚本**（自装 Spack、解析依赖、CMake+Ninja 构建，不引用 `tools/spack` 内部路径，上游再重排不受影响）。

**如何确认你的组合受支持**：`forge2k list` 与 GUI Configs 页按注册渲染——spack 2026.2 注册的是单组合 **mpich/x86_64/psmp**（与 e2e 验证面 `spack_2026.2_x86_64` 对齐，不广告未验证组合；CLI 自由字符串可合成其他组合但未经验证）。合成路径可先用 `--generate-only` 检查 Dockerfile 再实际构建。

**注意**：2026.2 / master 委托臂未接入 Spack buildcache（上游脚本自管其缓存），冷构建为**小时级**——构建长时间停在依赖编译属预期，不是卡死。

## 其他

- **退出码 130 / 取消行为**：见 [FAQ](faq.md#exit-code-130)。
- GUI 控件与 CLI 选项的逐项对应：见[覆盖率矩阵](coverage.md)。
