# Changelog

本文件记录 forge2k 的显著变更。格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [Unreleased]

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
