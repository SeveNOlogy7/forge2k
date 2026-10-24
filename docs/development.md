# 开发指南

面向贡献者的架构说明、质量门与 CI/发布流程。

## 技术栈

- Rust stable（edition 2021），单二进制 `forge2k`（`src/main.rs`）。
- CLI：clap 4（derive）；GUI：eframe/egui 0.31；序列化：serde/serde_json；错误处理：anyhow；终端着色：colored；时间：chrono。

## 架构

```
src/
├── main.rs          # clap CLI（build/list/gui/check/mirror）+ stdin 取消 + Windows AttachConsole
├── gui.rs           # egui 五标签界面（Build/Configs/System/Settings/About）
├── settings.rs      # 设置持久化（serde + ~/.forge2k/settings.json）
├── mirrors.rs       # registry 镜像源单一来源（known_mirrors）
├── templates/       # 合成 Dockerfile 模板（spack/toolchain，include_str!）
└── build/
    ├── mod.rs       # 共享类型（BuildConfig/BuildOutcome/LogLine/ConfigInfo）+ re-export
    ├── execute.rs   # docker/native 构建执行、取消、CUDA 支持矩阵、native 依赖补装
    ├── generate.rs  # 合成 Dockerfile（feature 注册表、臂校验、OpenBLAS 补丁）
    ├── catalog.rs   # 捆绑 Dockerfile 目录扫描、配置列表、等效 CLI 命令组装
    ├── docker.rs    # Docker 引擎探测/重启 + BuildKit 与磁盘诊断探针（CLI/GUI 共用）
    ├── registry.rs  # registry 连通性检测与 daemon.json 镜像源读写
    └── tests.rs     # 测试套件（纯 Rust，不依赖 Docker daemon）
```

### 模块要点

- **main.rs**：`cmd_build` 做预检（features 校验 → Docker 检查，`--force`/`--generate-only` 可跳过）→ 确认提示 → 后台构建线程 + stdin 取消线程 → 按结果设退出码（0/1/130）。Windows release 为 GUI 子系统，CLI 输出经 `AttachConsole` 挂回父控制台。
- **gui.rs**：`Forge2kApp` 持有全部状态。关键纯函数 `collect_build_config`（UI 状态 → `BuildConfig`，无 `ui` 参数，可单测）与 `build_config_blockers`（非法组合硬门）。所有耗时探测（构建、BuildKit/存储探测、Windows Docker 重启）经 `run_probe` 在后台线程执行，UI 只读互斥锁快照。
- **build/execute.rs**：docker 构建与 native 构建（Step 0 配合白名单校验 → 依赖探测与 apt 补装 → clone → toolchain → 构建 → 验证）共用 `run_cmd_logged` + 取消标志；`cuda_options_for_method` 是方法→CUDA 支持矩阵的唯一来源。
- **build/generate.rs**：`KNOWN_FEATURES` 注册表是 CLI `--features` 取值白名单与 GUI 勾选组的共同数据源；`validate_features` / `validate_feature_arm` / `features_supported_arm` 三层校验保证非法组合 fail-loud。deepmd preset 注入 OpenBLAS `SMALL_MATRIX_OPT=0` 补丁，无 features 的渲染逐字节不变。
- **build/catalog.rs**：`list_available_configs` 扫描捆绑 dockerfiles/（exe 旁 → `./dockerfiles/` → `.`）并附加 synthesized 配置；`cli_build_command` 是 `list` 输出与 GUI Copy 按钮共用的命令组装器。
- **build/docker.rs**：`check_docker` / `check_buildkit` / `check_docker_storage` / `restart_docker`。磁盘探测用 PowerShell CIM（wmic 在 Windows 11 24H2+ 已移除）；这批探针同时供 CLI `check` 与 GUI System 页调用。
- **settings.rs / mirrors.rs**：settings 五字段全 `#[serde(default)]`，加载失败静默回退；known mirrors 是 CLI 自动探测与 GUI Settings 页的共同列表。

## 构建与质量门

```bash
cargo fmt --all -- --check            # 格式门禁
cargo clippy --all-targets -- -D warnings   # lint 门禁（警告即失败）
cargo check --all-targets             # 编译检查
cargo test --all-targets              # 单元 + 行为测试
```

与 `.github/workflows/ci.yml` 同门：CI 在 push（main）与 PR 时于 ubuntu / windows 双平台依次跑这四道门禁。提交前请本地全绿。

## 测试

- 测试全部内嵌于 `src/build/tests.rs`、`src/gui.rs`（`advanced_options_tests` / `config_tab_tests`）、`src/settings.rs`，**不依赖 Docker daemon**，可在任何开发机全量运行。
- GUI 测试只覆盖"画像素之前"的纯逻辑（状态映射、blockers、Generate Only 内核、取值联动回归钉）；egui 渲染本身不测。
- 行为测试覆盖生成器逐字节回归（master/无 features 臂渲染不变）、捆绑文件解析、磁盘解析器、settings 往返等。

## E2E（镜像构建硬门）

`.github/workflows/e2e-images.yml` 为手动触发（`workflow_dispatch`，`variant` 输入），用 debug 二进制驱动真实构建，共 **8 个变体**（guard 白名单）：

`cuda_P100` / `cuda_V100`（toolchain 2023.2 make 臂）、`spack_2025.2_x86_64`、`native_2023.2_x86_64_none`、`native_2023.2_generic_none`（runner 直连构建，无 Docker）、`toolchain_2026.2_generic`、`cuda_2026.2_P100`（2026.2 cmake 臂）、`toolchain_2026.2_generic_deepmd`（deepmd preset）。

硬门（失败即红）：

- 镜像构建本身（`forge2k build ...`）；
- CPU 变体的 `docker run cp2k --version` 冒烟（GPU 变体因 runner 无 NVIDIA 驱动**容忍失败**，构建步才是验收门）；
- 2026.2 CPU 变体的 H2O 功能测试套件（`ci/cp2k-tests/` 四个钉死的上游输入 + `assert_cp2k_output.sh`：退出码 0、"PROGRAM ENDED AT"、能量参考值逐位比对）；
- deepmd 变体的 cp2kflags 硬门（编译产物确认 `deepmd` + `libtorch` token）与 `deepmd-water` 计算级测试（np1/np2 能量与上游 `TEST_FILES.toml` M011 参考一致）。

spack 变体另接 actions/cache 搬运 BuildKit buildcache mirror（cache 服务偶发 miss 时有重试与 artifact 主通道兜底）。

## 发布流程

1. 推送 `v*` tag（如 `v1.0.1`）触发 `.github/workflows/release.yml`（tag push 不触发 ci.yml，release 自带 gate job 重跑 fmt/clippy/test）。
2. `build` job 双平台 `cargo build --release` + 冒烟（Linux 断言 `--version`/`list` 的 stdout；Windows 二进制无 stdout，改为断言退出码），打包 zip（恰好 2 个条目：二进制 + README.md）。
3. `release` job 以 **GitHub Release 草稿** 附上全部 zip（`generate_release_notes` 开启）。
4. 维护者检查草稿后**手动 publish**。不自动发版。

## 文档站（mkdocs）

docs/ 是 doc 站唯一内容源（mkdocs-material，中文）。本地构建：

```bash
python -m pip install mkdocs-material   # 版本与 CI 同款 pin
mkdocs serve                            # 本地预览
mkdocs build --strict                   # 零警告门禁
```

> 注：`mkdocs.yml`、中文搜索配置与 GitHub Pages 部署 workflow 由后续任务接入，本节届时补充部署细节。新增页面的约定：首行为 `#` 标题、相对链接、截图占位用 HTML 注释。

## 贡献约定

- 不引入与 UI/CLI 对齐相悖的第二套事实源：feature 注册表、CUDA 矩阵、配置列表、命令组装、check 探针、已知镜像列表均为构建侧单一来源，CLI 与 GUI 只消费。
- 修改 CLI 行为时同步检查 GUI 对应控件与[覆盖率矩阵](coverage.md)；修改 README 时以代码为准。
- 提交前：`cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets`。
