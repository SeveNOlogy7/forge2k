# spack 方法

`forge2k build -m spack`（CLI 默认方法）。基于 [Spack](https://spack.io/) 包管理器在 Docker 内构建 CP2K，基础镜像 `ubuntu:24.04`。

## 构建原理

两段式 Dockerfile（多阶段构建）：

1. **构建阶段**（`build_cp2k`）：**2025.2 / 2026.1（旧 spack 布局臂）** 安装指定版本的 Spack 与 spack-packages，注册 CP2K 的 spack 仓库（`cp2k_dev_repo`），按 CP2K 源码树内的 **deps yaml** 创建 spack 环境并 concretize，随后安装全部依赖与 CP2K 本体。24 个依赖包的编译产物会推入本地 buildcache mirror（配合 cache mount，见 [构建缓存](../cli/build.md)）。**2026.2 / master（make_cp2k.sh 委托臂）** 的构建阶段则是一条 `./make_cp2k.sh` 委托调用，见[下节](#构建臂分界与-make_cp2ksh-委托2026-2--master)
2. **运行阶段**（`runtime`）：同样基于 `ubuntu:24.04`，只从构建阶段拷贝安装产物，镜像内 `ENTRYPOINT ["cp2k"]`

版本 → Spack 组件对应（旧布局臂的合成模板按版本自动选择；2026.2 / master 委托 make_cp2k.sh，组件版本由上游脚本自管）：

| CP2K 版本 | Spack | spack-packages | deps yaml |
|---|---|---|---|
| 2025.2 | v1.0.0 | v2025.07.0 | `cp2k_deps_all_<版本>.yaml` |
| 2026.1 | v1.1.1 | v2026.03.0 | `cp2k_deps_<版本>.yaml` |

deps yaml 的差异：v2026.1 起上游把 `cp2k_deps_all_psmp.yaml` 改名为 `cp2k_deps_psmp.yaml`，并在 `packages:all` 段新增 `require: target=""`（合成模板据此注入目标 CPU）。

## 版本支持

| 版本 | 支持状态 |
|---|---|
| 2025.2 | 支持；**捆绑 Dockerfile**（mpich/openmpi × x86_64/cascadelake 共 3 条，见 `forge2k list`） |
| 2026.1 | 支持；synthesized 合成（无捆绑文件，3 条广告配置见 [list](../cli/list.md)） |
| **2026.2** | 支持；synthesized 合成（make_cp2k.sh 委托臂，注册单组合 mpich/x86_64/psmp） |
| master | 支持；synthesized 合成（与 2026.2 同走 make_cp2k.sh 委托臂） |

## 构建臂分界与 make_cp2k.sh 委托（2026.2 / master）

上游 `support/v2026.2` 重排了 `tools/spack` 目录：`cp2k_dev_repo` → `spack_repo/cp2k_dev`、deps yaml 改名（`cp2k_deps_p.yaml` 系）、spack 包本体移出上游 tools/spack 仓库——旧 spack 合成模板引用的内容失效。从 2026.2 起（**master 一并如此**：上游 develop 携带同一重排后布局，走旧布局同样失效），合成模板改为 `spack-makecp2k` 委托臂：

- **机制**：构建阶段克隆 CP2K 源码后执行一条 `./make_cp2k.sh`——上游 v2.0 官方 Spack+CMake 一体化脚本，自装 Spack 至 `cp2k/spack/`、按其自带 deps yaml 解析依赖、CMake+Ninja 构建并装至 `cp2k/install/`；运行阶段照搬官方 `tools/docker/Dockerfile.test_spack_*` 的拷贝口径（spack opt 树 + install 树 + `/etc/ld.so.conf.d/cp2k.conf`）。模板不引用任何 `tools/spack` 内部路径，上游再重排不影响本模板
- **注册单组合**：GUI / `forge2k list` 只广告 mpich/x86_64/psmp 一组（与 e2e 验证面对齐，不广告未验证组合）；CLI 自由字符串可合成其他组合，但未经验证不广告
- **e2e 已验证**：run 41551594889，镜像 `cp2k/cp2k:2026.2_mpich_x86_64_psmp`，容器内 `cp2k --version` 硬门输出 `CP2K version 2026.2`（历经三轮修复：ld.so.conf COPY 路径 → launch 入口缺失 → WORKDIR pin）
- **buildcache 未接入**：旧布局臂的 Spack buildcache / ccache 不覆盖该臂（上游脚本自管其 `cp2k/spack_cache` 缓存），冷构建为小时级
- **旧臂零变化**：分臂谓词为 `master || >= 2026.2`，2025.2 / 2026.1 合成渲染与此前逐字节相同

## CUDA 限制（仅 none）

spack 方法**仅支持 `--cuda none`**：spack 模板不含 CUDA 安装段，捆绑镜像全部 CPU-only。GUI 的 CUDA 下拉按方法联动只给 spack 提供 `none`；CLI 的取值白名单不按方法再过滤，非 `none` 的取值传入也不会产生 GPU 支持，请保持默认。

## 典型命令

```bash
forge2k build -m spack -v 2025.2 --mpi mpich --cpu cascadelake --variant psmp
forge2k build -m spack -v 2026.1                       # 其余选项走默认值
forge2k build -m spack -v 2026.2 --mpi mpich --cpu x86_64 --variant psmp   # make_cp2k.sh 委托臂（e2e 已验证）
forge2k build -m spack -v 2026.1 --generate-only       # 只合成 Dockerfile，不构建
```

## 相关页面

- [build 参考](../cli/build.md)：全部选项、tag 命名、构建缓存
- [toolchain 方法](toolchain.md) / [native 方法](native.md)
