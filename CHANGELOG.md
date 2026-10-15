# Changelog

本文件记录 forge2k 的显著变更。格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [Unreleased]

### Added（第七期 CP2K 2026.2）

- **CP2K 2026.2 toolchain 支持**：版本注册 2026.2（toolchain 合成两条：generic none / generic P100，psmp 变体；CLI 默认版本仍为 2026.1）；release 构建新增 **release-cmake 臂**——上游 ≥2026.1 已删除根 Makefile，走 `cmake -GNinja` 构建（EVERYTHING=ON + DLAF/PEXSI/DEEPMD/LIBTORCH OFF 共用基础旗标集）；CUDA 侧新增 cmake 三旗标（`-DCP2K_USE_ACCEL=CUDA -DCP2K_WITH_GPU=<GPU> -DCMAKE_CUDA_ARCHITECTURES=<sm>`，P100→60 / V100→70），P100/V100 变体对 2026.2 可用
- 新增 `--generate-only` 模式：跳过 docker 检查与确认提示，仅解析/生成 Dockerfile 并打印路径摘要（exit 0），未置位时默认行为零变化
- `E2E image build` workflow 新增两 dispatch 变体（`toolchain_2026.2_generic` / `cuda_2026.2_P100`）：2026-10-13 双变体 CI 真实编译成功（CPU 冒烟硬门输出 `CP2K version 2026.2`；镜像 `cp2k/cp2k:2026.2_mpich_generic_psmp` 与 `cp2k/cp2k:2026.2_mpich_generic_cuda_P100_psmp`；run 38422866467 / 38422948855）
- **`--features` 功能预设机制**：`forge2k build` 新增 `--features <FEATURES>`（逗号分隔，默认空），首个 preset `deepmd`；仅在 toolchain 方法的 2026.x release-cmake 臂生效，未知 feature、重复 feature 或不支持臂上携带均 fail-loud 报错退出，不静默忽略
- **`deepmd` preset**：成对翻转 `-DCP2K_USE_DEEPMD` / `-DCP2K_USE_LIBTORCH` 为 ON（上游语义成对），无需任何额外路径/环境变量（工具链 `setup` 的 `CMAKE_PREFIX_PATH` 已覆盖依赖）；镜像 tag 追加后缀 `_deepmd`（如 `cp2k/cp2k:2026.2_mpich_generic_psmp_deepmd`），无 features 时 tag 与旧版逐字节相同

### Fixed（第七期）

- **dbcsr config 版本补丁**：release-cmake 臂内以 sed 修正 tarball 构建产出的 `DBCSRConfigVersion.cmake`——其 `git describe` 爬到外层 cp2k tag，产生畸形版本串 `2026.2.v2026.2v2026.2`，被 SameMajorVersion 语义拒绝导致 `find_package(DBCSR 2.8)` 失败（probe run 38393254283 实证）
- **openPMD OFF**（失败迭代如实记录：e2e 第 1 轮 runs 38404980161 / 38405057080 双变体同点 FAIL 于 `find_package(openPMD 0.16.1 REQUIRED)`，dbcsr 补丁在该轮已验证生效）：toolchain `--install-all` 依赖集不含 openPMD，经 41 项组件对照矩阵确认其为唯一额外 OFF，追加 `-DCP2K_USE_OPENPMD=OFF` 后第 2 轮双变体 success
- **native 2026.2 广告摘除**（对抗评审 verdict=confirmed followup）：GUI native 方法版本列表误含 2026.2，而 native release 臂仍走 `make`（`use_cmake` 仅覆盖 master）、上游 ≥2026.1 已删根 Makefile 必败，广告面与执行路径不一致——从 native 广告面移除 2026.2 并加负向断言锁定；native ≥2026.1 的 cmake 迁移与基线遗留的 2026.1 广告问题登记 backlog（README 已知限制）

### Documented（第七期）

- README：版本行与构建方法节补 2026.2（toolchain 的 cmake 构建路径、CUDA 变体可用性）；已知限制新增两条——spack 方法暂不支持 2026.2（上游 tools/spack 重排，backlog=委托上游 make_cp2k.sh）与 2026.2 CI 验证状态（构建 + `--version` 冒烟口径，两处 deliberate workaround 登记）

### Added（第六期 CI 根治 + native 参数化）

- **buildcache CI 侧根治**：搬运介质从 actions/cache（v2 索引间歇 miss）换为 **artifact 跨 run 主通道**（download-artifact run-id + actions:read；cache 链降级为 artifact miss 时的条件 fallback）——CI 复用轮实测 **107/107 依赖包从 binary cache 命中安装、零源码编译**，locate 跨 run 定位 `hit=true`
- **native 全参数化**：白名单从单组合扩展为 6 组合（CPU generic/x86_64 × CUDA none/P100/V100，mpich/psmp 锁定，master+CUDA 显式拒绝）；mpich prereq 产品级自动补装（mpicc 探测 + mpich/libmpich-dev/bzip2 包名映射 + sudo 升级）
- `E2E image build` workflow 新增 native 两变体（`native_2023.2_x86_64_none` / `native_2023.2_generic_none`）：CI 上全链路构建 + exe 断言 + UCX-pinned `--version` 冒烟硬门
- native CI e2e 实证：x86_64_none 与 generic_none 两组合全链路构建 + `--version` 冒烟均通过

### Fixed（第六期）

- **prune 自删 bug**：清理步 jq 用 `.run_id`（repo-wide API 无此字段，恒 null → 恒真）导致每个 run 结束时删掉自己刚上传的 buildcache artifact——改为 `.workflow_run.id`
- **微架构哈希漂移**：GitHub runner 池混合 icelake/zen2 等 CPU，host-adaptive target 使 DAG 哈希跨 runner 不稳定——spack 系钉 `concretizer:targets:granularity:generic`（产物为 baseline x86_64，可移植性增强；代价为放弃 host 自适应优化，见 README）
- **native 路径遗留缺陷**（runner probe 实证）：`--with-deepmd=no` 为 2023.2 脚本未知旗标（execute.rs 与模板双处）；`x86_64` 为 toolchain 脚本非法 CPU 值（OpenBLAS getarch / gfortran -mtune 均拒）；MPICH+UCX 在无 IB 环境的线程探测误报——分别以旗标清理、x86_64→generic 翻译、`UCX_TLS` 注入修复
- bunzip2 非 Ubuntu 包名（provider=bzip2）→ prereq 映射表补齐

### Added（第五期编译级缓存）

- **Spack buildcache**：spack 系捆绑 Dockerfile 与模板把 24 个依赖包编译产物推入本地 buildcache mirror（独立 cache mount，`--unsigned` 无签名门槛）；同版本重建时依赖整包从 binary cache 安装——实测 Dockerfile 层失效后的二次构建 **7 分 06 秒**（冷轮 ~4h，107 处 relocating 命中、零源码编译）
- **ccache 叠加**：buildcache miss 时新编译的 C/C++ 对象进入独立 ccache mount（`config:ccache:true` + `CCACHE_DIR`）
- `E2E image build` workflow 新增 spack 变体（`spack_2025.2_x86_64`）：actions/cache 搬运 buildcache mirror（restore→seed→构建→export→save 五环），CI 冷轮 ~4h 全量成功 + 冒烟硬门通过

### Fixed（第五期编译级缓存）

- Wave 0 先行实证拦截的形态偏差：Spack 1.2.2 `config add` 无 `--scope`（直写 user scope）、`mirror add` 保留 `--scope`、命中词法为 `relocating`、warm-mirror no-op push 误报容错

### Known limitation（第五期编译级缓存）

- toolchain 系（v2023.2）不做编译级缓存：上游脚本写死编译器路径 + ccache 不支持 Fortran（上游设计）
- GitHub Actions 侧 buildcache restore 间歇性 miss（cache service v2 索引问题，社区 #1710/#1694/#1735；已加 restore 重试 + v4.3.0 pin，仍偶发）——本机 cache mount 路径不受影响

### Added（第四期 CUDA 验证）

- CUDA P100/V100 两变体在 GitHub Actions 完成独立全量构建验证（均成功）；新增 workflow_dispatch 手动触发的 `E2E image build` workflow 作为按需镜像验证工具

### Fixed（第四期镜像 hygiene）

- CUDA Dockerfile 与合成生成器的 legacy `ENV K V` 语法改为 `K=value` 等值形式，BuildKit LegacyKeyValueFormat 警告全仓清零

## [1.0.1] - 2026-09-28

### Added（第三期镜像与卷缓存优化）

- **BuildKit cache mounts**：六份捆绑 Dockerfile 与两份模板对四类下载向量加缓存——apt（`apt-2404`/`apt-2204`）、CP2K 源码（`cp2k-src-<版本>`，seed-copy + 增量 fetch 刷新）、Spack 包源码（`spack-src`）、toolchain tarballs（`toolchain-tarballs`；构建期下载缓存，named volume 的构建期正解）
- `generate.rs` 的 git clone 生成器同步为 canon-git 块（版本进 cache id，master 走 REF=master 刷新路径）
- 字段断言测试：bundled 六文件 stage 分段 mount 断言 + synthesized 三版本组合断言（测试 28 → 32）
- README 新增"构建缓存"节：cache id 表、实测的 `--no-cache` 语义、purge 命令、named volume 与 cache mount 的三层澄清
- **e2e 量化验证**：apt 段 ~26 倍提速（683s→26s）、CP2K 源码 clone（5-8 分钟）→ fetch 秒级、spack-src mount 命中下 spack install 零网络行；`cp2k/cp2k:2025.2_mpich_x86_64_psmp` 与 `:2023.2_mpich_generic_psmp` 双镜像构建 + `cp2k --version` 冒烟通过

### Changed（第三期镜像与卷缓存优化）

- 两个构建体系假设前置实证成立：toolchain 脚本 BUILDDIR + tarball skip-if-exists；Spack 1.2.2 `config:source_cache` 键有效

### Fixed（第三期镜像与卷缓存优化）

- canon 网络健壮性：clone 三级重试 + HTTP/1.1、apt `Acquire::Retries=3`（真实代理瞬断下四次 clone 全部存活）
- torch symlink 探针修复补齐 mpich 两变体（fix-wave-2 仅覆盖 openmpi）
- toolchain 安装自愈重试环：下载中断的残缺 tar 不再毒化缓存 mount（失败自动清理重下）

### Added（第二期优化设计）

- **模块化架构**：2199 行的 `build.rs` 拆分为 `src/build/` 六个子模块（mod/execute/registry/catalog/generate/docker）+ `tests.rs`，纯移动零行为变更
- **GUI 设置持久化**：设置保存到 `~/.forge2k/settings.json`，重启保留；坏文件降级为默认值
- **版本列表缓存**：GUI 启动加载一次 + 手动 Refresh，消除每帧磁盘扫描
- **CUDA 选项过滤**：GUI 下拉按构建方法联动（spack 仅 none / toolchain 含 P100/V100），与 native 校验矩阵同源
- **结构化日志**：`LogLine.is_error` 升级为 `LogLevel`（Info/Warn/Error），为后续日志分级铺路
- **发布工程**：`.gitattributes` 行尾策略；`[profile.release]`（thin-LTO + strip）；`release.yml`（tag 触发，自含 fmt/clippy/test 门禁 + 产物冒烟，产出 GitHub Release 草稿）
- **测试 16 → 28**：新增选项过滤映射、LogLevel、settings schema 契约测试；全部红演练验证断言有效

### Changed（第二期优化设计）

- unwrap 45 → 0：36 处锁中毒改 `PoisonError::into_inner` 恢复，9 处改带原因注释的 `expect`
- 取消/超时语义审计：kill → Cancelled 单路径确认，无第二路径

## [1.0.0] - 2026-09-23

### Added

- Forge2K GUI + CLI：一键构建 CP2K Docker 镜像（spack / toolchain / native 三种方法）
- CLI 子命令 build / list / gui / check / mirror；构建可取消（stdin `q` 或 GUI 按钮）
- 6 个捆绑 Dockerfile（spack 2025.2×3 + toolchain 2023.2×3，已对齐上游 cp2k-containers）与合成回退
- GitHub Actions CI：双平台 fmt / clippy / check / test 四门禁
- 中文 README

### Fixed

- 构建失败仍报成功的假成功（`Result<BuildOutcome>` 传播）
- 取消失效（子进程轮询 + kill）
- GUI 选 native 走错构建路径
- openmpi 镜像运行时误装 libmpich-dev
- 捆绑 Dockerfile 的 spack concretize 失败与 toolchain 未知旗标（两条镜像全量 e2e 构建与冒烟验证通过）
