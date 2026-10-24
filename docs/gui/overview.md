# GUI 完整指南

Forge2K 的图形界面与 CLI 完全对齐（对应关系逐项见[覆盖率矩阵](../coverage.md)）。本文按五个标签页逐一说明每个控件的行为及其对应的 CLI 选项。

启动方式：`forge2k gui`（别名 `forge2k g`），或直接运行不带参数的 `forge2k`——无子命令时默认进入 GUI。窗口默认 1024×720（最小 800×600），标题为 "Forge2K - CP2K Docker Image Builder"。

<!-- TODO(screenshot): 五 Tab 截图，补图流程见 .omz/runtime/forge2k-parity/results/T10.md -->

## 窗口总览

- **顶栏**：左侧为应用名与版本号；右侧为两枚状态灯——构建进行中显示 `● BUILDING`（橙色），Docker 引擎状态显示 `● Docker OK`（绿）或 `○ Docker`（红/未确认）。Docker 状态灯只在本次会话内探测过后才准确。
- **标签栏**：Build / Configs / System / Settings / About 五页，启动后默认停在 Build 页。
- **约定**：所有耗时操作（构建、BuildKit/存储探测、Docker 重启）都在后台线程执行，UI 不冻结；探测进行中按钮显示 "Checking…"/"⏳ Restarting…" 并自动禁用，防重复点击。

## Build 标签页

构建的主入口，左侧为配置面板（含高级选项区），右侧为实时构建日志。

### 基础配置（与 CLI build 选项一一对应）

| 控件 | 行为 | CLI 对应 |
|---|---|---|
| Build Method | 下拉：spack / toolchain / native。切换后版本、MPI、CPU、CUDA 的可选项联动收窄 | `-m/--method` |
| CP2K Version | 下拉，候选来自与 `forge2k list` 同源的配置扫描（启动时缓存，`⟳ Refresh` 手动重扫）。选 native 时换用 native 专属版本表（见下）。选中 synthesized 配置时显示黄色说明行 | `-v/--version` |
| MPI Implementation | 下拉：spack/native 可选 mpich、openmpi；toolchain 只允许 mpich。方法切换后失效的旧值自动回退 mpich | `--mpi` |
| CPU Target | 下拉，按方法收窄：spack 为 x86_64/cascadelake/haswell/skylake-avx512/generic；toolchain 仅 generic；native 仅 x86_64/generic。失效旧值自动回退 generic | `--cpu` |
| CUDA Support | 下拉，选项由构建侧支持矩阵给出：spack 仅 none（此时下拉置灰并提示），toolchain 与 native 可选 none/P100/V100。方法切换后失效旧值自动回退 none | `--cuda` |
| Binary Variant | 下拉：psmp / ssmp / pdbg / sdbg | `--variant` |
| Parallel Jobs (0 = auto) | 文本输入，`0` 或非数字自动解析为主机并行度（与 CLI 同语义） | `-j/--jobs` |
| Shared Memory Size | 文本输入，默认 `1g` | `--shm-size` |
| Custom Image Tag | 文本输入，留空自动生成（`cp2k/cp2k:{version}_{mpi}_{cpu}[_cuda_{GPU}]_{variant}`，master 用日期戳） | `-t/--tag` |

!!! note "版本与 CPU 为何是下拉而非自由输入"
    CLI 的 `-v/--version` 与 `--cpu` 是自由字符串（拼错要到 Dockerfile 合成或源码 clone 阶段才失败）；GUI 刻意收窄为已知配置（与 `forge2k list` 所列一致），从源头避免拼写错误。这是刻意行为，详见[覆盖率矩阵](../coverage.md)第 2/4 行。

**native 版本表**：GUI 的 native 版本下拉为 master / 2026.1 / 2025.2 / 2024.3，**刻意不含 2026.2**（上游 v2026.1 起删除根 Makefile，native release 臂仍走 make，选 2026.2 必败）。选中 native 时界面直接显示 "2026.2 has no verified native build matrix yet" 说明行。

### 高级选项区（折叠面板，CLI 对齐）

点击 "高级选项 / Advanced Options" 展开：

| 控件 | 行为 | CLI 对应 |
|---|---|---|
| Feature Presets (--features) | 勾选组，选项由构建侧注册表驱动（当前仅 `deepmd`）。不支持的构建臂整体置灰，并显示具体原因（见下方联动规则） | `--features` |
| Custom Dockerfile (optional, -f) | 文本路径输入，留空 = 捆绑/自动生成。**失焦或回车时**校验文件存在性，不存在显示红字；与 Feature Presets 互斥，同启时显示红字提示 | `-f/--dockerfile` |
| No cache (--no-cache) | 勾选后 `docker build` 带 `--no-cache` | `--no-cache` |
| Force build even if Docker is not running (--force) | 勾选后 START BUILD 跳过 Docker 运行预检（Docker 真不可用时构建本身仍可能失败） | `--force` |

高级选项（no-cache / force / features / dockerfile）与 Settings 页的镜像源草稿一起持久化到 `~/.forge2k/settings.json`，重启后保留——持久化细节见下文 [Settings 持久化](#settings-persistence)。

### 联动规则表

| 规则 | 行为 |
|---|---|
| features 支持臂 | 仅 toolchain 方法 + CP2K ≥2026.1 的 release-cmake 臂可用（master 例外）。spack → 置灰 + "Feature presets require the toolchain method"；native → 置灰 + "not supported by the native method"；master → 置灰 + "not available for master"；2026.1 以前的 make 臂 → 置灰 + "require CP2K v2026.1+"。点 START BUILD 时还有一道硬门，非法组合直接拒绝并写入日志，不会带病进构建层 |
| features × 自定义 Dockerfile | **互斥**：同时出现时输入框下方即时红字提示，START BUILD / Generate Only 均拒绝。原因：feature 预设只影响合成 Dockerfile，对现成文件无意义 |
| CUDA × 方法 | spack 仅 none；toolchain/native 为 none/P100/V100（master+CUDA 在 native 臂会被 Step 0 拒绝） |
| CPU × 方法 | toolchain 仅 generic；native 仅 x86_64/generic；spack 五值全开。切换方法后失效旧值自动回退 generic |
| MPI × 方法 | toolchain 仅 mpich；切换后失效旧值自动回退 mpich |
| force × Docker 预检 | START BUILD 先做 Docker 探测：引擎在跑、或勾了 Force，才进入构建；否则日志区写入 "Docker is not running. Please start Docker first."（等价 CLI 在此处询问 `Continue anyway?`，GUI 以勾选 Force 表达"继续"） |

### 构建流程与状态条语义

1. **START BUILD**（构建中自动变为 **⛔ CANCEL BUILD**）：先跑一次 Docker 探测与高级选项硬门，随后清空日志、在后台线程启动构建（native 直连主机，其余走 Docker），日志经通道实时流入右侧面板。
2. **日志流**：右侧 "Build Output" 面板滚动显示构建日志，`Auto-scroll` 勾选在构建中自动吸底，`Clear` 清空。颜色语义：错误行红、警告行黄、`✅/🚀/📋` 开头的行为绿、其余灰白。
3. **状态条三态**（构建结束显示，与 CLI 退出码等价）：
    - ✅ **绿** "Build completed successfully." —— 等价 CLI 退出码 `0`
    - ❌ **红** "Build failed."（附错误摘要） —— 等价 CLI 退出码 `1`
    - ⏹ **黄** "Build cancelled." —— 等价 CLI 退出码 `130`
4. **取消**：CLI 构建中输入 `q` / `quit` 或回车即取消；GUI 等价操作是点击 **CANCEL BUILD** 按钮——写入同一个取消标志，终止正在运行的 Docker/构建进程。取消不是错误：镜像 tag 只在成功构建后产生，取消不会留下坏镜像。
5. **Generate Only**：等价 CLI `--generate-only`。按当前配置解析或合成 Dockerfile，弹出浮动预览窗显示**文件完整内容**与**落盘路径**；合成文件写入 `%TEMP%/forge2k/`（与 CLI 同一副作用）。不启动构建、不需要 Docker 运行（Docker 停机时也能用）。构建进行中按钮禁用。高级选项的非法组合在预览窗内直接显示失败原因。

## Configs 标签页

CLI `forge2k list` 的 GUI 等价：预置配置全量浏览。

| 控件 | 行为 | CLI 对应 |
|---|---|---|
| Method 过滤下拉 | All / spack / toolchain（与 `list --method` 同枚举；native 不在 list 范围内，故亦无此过滤项） | `list --method` |
| Version 过滤下拉 | All + 当前 Method 过滤下扫描到的全部版本（与 `list --version` 同精确匹配语义） | `list --version` |
| `⟳ Refresh` | 重扫捆绑 dockerfiles/ 目录，同时刷新 Build 页的版本候选 | — |
| 配置列表 | 每条含：`#N` 描述行（synthesized 配置带黄色标记）、Method 行（方法/版本/基础镜像）、Specs 行（MPI/CPU/CUDA/变体）、等效 `forge2k build ...` 命令行 | `list` 的四行输出 |
| Copy 按钮 | 复制该配置的完整 build 命令到剪贴板，过滤行旁显示 "✓ Copied to clipboard" 反馈 | — |

列表顺序与 CLI 一致（方法 → 版本降序 → mpi → cpu → cuda）；空结果时显示与 CLI 相同的 "No configurations match the given filters."。

## System 标签页

系统诊断，与 CLI `forge2k check` 五项**同源**（共用同一批探测函数），四张卡片按需手动探测：

| 卡片 | 内容 | 操作 | CLI check 对应项 |
|---|---|---|---|
| 🐳 Docker Engine | 安装/运行状态与版本；未检测时显示 "Not checked" | Check Docker | Docker Engine |
| 🧱 Docker BuildKit | buildx 插件探测；未检出时给出黄色提示（构建回退 legacy builder）与 Linux 安装命令 | Check BuildKit | Docker BuildKit |
| 🌐 Docker Registry | 实测 `docker pull alpine:latest` 的连通性（✅ Fast / ⚠️ Slow / ❌ Blocked / ❓），Blocked 且未配镜像源时提示去 Settings 自动探测；已配置时显示当前 mirror | Test Registry | Docker Registry + 当前 mirror |
| 💾 Docker Storage | Docker 存储根目录 + 所在卷剩余空间（Windows 走 PowerShell CIM 探测；根目录取不到时磁盘探测回退所有固定盘） | Check Storage | Docker Storage |

另有 Docker 安装指引卡片（打印分平台安装指南）与 Quick Tips（构建耗时 1–3 小时、建议 50GB 空闲磁盘、WSL2 需开启 Docker Desktop WSL integration 等）。

## Settings 标签页

registry 镜像源管理，与 CLI `mirror` 子命令同源：

| 控件 | 行为 | CLI 对应 |
|---|---|---|
| Mirror URL 输入框 | 草稿内容即时持久化（失焦/回车提交），重启后保留 | — |
| Apply | 校验 URL 以 `http://`/`https://` 开头后写入 daemon.json 的 `registry-mirrors` 键（**整体覆盖为单元素数组**，其余键保留），成功提示需重启 Docker 生效 | `forge2k mirror <url>` |
| 🔍 Auto-detect Best Mirror | 按内置已知镜像列表逐个探测 `/v2/_catalog`（5 秒超时），取第一个可用者填入输入框 | `forge2k mirror`（省略 url） |
| 🗑 Remove Mirror | 从 daemon.json 删除 `registry-mirrors` 键（其余键保留；本就没配置时静默成功） | `forge2k mirror -r` |
| 🐳 Restart Docker | 重启 Docker 使配置生效（平台差异见下文） | `forge2k mirror --restart` |
| Known Public Mirrors | 内置 5 个已知镜像（USTC / 腾讯云 / DaoCloud / Docker CN / Timeweb），每行 `Use` 按钮填入输入框 | CLI 自动探测失败时打印同一列表 |

镜像配置写入/删除**不会**写入 settings.json 的持久化字段——`current_mirror` 是 Docker daemon（daemon.json）的状态，settings.json 只保存输入框草稿，二者不双写。

### Restart Docker 平台差异

- **Windows**：按钮直接可用。点击后在**后台线程**执行——强制结束 Docker Desktop、等待 3 秒、重新拉起并轮询引擎就绪（最长 60 秒），期间按钮显示 "⏳ Restarting…"；构建进行中按钮禁用（重启会杀掉进行中的构建）。完成后提示去 System 页用 Check Docker 确认引擎恢复。
- **Linux / macOS**：**不直接执行**。GUI 进程没有终端，`sudo systemctl` 的密码提示无法应答，因此界面只显示等价的手动命令（`sudo systemctl restart docker`，rootless 为 `systemctl --user restart docker`）及原因说明。

## Settings 持久化 { #settings-persistence }

所有跨重启保留的 GUI 输入存在 `<home>/.forge2k/settings.json`（home 取 `USERPROFILE`，回退 `HOME`）：

| 字段 | 来源 |
|---|---|
| `mirror_input` | Settings 页镜像 URL 输入框草稿 |
| `no_cache` | Build 页高级选项 No cache |
| `force` | Build 页高级选项 Force |
| `features` | Build 页高级选项 Feature Presets 勾选 |
| `dockerfile` | Build 页高级选项 Dockerfile 路径草稿（原样保存） |

规则：字段全部 `#[serde(default)]`，旧版 settings.json（只有 `mirror_input`）可无损加载；加载任何失败（文件缺失/损坏）静默回退默认值；加载时过滤掉注册表之外的 feature 值，手工编辑 settings.json 无法注入未知预设。写盘只发生在真实变更点（勾选、失焦提交、Apply、Use），帧循环绝不写盘。删除该文件即可恢复全部默认。

## About 标签页

项目简介、功能清单、版本号与已知限制（toolchain CUDA 变体 e2e 延后、2026.1/2026.2/master 走合成路径、Windows release CLI 无 stdout 等）。

## 相关页面

- 遇到问题？见[故障排查](../troubleshooting.md)与 [FAQ](../faq.md)。
- GUI 与 CLI 的逐项对应关系见[覆盖率矩阵](../coverage.md)。
- 参与开发见[开发指南](../development.md)。
