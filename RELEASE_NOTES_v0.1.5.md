# Obsidian Plugin Sync v0.1.5

[中文](#中文) · [English](#english)

## 中文

### 版本概览

v0.1.5 是 Obsidian Plugin Sync 的首个公开预发布版本。这是一款 Windows 桌面应用，用于在多个本地 Obsidian Vault 之间安全同步社区插件、插件配置和启用状态。

### 主要功能

- 支持一个源 Vault 同步到一个或多个目标 Vault。
- 可按插件分别选择插件文件、`data.json` 和启用状态。
- 提供版本差异检查，并对删除和降级操作要求额外确认。
- 写入前自动创建操作级备份，可根据备份清单完整恢复。
- 每次同步生成 `sync-report.md` 和 `sync-report.json`。
- 提供插件管理、标准设置编辑、原始 JSON 编辑和受信任适配器设置编辑。
- 可选 Settings Bridge 能捕获插件实际渲染的设置控件结构，同时过滤文本值、密码和可执行回调。
- 同步过程会跳过 junction、目录链接和符号链接。

### 下载文件

- `Obsidian Plugin Sync_0.1.5_x64-setup.exe`：推荐使用的 NSIS 安装程序。
- `Obsidian Plugin Sync_0.1.5_x64_en-US.msi`：适用于 MSI 部署流程的安装包。

### 使用说明

1. 下载并运行任一安装包。
2. 首次使用时先选择测试 Vault 验证同步结果。
3. 执行同步或恢复前关闭 Obsidian；检测到 `Obsidian.exe` 时应用会阻止写入。
4. 对删除或降级提示进行复核后再确认操作。

### 已知限制

- 当前仅提供 Windows x64 构建。
- 界面和可读报告以中文为主。
- 当前安装包未进行代码签名，Windows SmartScreen 可能显示未知发布者提示。
- Settings Bridge 捕获真实插件设置页时，目标插件自身仍可能执行网络请求、扫描或监听器初始化。
- 当前版本不提供自动更新；升级需要手动安装新版本。

### 验证情况

- Bridge 测试：11 项通过。
- Rust 测试：94 项通过，1 项需要本地插件环境的审计测试被忽略。
- TypeScript 类型检查与 Vite 生产构建通过。
- NSIS 和 MSI 安装包构建通过。

## English

### Overview

v0.1.5 is the first public pre-release of Obsidian Plugin Sync, a Windows desktop app for safely synchronizing community plugins, plugin configuration, and enabled state across local Obsidian vaults.

### Highlights

- Synchronize one source vault to one or more target vaults.
- Select plugin files, `data.json`, and enabled state independently for each plugin.
- Compare plugin versions and require additional confirmation for deletion and downgrade operations.
- Create operation-wide backups before writes and restore complete operations from backup manifests.
- Generate `sync-report.md` and `sync-report.json` for every synchronization.
- Manage plugins and edit supported settings through standard, raw JSON, and trusted-adapter editors.
- Optionally capture controls rendered by real plugin settings pages through Settings Bridge while excluding text values, passwords, and executable callbacks.
- Skip junctions, directory links, and symbolic links during synchronization.

### Assets

- `Obsidian Plugin Sync_0.1.5_x64-setup.exe`: recommended NSIS installer.
- `Obsidian Plugin Sync_0.1.5_x64_en-US.msi`: installer for MSI-based deployment workflows.

### Getting Started

1. Download and run either installer.
2. Use test vaults to verify your first synchronization.
3. Close Obsidian before applying or restoring changes; the app blocks writes while `Obsidian.exe` is detected.
4. Review all deletion or downgrade prompts before confirming the operation.

### Known Limitations

- Only Windows x64 builds are currently provided.
- The UI and human-readable reports are Chinese-first.
- The installers are currently unsigned, so Windows SmartScreen may show an unknown-publisher prompt.
- When Settings Bridge renders a real plugin settings page, the target plugin may still perform network requests, scans, or listener initialization.
- This version has no automatic updater; upgrades require manual installation.

### Verification

- 11 Bridge tests passed.
- 94 Rust tests passed; one environment-dependent local plugin audit test was ignored.
- TypeScript type checking and the Vite production build passed.
- Both NSIS and MSI bundles built successfully.
