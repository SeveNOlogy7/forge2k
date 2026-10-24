# FAQ

## 取消构建后镜像会残留吗？

不会留下坏镜像。取消（CLI 输入 `q`/`quit`/回车，GUI 点 CANCEL BUILD）会终止正在运行的 Docker/构建进程，镜像 tag 只在构建成功后才打上。已完成的中间层与 BuildKit 缓存仍在，可用于下次提速；已存在的旧镜像不受影响。

## 构建缓存怎么清？

- 清 BuildKit cache mount（下载/编译缓存）：`docker builder prune --filter type=exec.cachemount`
- 层缓存：`docker builder prune`（或 Docker Desktop 设置里清理）。
- **注意**：`--no-cache` 重建时 cache mount 视图为空、不命中既有缓存——若只想制造层失效而保留下载缓存，应修改 Dockerfile 而非用 `--no-cache`。缓存磁盘占用约 5–6GB，以 `docker system df` 实测为准。

## GUI 设置存在哪？能删吗？

`~/.forge2k/settings.json`（home 取 `USERPROFILE`，回退 `HOME`）。保存 5 个字段：`mirror_input`（镜像 URL 草稿）、`no_cache`、`force`、`features`、`dockerfile`。注意**已生效的 registry 镜像源不在这里**——它写在 Docker 的 daemon.json 里。文件损坏或删除都会静默回退默认值，可以放心删。

## native 方法支持 Windows 吗？

不支持。native 构建直接在主机上跑 bash 脚本、apt 补装依赖、使用 `/opt/cp2k_build` 工作目录，仅 Linux/WSL 可用（GitHub Actions ubuntu runner 已验证 x86_64/generic 两组合全链路）。Windows 用户请用 spack/toolchain（Docker）方法。

## 多个镜像源能配几个？

通过 forge2k 只能配 **1 个**：`mirror` 子命令与 GUI Apply 会把 daemon.json 的 `registry-mirrors` 键整体覆盖为单元素数组（`[本次写入的 URL]`），文件中其余键保留。需要多个镜像源请手动编辑 daemon.json；`check` 也只显示该数组的第一个条目。

## 退出码 130 是什么？

**构建被取消**。退出码语义：成功 `0`、构建失败（含构建进程异常退出）`1`、取消 `130`。取消方式：构建中输入 `q` / `quit` 或直接回车（大小写不敏感），GUI 等价操作是 CANCEL BUILD 按钮。两类"温和退出"仍是 `0`：确认提示处回答否（`Continue anyway?` / `Start build?`），以及 Docker 未安装时。

## `forge2k list` 里为什么看不到 native 配置？

`list` 只扫描捆绑的 `dockerfiles/spack/` 与 `dockerfiles/toolchain/` 目录并附加已知 synthesized 配置；native 不经 Docker、没有 Dockerfile，因此不在 list 范围内，`list --method` 的枚举也只有 spack/toolchain（GUI Configs 页的过滤下拉一致）。native 的版本/组合约束见 `build -m native --help` 与 GUI Build 页的 native 选项说明。

## spack + 2026.2 为什么构建失败？

上游 support/v2026.2 重排了 `tools/spack` 目录，现有 spack 模板引用失效，因此配置目录不再广告 spack 2026.2。2026.2 请用 toolchain 方法。详见[故障排查](troubleshooting.md#spack-方法为什么不能构建-2026-2)。

## 为什么 GUI 的 CUDA 下拉在 spack 下没有 P100/V100？

spack 捆绑镜像只有 CPU 变体，支持矩阵规定 spack 仅 `none`；GUI 直接按构建侧矩阵渲染（而非界面层另行发明），所以下拉只有 none。toolchain/native 方法可选 none/P100/V100。

## 构建要多久？要多少磁盘？

CP2K 镜像构建通常 1–3 小时（视机器而定），建议至少 50GB 空闲磁盘。spack 系二次构建受益于 BuildKit cache mount + 编译级 buildcache（实测层失效后二次构建约 7 分钟；冷轮约 4 小时）。System 页的 Check Storage 可查 Docker 存储卷剩余空间。

## Windows 下 release 版 forge2k 在终端没有输出？

正常现象。Windows release 构建是 GUI 子系统（双击不弹黑框），CLI 模式靠 AttachConsole 挂到父控制台；个别终端环境下可能无 stdout（退出码正常）。debug 构建（`cargo run`）不受影响。

## master 版本的镜像 tag 是什么样？

`master` 分支不带固定版本号，tag 使用构建当日日期戳：`cp2k/cp2k:master_YYYYMMDD_{mpi}_{cpu}[_cuda_{GPU}]_{variant}`。其余版本为 `cp2k/cp2k:{version}_{mpi}_{cpu}[_cuda_{GPU}]_{variant}`；带 `--features deepmd` 时追加 `_deepmd` 后缀。

## 相关页面

- 安装与快速开始见站内《安装》《快速开始》页；CLI 全量参考见《CLI 参考》页。
- 故障排查细目：[troubleshooting](troubleshooting.md)；GUI 逐控件说明：[GUI 指南](gui/overview.md)；开发相关：[开发指南](development.md)。
