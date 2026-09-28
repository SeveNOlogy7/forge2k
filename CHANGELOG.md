# Changelog

本文件记录 forge2k 的显著变更。格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [Unreleased]

### Added（第三期镜像与卷缓存优化）

- **BuildKit cache mounts**：六份捆绑 Dockerfile 与两份模板对四类下载向量加缓存——apt（`apt-2404`/`apt-2204`）、CP2K 源码（`cp2k-src-<版本>`，seed-copy + 增量 fetch 刷新）、Spack 包源码（`spack-src`）、toolchain tarballs（`toolchain-tarballs`）；`--no-cache` 重建与层缓存逐出时下载仍命中
- `generate.rs` 的 git clone 生成器同步为 canon-git 块（版本进 cache id，master 走 REF=master 刷新路径）
- 字段断言测试：bundled 六文件 stage 分段 mount 断言 + synthesized 三版本组合断言（测试 28 → 32）
- README 新增"构建缓存"节：cache id 表、实测的 `--no-cache` 语义、purge 命令、named volume 与 cache mount 的三层澄清
- **e2e 量化验证**：apt 段 ~26 倍提速（683s→26s）、CP2K 源码 clone（5-8 分钟）→ fetch 秒级、spack-src mount 命中下 spack install 零网络行；`cp2k/cp2k:2025.2_mpich_x86_64_psmp` 与 `:2023.2_mpich_generic_psmp` 双镜像构建 + `cp2k --version` 冒烟通过

### Changed（第三期镜像与卷缓存优化）

- 两个构建体系假设前置实证成立：toolchain 脚本 BUILDDIR + tarball skip-if-exists；Spack 1.2.2 `config:source_cache` 键有效

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
