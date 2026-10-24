# gui — 启动图形界面

别名 `g`。启动 egui 图形界面。

```bash
forge2k gui
```

- **无参数**：`forge2k` 不带子命令时**默认启动 GUI**，无需显式输入 `gui`
- **无任何选项**：GUI 没有命令行参数；构建配置全部在界面内选择
- **窗口**：单窗口五个标签页（Build / Configs / System / Settings / About），覆盖 CLI 全部可 UI 化功能——构建（含 `--features` / `--no-cache` / 自定义 Dockerfile / `--force` / Generate Only 预览）、配置浏览与过滤、系统诊断、镜像源管理与 Restart Docker

各标签页的逐页说明（含控件行为与 CLI 等价关系）见 [GUI 完整指南](../gui/overview.md)。

!!! note "Windows 提示"

    Windows release 版为 GUI 子系统构建，双击 `forge2k.exe` 直接打开窗口、无黑色控制台；CLI 用法见 [build](build.md) / [list](list.md) / [check](check.md) / [mirror](mirror.md)。

## 相关页面

- [GUI 完整指南](../gui/overview.md)：五标签页详解
- [build 参考](build.md)：GUI Build 标签页背后的 CLI 语义
- [快速开始](../quick-start.md)
