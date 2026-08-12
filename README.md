# Obsidian Plugin Sync

<p align="center">
  <img src="./public/icon.png" alt="Obsidian Plugin Sync" width="112" />
</p>

<p align="center">
  在多个本地 Obsidian 仓库之间安全同步社区插件的 Windows 桌面应用。<br />
  A Windows desktop app for safely synchronizing Obsidian community plugins across local vaults.
</p>

<p align="center">
  <a href="#中文">中文</a> · <a href="#english">English</a>
</p>

---

## 中文

### 项目简介

Obsidian Plugin Sync 是一款基于 Tauri 的 Windows 桌面应用，用于将一个源仓库中的 Obsidian 社区插件同步到一个或多个目标仓库。它可以分别控制插件文件、`data.json`、启用状态和目标端多余插件的删除，并在写入前创建可恢复备份。

当前版本为 **0.1.5**。项目目前仅面向 Windows，界面和可读报告以中文为主。

### 主要功能

- 从一个源 Vault 同步到一个或多个目标 Vault。
- 按插件选择是否同步插件文件、配置文件 `data.json` 和启用状态。
- 检测版本差异，并对删除、降级等高影响操作要求明确确认。
- 在目标 Vault 内创建完整操作备份，并支持按备份清单恢复。
- 为每次同步生成 Markdown 和 JSON 两种格式的报告。
- 通过可选的 Settings Bridge 读取插件实际渲染的设置控件结构。
- 提供插件管理、原始 JSON 配置编辑和适配器增强的设置编辑能力。

### 安全机制

- 应用或恢复操作前需要关闭 Obsidian；检测到 `Obsidian.exe` 运行时会阻止写入。
- 每次写入前，目标 Vault 中的原文件会备份到 `.obsidian-plugin-sync-backups/`。
- 应用会通过 `.obsidian/app.json` 将备份目录加入 Obsidian 的排除文件列表。
- 删除和降级操作需要额外确认。
- 目录链接、junction 和符号链接会被跳过，不会被复制或删除。
- 每次同步都会在备份目录中写入 `sync-report.md` 和 `sync-report.json`。

> [!IMPORTANT]
> 同步或恢复前请先关闭 Obsidian，并在首次使用时先对测试 Vault 执行操作。

### 基本使用流程

1. 启动应用并选择一个源 Vault。
2. 添加一个或多个目标 Vault。
3. 扫描插件差异，选择需要同步的内容和目标。
4. 关闭 Obsidian，检查删除或降级提示，然后执行同步。
5. 查看同步报告；如需回滚，可从对应备份清单执行完整恢复。

### Settings Bridge

可选的 Settings Bridge 伴随插件运行在指定的 Obsidian Vault 中，使桌面应用能够读取普通插件实际渲染的设置控件。

1. 在“插件管理”中选择一个 Vault 和普通社区插件。
2. 在 Settings Bridge 面板中为该 Vault 安装或启用 Bridge。
3. 保持 Obsidian 运行，请求捕获设置结构或打开插件真实设置页。
4. 返回桌面应用并刷新，查看捕获结果。

快照保存在：

```text
.obsidian/plugins/obsidian-plugin-sync-bridge/cache/v1/
```

快照仅保存经过清理的结构信息，例如标签、控件类型、下拉选项和滑块范围，不保存当前文本、密码值或可执行回调。运行时元数据只改善设置展示，不会绕过 JSON 写入限制。Bridge 本体及其缓存不会参与普通的多 Vault 同步。

安装、修复、禁用和移除均按 Vault 独立管理。修改 Bridge 安装时需要关闭 Obsidian，并会创建可恢复备份。

### 技术栈

- [Tauri 2](https://tauri.app/)：桌面应用外壳与原生能力
- [React 19](https://react.dev/) + [TypeScript 6](https://www.typescriptlang.org/)：前端界面
- [Rust](https://www.rust-lang.org/)：文件同步、备份、恢复和进程检查
- [Vite 8](https://vite.dev/)：前端开发与构建
- [tree-sitter](https://tree-sitter.github.io/tree-sitter/)：插件设置结构分析

### 项目结构

```text
.
├─ src/                 # React 桌面界面
├─ src-tauri/           # Tauri/Rust 后端与打包配置
├─ bridge-plugin/       # Obsidian Settings Bridge 伴随插件
├─ public/              # 前端静态资源
└─ scripts/             # 项目辅助脚本
```

### 开发环境

#### 前置要求

- Windows 10 或 Windows 11
- Node.js 与 npm
- Rust stable 工具链
- Tauri 在 Windows 上要求的 Microsoft C++ Build Tools 与 WebView2
- 用于实际测试的 Obsidian 和至少两个测试 Vault

#### 安装依赖

```powershell
npm install
```

#### 启动开发环境

仅启动 Vite 前端：

```powershell
npm run dev
```

启动完整 Tauri 桌面应用：

```powershell
npm run tauri dev
```

#### 测试与构建

```powershell
# Bridge 单元测试
npm run test:bridge

# Rust 后端测试
cargo test --manifest-path src-tauri/Cargo.toml

# Bridge + TypeScript + 前端生产构建
npm run build

# Windows 安装包
npm run tauri build
```

生成的 Windows 安装包位于：

```text
src-tauri/target/release/bundle/nsis/Obsidian Plugin Sync_<version>_x64-setup.exe
src-tauri/target/release/bundle/msi/Obsidian Plugin Sync_<version>_x64_en-US.msi
```

非重大更新默认保持当前应用版本，不自动递增补丁版本；同一版本的构建应通过 Git 提交进行追踪。

### 项目状态

- 平台：Windows
- 当前版本：0.1.5
- 发布说明：[v0.1.5 Release Notes](./RELEASE_NOTES_v0.1.5.md)
- 界面语言：中文优先
- 阶段：持续开发中

问题反馈和 Pull Request 均可通过 GitHub 提交。

### 许可证

本项目基于 [MIT License](./LICENSE) 开源。

---

## English

### Overview

Obsidian Plugin Sync is a Tauri-based Windows desktop app that synchronizes Obsidian community plugins from one source vault to one or more target vaults. Plugin files, `data.json`, enabled state, and target-only deletion can be controlled independently, with recoverable backups created before any write operation.

The current version is **0.1.5**. The project currently targets Windows only, and its UI and human-readable reports are Chinese-first.

### Features

- Synchronize one source vault to one or more target vaults.
- Choose per plugin whether to sync plugin files, `data.json`, and enabled state.
- Compare plugin versions and require explicit confirmation for deletion or downgrade operations.
- Create operation-wide backups inside each target vault and restore from backup manifests.
- Generate both Markdown and JSON reports for every synchronization.
- Inspect the setting controls rendered by plugins through the optional Settings Bridge.
- Manage plugins and edit supported settings through raw JSON or adapter-enhanced editors.

### Safety Model

- Obsidian must be closed before applying or restoring changes; writes are blocked while `Obsidian.exe` is detected.
- Existing target files are backed up under `.obsidian-plugin-sync-backups/` before each write.
- The backup directory is added to Obsidian's excluded files through `.obsidian/app.json`.
- Delete and downgrade operations require an additional confirmation.
- Directory links, junctions, and symbolic links are skipped rather than copied or deleted.
- Every synchronization writes `sync-report.md` and `sync-report.json` into its backup directory.

> [!IMPORTANT]
> Close Obsidian before synchronizing or restoring, and use test vaults for your first run.

### Basic Workflow

1. Launch the app and select a source vault.
2. Add one or more target vaults.
3. Scan plugin differences and select the content and targets to synchronize.
4. Close Obsidian, review deletion or downgrade prompts, and apply the operation.
5. Review the generated report. If necessary, restore the complete operation from its backup manifest.

### Settings Bridge

The optional Settings Bridge companion runs inside a selected Obsidian vault, allowing the desktop app to inspect the setting controls that regular plugins render at runtime.

1. Select a vault and a regular community plugin in Plugin Management.
2. Install or enable the Bridge for that vault from the Settings Bridge panel.
3. Keep Obsidian running, then request a structure capture or open the plugin's real settings page.
4. Return to the desktop app and refresh to inspect the captured result.

Snapshots are stored under:

```text
.obsidian/plugins/obsidian-plugin-sync-bridge/cache/v1/
```

Snapshots contain sanitized structure such as labels, control types, dropdown options, and slider limits. They do not contain current text, password values, or executable callbacks. Runtime metadata improves presentation only and does not bypass JSON write restrictions. The Bridge and its cache are excluded from ordinary multi-vault synchronization.

Installation, repair, disablement, and removal are managed independently per vault. Obsidian must be closed when changing the Bridge installation, and a recoverable backup is created first.

### Tech Stack

- [Tauri 2](https://tauri.app/) for the desktop shell and native capabilities
- [React 19](https://react.dev/) and [TypeScript 6](https://www.typescriptlang.org/) for the UI
- [Rust](https://www.rust-lang.org/) for synchronization, backup, restore, and process checks
- [Vite 8](https://vite.dev/) for frontend development and builds
- [tree-sitter](https://tree-sitter.github.io/tree-sitter/) for plugin settings structure analysis

### Project Structure

```text
.
├─ src/                 # React desktop UI
├─ src-tauri/           # Tauri/Rust backend and bundling configuration
├─ bridge-plugin/       # Obsidian Settings Bridge companion plugin
├─ public/              # Frontend static assets
└─ scripts/             # Project utility scripts
```

### Development

#### Prerequisites

- Windows 10 or Windows 11
- Node.js and npm
- A stable Rust toolchain
- Microsoft C++ Build Tools and WebView2 as required by Tauri on Windows
- Obsidian and at least two test vaults for end-to-end testing

#### Install Dependencies

```powershell
npm install
```

#### Start Development

Run the Vite frontend only:

```powershell
npm run dev
```

Run the complete Tauri desktop app:

```powershell
npm run tauri dev
```

#### Test and Build

```powershell
# Bridge unit tests
npm run test:bridge

# Rust backend tests
cargo test --manifest-path src-tauri/Cargo.toml

# Bridge + TypeScript + frontend production build
npm run build

# Windows installers
npm run tauri build
```

Generated Windows installers are written to:

```text
src-tauri/target/release/bundle/nsis/Obsidian Plugin Sync_<version>_x64-setup.exe
src-tauri/target/release/bundle/msi/Obsidian Plugin Sync_<version>_x64_en-US.msi
```

Non-major updates keep the current application version by default rather than automatically incrementing the patch version. Builds that share a version should be tracked by Git commit.

### Project Status

- Platform: Windows
- Current version: 0.1.5
- Release notes: [v0.1.5](./RELEASE_NOTES_v0.1.5.md)
- UI language: Chinese-first
- Stage: Active development

Issues and pull requests are welcome through GitHub.

### License

This project is released under the [MIT License](./LICENSE).
