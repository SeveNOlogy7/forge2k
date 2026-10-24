# spack 方法

`forge2k build -m spack`（CLI 默认方法）。基于 [Spack](https://spack.io/) 包管理器在 Docker 内构建 CP2K，基础镜像 `ubuntu:24.04`。

## 构建原理

两段式 Dockerfile（多阶段构建）：

1. **构建阶段**（`build_cp2k`）：安装指定版本的 Spack 与 spack-packages，注册 CP2K 的 spack 仓库（`cp2k_dev_repo`），按 CP2K 源码树内的 **deps yaml** 创建 spack 环境并 concretize，随后安装全部依赖与 CP2K 本体。24 个依赖包的编译产物会推入本地 buildcache mirror（配合 cache mount，见 [构建缓存](../cli/build.md)）
2. **运行阶段**（`runtime`）：同样基于 `ubuntu:24.04`，只从构建阶段拷贝安装产物，镜像内 `ENTRYPOINT ["cp2k"]`

版本 → Spack 组件对应（合成模板按版本自动选择）：

| CP2K 版本 | Spack | spack-packages | deps yaml |
|---|---|---|---|
| 2025.2 | v1.0.0 | v2025.07.0 | `cp2k_deps_all_<版本>.yaml` |
| 2026.1 及以后 / master | v1.1.1 | v2026.03.0 | `cp2k_deps_<版本>.yaml` |

deps yaml 的差异：v2026.1 起上游把 `cp2k_deps_all_psmp.yaml` 改名为 `cp2k_deps_psmp.yaml`，并在 `packages:all` 段新增 `require: target=""`（合成模板据此注入目标 CPU）。

## 版本支持

| 版本 | 支持状态 |
|---|---|
| 2025.2 | 支持；**捆绑 Dockerfile**（mpich/openmpi × x86_64/cascadelake 共 3 条，见 `forge2k list`） |
| 2026.1 | 支持；synthesized 合成（无捆绑文件，3 条广告配置见 [list](../cli/list.md)） |
| master | 支持；synthesized 合成 |
| **2026.2** | **暂不支持**（见下） |

### 2026.2 暂不支持的原因

上游 `support/v2026.2` 分支重排了 `tools/spack` 目录：`cp2k_dev_repo` → `spack_repo/cp2k_dev`、deps yaml 改名（`cp2k_deps_p.yaml`）、spack 包本体已从上游 tools/spack 仓库移除——现有 spack 合成模板引用的内容失效。

- CLI 的 `--generate-only` 对 spack + 2026.2 **仍可合成** Dockerfile（不硬挡），但实际 `docker build` 必败
- backlog：委托上游 `make_cp2k.sh` 路径。**当前 2026.2 仅 [toolchain 方法](toolchain.md)可用**

## CUDA 限制（仅 none）

spack 方法**仅支持 `--cuda none`**：spack 模板不含 CUDA 安装段，捆绑镜像全部 CPU-only。GUI 的 CUDA 下拉按方法联动只给 spack 提供 `none`；CLI 的取值白名单不按方法再过滤，非 `none` 的取值传入也不会产生 GPU 支持，请保持默认。

## 典型命令

```bash
forge2k build -m spack -v 2025.2 --mpi mpich --cpu cascadelake --variant psmp
forge2k build -m spack -v 2026.1                       # 其余选项走默认值
forge2k build -m spack -v 2026.1 --generate-only       # 只合成 Dockerfile，不构建
```

## 相关页面

- [build 参考](../cli/build.md)：全部选项、tag 命名、构建缓存
- [toolchain 方法](toolchain.md) / [native 方法](native.md)
