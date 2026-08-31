# GWS Desk

**gws 多仓工作区管理工具的桌面客户端**（Tauri 2 + Vue 3 + Pinia + Rust）。

gws 是一个管理「多仓库 + git worktree」的命令行工具：一个 hub 目录统一收管多个代码仓库，按「需求工作区」横向拉通各仓库的同名分支。GWS Desk 把仓库、环境、工作区、文档的日常操作搬进图形界面。

> 📖 **使用手册（面向最终用户）请看 [`README-user.md`](./README-user.md)**；本文件面向开发者与维护者。

## 技术栈

- 前端：Vue 3.5 + Pinia + TypeScript，Vite 5 构建，vitest 4 测试（happy-dom 环境，手写 createApp 挂载，不用 @vue/test-utils）
- 桌面：Tauri 2（Rust），插件：store / dialog / opener / clipboard-manager
- 运行依赖：`gws` CLI 需在宿主机的 PATH 中

## 目录结构

```
src/                  前端
  views/              StartupView（打开/新建 hub）、MainView（四页签布局）
  components/         弹窗（CmdDialog 命令输出、各业务弹窗）与 TopBar
    tabs/             ReposTab / EnvsTab / WorkspacesTab / DocsTab
  stores/             hub（当前 hub 路径）、settings（lastHub/主题/终端）、cmd（execDialog/waitDone）
  lib/                gws-bridge（与 Rust 的命令桥）、busy、confirm、toast、ansi、parse
src-tauri/            Rust 端：gws 子进程编排（run_gws / run_gws_stream / respond_confirm）、
                      事件投递（gws-output:<runId> / gws-exit:<runId> / gws-confirm:<runId>）、
                      shell 探测（终端候选）；54 个测试（含 tauri::test::mock_app 集成）
docs/                 manual-acceptance.md 与 x-turbo/ 仅保留在本地（.gitignore，不随仓库发布）
```

## 核心机制（一句话版）

- 所有 gws 调用走 `lib/gws-bridge.ts`：一次性调用 `run_gws`（busy 遮罩）；长命令用 `run_gws_stream` 拿 runId，输出/退出码经 Tauri 事件推送，前端先订阅再回放（`replayOutput`）防丢；
- 交互确认：Rust 侧按 stdout 静默超时发 `gws-confirm`，前端 `respondConfirm` 写 `y\n` 或杀进程；`confirmTimeoutMs` 可拉长（如 sync 30s）防假确认；
- 命令执行统一弹 CmdDialog 阻塞展示，可终止；全局 `busy` 计数挡住并发操作。

## 开发

前置：Node ≥ 20.12（`.node-version` 固定 22.12.0，建议 volta）、Rust stable、gws 已安装。

```bash
npm install
npm run tauri dev        # 开发（Vite devServer :1420）
```

## 测试与类型检查

```bash
npx vitest run           # 前端：232 用例 / 23 文件
cargo test --manifest-path src-tauri/Cargo.toml   # Rust：54 用例
npm run build            # vue-tsc 类型检查 + vite 产物
```

## 构建发布

产物位于 `src-tauri/target/...`（git 忽略，注意备份）。

**macOS**

```bash
npm run tauri build
# → target/release/bundle/dmg/GwsDesk_0.1.0_aarch64.dmg（+ .app）
```

**Windows x64（从 ARM64 虚拟机交叉编译）**

`bundle.targets` 含 `nsis`。macOS 上无法直接产 Windows 包；我们的做法是在 Windows 11 ARM64 虚拟机里：宿主用 `stable-aarch64-pc-windows-gnullvm`（自带 CRT 可链接宿主侧 build script），目标仍 `x86_64-pc-windows-msvc`（用 MSVC 的 Hostarm64→x64 link.exe），另需补 sysroot 导入库与自写 `windres` 壳。

```
→ target/x86_64-pc-windows-msvc/release/bundle/nsis/GwsDesk_0.1.0_x64-setup.exe
```

两个平台均未做代码签名（Windows 首启有 SmartScreen 提示）。

## 约定与记录

- 界面文案与文档均为中文；验收与反馈处理记录（`docs/manual-acceptance.md`）仅保存在本地仓库，不随开源仓库发布。
- commit 信息风格参考 `git log`（中文，`feat:` / `fix:` 前缀）。
