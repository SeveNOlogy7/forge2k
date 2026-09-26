# Changelog

本文件记录 forge2k 的显著变更。格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [Unreleased]

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
