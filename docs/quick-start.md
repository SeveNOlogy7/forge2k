# 快速开始

## 启动图形界面

无参数运行即默认启动 GUI：

```bash
forge2k
```

等价于显式的 `forge2k gui`。五个标签页（Build / Configs / System / Settings / About）的逐页说明见 [GUI 完整指南](gui/overview.md)。

## CLI 三条常用命令

```bash
# 查看全部预置配置（方法 / 版本 / 规格 / 等效 build 命令）
forge2k list

# 系统体检（Docker、BuildKit、registry 连通性、当前镜像源、存储与磁盘）
forge2k check

# CLI 构建：spack 方法 + CP2K 2026.1 + mpich + x86_64 + psmp（后四项均为默认值）
forge2k build -m spack -v 2026.1 --mpi mpich --cpu x86_64 --variant psmp
```

`build` 的确认提示 `Start build? [Y/n]` 处直接回车（或输入 `y` / `yes`）即开始。全部 14 个选项见 [build 参考](cli/build.md)。

## 首次构建要多久？

构建启动后日志会提示 **1-3 小时**量级，例如：

```text
[10:32:01] 🔨 Forge2K Build Engine v1.0.1
[10:32:01]    Method:    spack
[10:32:01]    Version:   2026.1
[10:32:01]    ...
[10:32:01] 🚀 Starting build (this may take 1-3 hours)...
[10:32:01] $ docker build --shm-size 1g -f ... -t cp2k/cp2k:2026.1_mpich_x86_64_psmp ...
```

- **spack 方法**冷构建实测约 4 小时（24 个依赖包全部编译）；BuildKit cache mount 与 Spack buildcache 命中时，同版本二次构建实测约 **7 分钟**（CP2K 本体的 cmake/编译段约 5 分钟仍会重跑）
- 构建过程中输入 `q` / `quit` 或直接按回车即可取消（大小写不敏感，退出码 130）
- 成功结束后按日志提示运行，例如：

```bash
docker run --rm -v $(pwd):/work cp2k/cp2k:2026.1_mpich_x86_64_psmp cp2k --help
```

缓存机制详见 [build 参考 · 构建缓存](cli/build.md)。

## 下一步

- 选方法：[spack](methods/spack.md) / [toolchain](methods/toolchain.md) / [native](methods/native.md)
- 预置配置清单：[list 参考](cli/list.md)
- 遇到问题：[故障排查](troubleshooting.md) / [FAQ](faq.md)
