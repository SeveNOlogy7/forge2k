# 覆盖率矩阵（CLI ↔ GUI）

本页是 UI/CLI 功能对齐的**覆盖率矩阵终稿**（计划 F2 验收产物）：每行一个 CLI 功能点，左侧给出 CLI 侧代码位置，右侧给出 GUI 侧真实代码位置或等价性声明。

- **代码基线**：main @ `1388ae8`（2026-10-24 核对；src/ 自 `9b9ebc3` 起未变，行号可直接对照 `src/main.rs` 与 `src/gui.rs`）。
- **判据**：全部覆盖行 ✅（双侧 file:line），零 ❌/⚠️ 残留；N-A 声明行不计覆盖率但必须成文。
- **`--version` / `--cpu` 收窄说明**：CLI 侧二者为自由字符串（clap 不做取值校验，拼错在合成/clone 阶段才失败）；GUI 下拉**刻意收窄为已知配置**（与 `forge2k list` 所列一致），从源头避免拼写错误。属合法的刻意差异，不判缺口（见第 2/4 行）。

## 覆盖率矩阵

| # | CLI 功能点 | CLI 侧 file:line | GUI 侧 file:line（或等价性声明） | 状态 |
|---|---|---|---|---|
| 1 | build `-m/--method` | `src/main.rs:47`（三值枚举） | Build Method 下拉 `src/gui.rs:814-828`（同三值） | ✅ |
| 2 | build `-v/--version` | `src/main.rs:51`（自由字符串） | CP2K Version 下拉 `src/gui.rs:839-872`（同源扫描 `build::list_available_configs`，启动缓存 `src/gui.rs:187`；native 专属列表 `src/gui.rs:2243-2248`）。GUI **刻意收窄为已知配置**（与 list 一致） | ✅ |
| 3 | build `--mpi` | `src/main.rs:55`（mpich/openmpi） | MPI 下拉 `src/gui.rs:893-914`（toolchain 收窄 mpich `src/gui.rs:898-907`，旧值回退） | ✅ |
| 4 | build `--cpu` | `src/main.rs:59`（自由字符串） | CPU Target 下拉 `src/gui.rs:917-954`（按方法白名单 `src/gui.rs:923-937`，失效回退 generic `src/gui.rs:938-947`）。GUI **刻意收窄为已知配置** | ✅ |
| 5 | build `--cuda` | `src/main.rs:63`（none/P100/V100） | CUDA 下拉 `src/gui.rs:957-991`（选项=构建侧支持矩阵 `build::cuda_options_for_method`，`src/build/execute.rs:35-43`，经 `src/gui.rs:967`；不支持臂禁用+提示 `src/gui.rs:973-991`） | ✅ |
| 6 | build `--variant` | `src/main.rs:67`（psmp/ssmp/pdbg/sdbg） | Binary Variant 下拉 `src/gui.rs:995-1007`（同四值） | ✅ |
| 7 | build `--features` | `src/main.rs:71-73`（KNOWN_FEATURES 白名单，`src/build/generate.rs:13`） | 高级选项 Feature Presets 勾选 `src/gui.rs:1060-1100`（注册表驱动；支持臂联动置灰 `src/gui.rs:1066-1082`，原因文案 `src/gui.rs:1083-1100`；START BUILD 硬门 `src/gui.rs:251-265`） | ✅ |
| 8 | build `-j/--jobs` | `src/main.rs:76`（0=自动，`src/main.rs:294`） | Parallel Jobs 输入 `src/gui.rs:1010-1016`（0/非数字→available_parallelism，`src/gui.rs:210-218`，同语义） | ✅ |
| 9 | build `-t/--tag` | `src/main.rs:80-81` | Custom Image Tag 输入 `src/gui.rs:1029-1038`（空=自动生成 tag，映射 `src/gui.rs:228-232`） | ✅ |
| 10 | build `--no-cache` | `src/main.rs:84-85`（传入 docker args `src/build/mod.rs:160-162`） | 高级选项 No cache 勾选 `src/gui.rs:1143-1148`（映射 `src/gui.rs:233`） | ✅ |
| 11 | build `-f/--dockerfile` | `src/main.rs:88-89` | 高级选项 Custom Dockerfile 输入 `src/gui.rs:1109-1139`（失焦存在性校验 `src/gui.rs:1116-1127`；映射 `src/gui.rs:234-238`） | ✅ |
| 12 | build `--shm-size` | `src/main.rs:92-93`（默认 1g） | Shared Memory Size 输入 `src/gui.rs:1019-1025`（默认 1g `src/gui.rs:171`） | ✅ |
| 13 | build `--force` | `src/main.rs:96-97`（跳过 Docker 预检 `src/main.rs:263`） | 高级选项 Force 勾选 `src/gui.rs:1149-1157`（START BUILD 跳过预检 `src/gui.rs:1186-1205`） | ✅ |
| 14 | build `--generate-only` | `src/main.rs:102-103`（跳过预检 `src/main.rs:263`；打印路径 `src/main.rs:322-326`） | Generate Only 按钮 `src/gui.rs:1214-1229` → `generate_only` `src/gui.rs:387-453` → 浮动预览窗 `src/gui.rs:727-795`（同一 `resolve_dockerfile` 内核 `src/gui.rs:407-424`；合成写 `%TEMP%/forge2k/` 同副作用） | ✅ |
| 15 | `list` 全量浏览 | `src/main.rs:414-499`（`cmd_list`） | Configs 标签页 `src/gui.rs:2050-2228`（同源 `build::list_available_configs` `src/build/catalog.rs:155-221`；含 synthesized 标记、Specs、等效命令 Copy `src/gui.rs:2210-2222`） | ✅ |
| 16 | `list --method` 过滤 | `src/main.rs:110-111`（spack/toolchain 枚举）+ 过滤逻辑 `src/main.rs:417-427` | Configs Method 过滤下拉 `src/gui.rs:2060-2076`（同枚举，无 All 之外的额外项） | ✅ |
| 17 | `list --version` 过滤 | `src/main.rs:114-115` + 过滤逻辑 `src/main.rs:417-427` | Configs Version 过滤下拉 `src/gui.rs:2079-2105`（候选=当前 method 扫描版本 `src/gui.rs:2087-2097`，精确匹配同语义） | ✅ |
| 18 | check：Docker Engine | `src/main.rs:524-545`（`docker version` 探测） | System 页 Docker Engine 卡片 `src/gui.rs:1356-1411`（Check Docker 按钮 `src/gui.rs:1401-1405` → `src/gui.rs:455-471` → 同一 `build::check_docker` `src/build/docker.rs:19-43`） | ✅ |
| 19 | check：BuildKit | `src/main.rs:549-564`（`docker buildx version`） | System 页 BuildKit 卡片 `src/gui.rs:1415-1510`（后台探测 `src/gui.rs:1462-1474` → 同一 `build::check_buildkit` `src/build/docker.rs:143-156`） | ✅ |
| 20 | check：Registry 连通性 | `src/main.rs:568-592`（实测 `docker pull alpine`） | System 页 Registry 卡片 `src/gui.rs:1514-1541`（Test Registry `src/gui.rs:1528-1532` → `src/gui.rs:473-492` → 同一 `build::check_registry` `src/build/registry.rs:19-55`） | ✅ |
| 21 | check：当前 mirror 显示 | `src/main.rs:594-597`（daemon.json 首条目） | System 页镜像行 `src/gui.rs:1535-1540` + Settings 页当前镜像 `src/gui.rs:1917-1922`（同一读取 `build::get_registry_mirror` `src/build/registry.rs:179-194`） | ✅ |
| 22 | check：存储根目录 + 磁盘剩余 | `src/main.rs:600-628`（共享探针；Windows 走 PowerShell CIM） | System 页 Docker Storage 卡片 `src/gui.rs:1545-1653`（`src/gui.rs:1607` → 同一 `build::check_docker_storage` `src/build/docker.rs:211-222`） | ✅ |
| 23 | `mirror <url>` 写入 | `src/main.rs:684-702`（http(s) 校验 `src/main.rs:684-690`；写 daemon.json `src/build/registry.rs:131-154`） | Settings 页 Mirror URL + Apply `src/gui.rs:1735-1757`（同 http(s) 校验 `src/gui.rs:1736-1738`，同一 `set_registry_mirror`） | ✅ |
| 24 | `mirror`（省略 url）自动探测 | `src/main.rs:660-680`（同一 `detect_best_mirror` `src/build/registry.rs:58-65`） | Settings 页 Auto-detect Best Mirror 按钮 `src/gui.rs:1761-1771` | ✅ |
| 25 | `mirror -r/--remove` | `src/main.rs:653-656`（删除 daemon.json 键 `src/build/registry.rs:157-176`） | Settings 页 Remove Mirror 按钮 `src/gui.rs:1773-1784` | ✅ |
| 26 | `mirror --restart` | `src/main.rs:707-710`（`build::restart_docker` `src/build/docker.rs:401-492`） | Settings 页 Restart Docker 按钮 `src/gui.rs:1815-1839`（Windows 后台线程执行同一函数；**非 Windows 为等价性声明**：GUI 无 tty，不执行 sudo，改显同一手动命令指引 `src/gui.rs:1866-1898`） | ✅ |

**合计说明**：计划口径为 25 行；实际核对为 **26 行覆盖 + 3 行 N-A**——build 14 项 + list 3 项 + check 5 项 + mirror 4 项，算术和即 26，计划文本"25"为计数笔误。`--version`/`--cpu` 的 GUI 收窄以说明**并入第 2/4 行**（不单设行）。核对后准确数为 **26**，26 行全部 ✅，零 ❌/⚠️。

## N-A 声明行（CLI 专属面，GUI 以等价语义覆盖，不计覆盖率）

| # | CLI 功能点 | CLI 侧 file:line | GUI 侧等价性声明 | 状态 |
|---|---|---|---|---|
| N1 | 构建确认提示（`Start build?` / `Continue anyway? [Y/n]`） | `src/main.rs:328`、`src/main.rs:275`、`src/main.rs:287`（`ask_continue` `src/main.rs:739-745`） | GUI 无确认提示：**点击 START BUILD 即确认**（`src/gui.rs:1186-1193`）；"Docker 未运行仍继续"的语义由 Force 勾选表达（`src/gui.rs:1149-1157`、`src/gui.rs:1192`） | N-A（等价性声明） |
| N2 | 退出码语义（成功 0 / 失败 1 / 取消 130） | `src/main.rs:387-411`（`exit(1)` `src/main.rs:393`；`exit(130)` `src/main.rs:397`） | GUI 进程无 CLI 退出码，以**状态条三态语义等价**：✅ 绿=成功（`src/gui.rs:1239-1243`）/ ❌ 红=失败（`src/gui.rs:1244-1246`）/ ⏹ 黄=已取消（`src/gui.rs:1247-1249`） | N-A（等价性声明） |
| N3 | stdin 取消词（`q` / `quit` / 回车，大小写不敏感） | `src/main.rs:344-360`（取消词判定 `src/main.rs:352`） | GUI 以 **CANCEL BUILD 按钮**取消（`src/gui.rs:1167-1177` → `cancel_build` `src/gui.rs:364-369`，写同一个 cancel 标志，构建线程以同一 `BuildOutcome::Cancelled` 收束） | N-A（等价性声明） |

## 相关页面

- 各 GUI 控件的行为细节：[GUI 完整指南](gui/overview.md)。
- 覆盖不了的场景（如 Windows release CLI 无 stdout）属平台已知限制，不列入矩阵：见 [FAQ](faq.md) 与[故障排查](troubleshooting.md)。
- 质量门与 e2e 硬门口径：[开发指南](development.md)。
