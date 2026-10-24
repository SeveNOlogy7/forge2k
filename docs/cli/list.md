# list — 列出预置配置

别名 `l`。列出全部可用的预置构建配置（CLI 模式；GUI 的 Configs 标签页与之同源）。

```bash
forge2k list [--method <METHOD>] [--version <VERSION>]
```

## 选项

| 选项 | 允许值 | 行为 |
|---|---|---|
| `--method <METHOD>` | `spack` / `toolchain`（白名单） | 按构建方法过滤。**没有 `native`**：native 无预置配置目录，其 6 组合白名单见 [native 方法](../methods/native.md) |
| `--version <VERSION>` | 任意字符串 | 按 CP2K 版本精确匹配过滤（如 `2025.2`、`master`） |

两个过滤都不给时列出全部。无匹配时打印提示并以退出码 0 结束。

## 输出列含义

每条配置四行，例如：

```text
  #4  CP2K v2023.2 with mpich, toolchain build
      Method: toolchain/2023.2 (ubuntu:22.04)
      Specs:  MPI=mpich / CPU=generic / CUDA=none / Variant=psmp
      Build:  forge2k build -m toolchain -v 2023.2 --mpi mpich --cpu generic --cuda none --variant psmp
```

| 列 | 含义 |
|---|---|
| `#N` | 序号（当前过滤结果内重新编号） |
| 描述行 | 版本、MPI、构建方法（toolchain build / Spack build），CUDA 非 none 时追加 GPU 型号、CPU 非 generic 时追加优化目标 |
| `[synthesized: ...]` 标记 | 该配置**没有捆绑 Dockerfile**，构建时按模板现场合成（见下节）；捆绑配置无此标记 |
| `Method:` | 方法 / 版本，括号内为 Docker 基础镜像（spack=ubuntu:24.04；toolchain CPU=ubuntu:22.04、CUDA=nvidia/cuda:12.2.0-devel-ubuntu22.04） |
| `Specs:` | MPI / CPU / CUDA / 变体四元组 |
| `Build:` | 等效的 `forge2k build` 命令，可直接复制执行 |

## synthesized 标记

`[synthesized: no bundled Dockerfile, generated at build time]` 表示该配置在 `dockerfiles/` 目录下没有捆绑文件，构建时 Forge2K 按 `src/templates/` 模板现场合成（构建日志会显式提示）。配置仍可正常选择与构建；想先看合成结果可用 `forge2k build ... --generate-only`。

## 广告配置（6 条无捆绑文件的合成配置）

除 `dockerfiles/` 下 6 份捆绑 Dockerfile（spack 2025.2 三条 + toolchain 2023.2 三条）外，以下 6 条知名配置始终被列出并标记 synthesized：

| 方法 | 版本 | MPI | CPU | CUDA | 变体 |
|---|---|---|---|---|---|
| spack | 2026.1 | mpich | x86_64 | none | psmp |
| spack | 2026.1 | mpich | cascadelake | none | psmp |
| spack | 2026.1 | openmpi | cascadelake | none | psmp |
| toolchain | master | mpich | generic | none | psmp |
| toolchain | 2026.2 | mpich | generic | none | psmp |
| toolchain | 2026.2 | mpich | generic | P100 | psmp |

捆绑清单与各方法的版本支持边界见 [spack 方法](../methods/spack.md) / [toolchain 方法](../methods/toolchain.md)。

## 相关页面

- [build 参考](build.md)：`Build:` 行各选项的完整说明
- [快速开始](../quick-start.md)
