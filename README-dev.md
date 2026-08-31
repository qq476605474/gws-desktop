# GWS Desk — 开发者文档

**gws 多仓工作区管理工具的桌面客户端**（Tauri 2 + Vue 3 + Pinia + Rust）。

gws 是一个管理「多仓库 + git worktree」的命令行工具：一个 hub 目录统一收管多个代码仓库，按「需求工作区」横向拉通各仓库的同名分支。GWS Desk 把仓库、环境、工作区、文档的日常操作搬进图形界面，本身**不实现任何 git 逻辑**——所有能力都来自调用宿主机上的 `gws` CLI。

> 📖 **使用手册（面向最终用户）请看 [`README.md`](./README.md)**；本文面向开发者与维护者。

---

## 目录

1. [架构总览](#架构总览)
2. [技术栈](#技术栈)
3. [目录结构](#目录结构)
4. [核心机制](#核心机制)
5. [前端模块](#前端模块)
6. [Rust 端模块](#rust-端模块)
7. [开发环境](#开发环境)
8. [测试](#测试)
9. [构建发布](#构建发布)
10. [编码约定](#编码约定)

---

## 架构总览

```
┌────────────────────────── 前端（Vue 3 + Pinia） ──────────────────────────┐
│  views（StartupView / MainView）                                          │
│  components（业务弹窗 / CmdDialog / TopBar / tabs）                        │
│  stores（hub / settings / cmd）              lib/gws-bridge.ts（IPC 桥）   │
└───────────────────────────────┬───────────────────────────────────────────┘
                                │ Tauri IPC（invoke + 事件）
┌───────────────────────────────┴───────────────────────────────────────────┐
│                        Rust 端（src-tauri）                               │
│  gws_runner.rs：gws 进程编排、事件投递、确认交互                            │
│  shell.rs：文件管理器 / 终端 / 剪贴板 / 环境探测                            │
└───────────────────────────────┬───────────────────────────────────────────┘
                                │ 子进程（stdin/stdout/stderr 管道）
┌───────────────────────────────┴───────────────────────────────────────────┐
│                        gws CLI（宿主机，bash 脚本）                        │
│  仓库 / 环境 / 工作区 / 文档 的所有真实操作                                 │
└────────────────────────────────────────────────────────────────────────────┘
```

设计原则：**前端不直接拼 gws 命令细节，后端不做业务判断**。前端通过 `lib/gws-bridge.ts` 把「命令参数 + 工作目录」交给 Rust，Rust 负责启动进程、收集输出、管理交互确认，结果以事件流回传。

---

## 技术栈

| 层 | 技术 | 版本要点 |
|---|---|---|
| 前端框架 | Vue 3 + Pinia + TypeScript | Vue 3.5、Pinia 2 |
| 构建 | Vite | Vite 5，devServer 端口 1420 |
| 测试 | vitest 4 + happy-dom | 手写 createApp 挂载，不用 @vue/test-utils |
| 桌面壳 | Tauri 2（Rust） | tauri-plugin-store / dialog / opener / clipboard-manager / single-instance |
| 外部依赖 | `gws` CLI | 需在宿主机可执行（macOS PATH；Windows 借 Git Bash） |

---

## 目录结构

```
├── src/                      前端（Vue 3 + TS）
│   ├── main.ts               应用入口（createApp + Pinia）
│   ├── router.ts             极简视图路由：startup / main 两个状态
│   ├── App.vue               根组件：busy 遮罩、Toast、路由切换
│   ├── views/
│   │   ├── StartupView.vue   启动页：打开 / 新建 hub
│   │   └── MainView.vue      主界面：四页签布局 + 顶栏
│   ├── components/
│   │   ├── TopBar.vue        顶栏：页签、切换 hub、当前版本、设置
│   │   ├── CmdDialog.vue     命令执行弹窗（流式输出、终止、确认）
│   │   ├── ConfirmDialog.vue gws 交互确认弹窗（继续 y / 取消）
│   │   ├── ConfirmBox.vue    危险操作二次确认
│   │   ├── PathActions.vue   行内小按钮：复制路径 / 打开访达 / 打开终端
│   │   ├── WorkspaceDetail.vue  需求详情（模块表格 + 合并/推送/收尾）
│   │   ├── tabs/             四页签：ReposTab / EnvsTab / WorkspacesTab / DocsTab
│   │   └── *Dialog.vue       业务弹窗：AddRepo / NewWorkspace / AddEnv /
│   │                          Merge / SyncMain / AddModule / AddDoc /
│   │                          GetWorkspace（导入需求）/ SwitchHub / Settings / About
│   ├── stores/               Pinia stores
│   │   ├── hub.ts            当前 hub 路径、打开/切换/新建
│   │   ├── settings.ts       lastHub / 主题 / 终端偏好（自动持久化）
│   │   └── cmd.ts            命令执行状态机（核心，见下文）
│   └── lib/
│       ├── gws-bridge.ts     所有 Tauri invoke 的封装（前端唯一 IPC 入口）
│       ├── busy.ts           全局 busy 计数（App.vue 遮罩）
│       ├── confirm.ts        确认弹窗逻辑
│       ├── toast.ts          轻提示
│       ├── ansi.ts           ANSI 颜色码剥离（命令输出转纯文本）
│       ├── parse.ts          gws 输出解析（状态行、分支信息等）
│       └── consts.ts         常量（如 HUB_ROOT 哨兵值）
├── src-tauri/                Rust 端
│   ├── src/
│   │   ├── main.rs           入口
│   │   ├── lib.rs            Builder：插件注册 + invoke_handler + 单实例
│   │   ├── gws_runner.rs     gws 进程编排与事件投递（核心）
│   │   └── shell.rs          shell 集成（访达/终端/剪贴板/版本探测）
│   └── tests/                Rust 集成测试（tauri::test::mock_app）
└── docs/                     仅本地保留（.gitignore，不随仓库发布）
    ├── manual-acceptance.md  历轮验收与反馈处理记录
    └── x-turbo/              开发计划与设计稿
```

---

## 核心机制

### 1. 命令执行的两条管线

所有 gws 调用走 `lib/gws-bridge.ts`，按命令类型二选一：

| 管线 | 入口 | 适用场景 | 行为 |
|---|---|---|---|
| 一次性 | `runGws(args, cwd)` → `run_gws` | 数据刷新类（列表、状态查询） | Rust 收集全部输出后一次性返回 `{code, output}`；在途期间 `busyCount+1`，全屏遮罩挡住并发操作 |
| 流式 | `runGwsStream(args, cwd, confirmTimeoutMs)` → `run_gws_stream` | 操作类（新建、同步、合并、推送） | 立即返回 `runId`，输出/退出/确认经 Tauri 事件推送，前端实时展示在 CmdDialog |

### 2. 事件协议（流式管线）

Rust 侧为每个 `runId` 发送三类事件：

| 事件名 | payload | 含义 |
|---|---|---|
| `gws-output:<runId>` | `{ chunk: string }` | stdout / stderr 增量输出（UTF-8 安全切块） |
| `gws-exit:<runId>` | `{ code: number \| null }` | 进程退出（恒为该 run 的最后一个事件） |
| `gws-confirm:<runId>` | `{ question: string }` | stdout 静默超过阈值，gws 可能在等 stdin |

**防丢事件的关键：缓存回放。** 前端流程是「先 `invoke(run_gws_stream)` 拿 runId → 再注册三个事件订阅 → 最后 `replayOutput(runId)`」。订阅完成前 Rust 已发出的事件会被缓存，`replay_output` 一次性回放并切换为直发。若没有这一步，快命令在订阅注册前就退出，输出会永远丢失。

### 3. 交互确认（gws 提问）

gws 的命令可能中途提问（如 `gws drop` 问「确认丢弃未推送提交？」）。Rust 侧规则：

- 子进程 stdout **静默超过 `confirm_timeout_ms`** 即发 `gws-confirm:<runId>`（默认 1500ms）；
- 前端 `answerConfirm(yes)` → `respond_confirm`：`yes=true` 向 stdin 写 `y\n`，`false` 杀进程；
- 前端通过 `confirmTimeoutMs` 参数控制阈值，**这是前后端之间的关键契约**：

| 场景 | 阈值 | 原因 |
|---|---|---|
| 操作类命令（execDialog 默认） | 30000ms | sync 的静默 git 阶段、repo add 的 clone 静默可能远超 1500ms，会误弹「等待确认」 |
| 真读 stdin 的命令（`gws drop`） | 1500ms（默认） | 必须快速弹出确认，用户才能回答 |
| 数据刷新类 | 不用流式管线 | 无交互 |

### 4. CmdDialog 与全局并发保护

- 操作类命令统一走 `execDialog`：CmdDialog 全屏遮罩 + 流式输出，**命令结束后必须手动点「关闭」**（防止误点丢失输出）；
- 多命令序列（如 AddModuleDialog 逐模块添加）用 `holdDialog()` / `releaseDialog()` 持有弹窗，序列期间关闭按钮禁用；
- 全局 `busy` 计数：任何在途调用（含数据刷新）都会让 App.vue 的加载遮罩挡住其他操作入口，从根上避免并行操作同一 hub。

### 5. Windows 上的 gws 启动链（GwsLaunch）

gws 是 bash 脚本，Windows 原生 shell 无法直接运行。Rust 端 `find_gws()` 按平台探测，Windows 上按优先级尝试：

```
gws.exe                    → 直接执行（原生可执行）
gws.cmd / gws.bat          → cmd /C 执行
无扩展名 gws（有 shebang）  → bash.exe --noprofile --norc <script>
                            bash 从 C:\Program Files\Git\{bin,usr\bin}\bash.exe
                            或 PATH 探测；path_append 把 Git 的 usr\bin、mingw64\bin
                            追加进子进程 PATH
```

配套两个细节（`GwsLaunch::command`）：

- `CREATE_NO_WINDOW`（0x08000000）：GUI 进程拉起控制台程序时抑制黑色命令窗口弹出（Rust std 不会自动设置该 flag，需要显式加）；
- `tauri-plugin-single-instance`：防止多开 GwsDesk（多实例会带来界面不同步和并行操作风险），重复启动时聚焦已有主窗口。

### 6. gws 查找与版本

- macOS/Linux：PATH 中找 `gws`；
- Windows：PATH → `%USERPROFILE%\.local\bin` → `%USERPROFILE%\bin` 依次探测；
- `check_gws_installed`（启动页检测）、`latest_gws_version`（curl 远端 raw 脚本解析 `GWS_VERSION`，可用 `GWS_UPDATE_URL` 覆盖源，更新按钮执行 `gws update`）。

---

## 前端模块

### stores/cmd.ts（状态机核心）

`CmdRun`：`{ id, label, output, state: running|confirm|done|failed, code }`。`exec()` 的时序要求严格（代码注释里有完整论证）：

1. `run_gws_stream` 必须**在子进程退出前**返回 runId（Rust 侧保证先插入 run 表再 spawn 读线程）；
2. 三个订阅注册完成后才 `replayOutput`（否则订阅前的事件丢失）；
3. `gws-exit` 到达后 `setTimeout(0)` 延迟拆订阅（让在途事件投递全部落地，避免监听表线性增长）；
4. `waitDone(run)` 前置短路：tauri 事件可能先于 invoke promise 决议。

### 数据流示例（以「新建需求」为例）

```
NewWorkspaceDialog.vue
  → store 方法（校验、组装参数）
  → execDialog("新建需求", ["new", ...], hub)
      → gws-bridge.runGwsStream → invoke(run_gws_stream)
      → listen gws-output / gws-exit / gws-confirm（cmd.ts）
      → replayOutput
      → CmdDialog 展示输出 / ConfirmDialog 处理提问
      → 命令结束 → waitDone → 弹窗收尾 → 刷新需求列表（runGws 一次性）
```

---

## Rust 端模块

### gws_runner.rs

| 函数 | 职责 |
|---|---|
| `run_gws` | 一次性执行：阻塞收集输出，返回 `{code, output}`（`#[tauri::command(async)]` 跑线程池） |
| `run_gws_stream` / `spawn_stream` | 流式执行：spawn 子进程 + stdout/stderr 读线程 + 事件推送；返回 runId |
| `replay_output` | 回放订阅前缓存的事件，切换直发 |
| `respond_confirm` | 向 stdin 写 `y\n` 或杀进程 |
| `find_gws` / `find_gws_windows` | 平台化的 gws 启动链探测，返回 `GwsLaunch` |
| `GwsLaunch::command` | 组装 `std::process::Command`（前缀参数、cwd、Windows flag、path_append） |

实现细节：跨块 UTF-8 重组（`take_complete_utf8`，避免中文字符被块边界劈裂成 U+FFFD）；`RUN_ID` 原子自增；`Mutex<HashMap<u32, RunShared>>` 全局 run 表，看门狗在线程排空后自动清理。

### shell.rs

| 函数 | 职责 |
|---|---|
| `open_in_finder` / `open_path` | 文件管理器 / 系统默认应用打开（Windows 走 `cmd /C start ""`，带 CREATE_NO_WINDOW） |
| `open_in_terminal` | 用设置里选的终端打开目录（macOS AppleScript 驱动 iTerm2 / Terminal.app；Windows 探测 wt / cmd） |
| `copy_text` | 写系统剪贴板（绕过 WKWebView 的 JS 剪贴板层——部分环境静默失效） |
| `terminal_options` | 按当前系统实际安装情况生成终端候选（`system` 恒在首位） |
| `check_gws_installed` / `latest_gws_version` | 启动页的 gws 检测与版本检查 |
| `hub_exists` / `read_text_file` / `list_dir` | hub 标记检测、文档读取、目录浏览 |

---

## 开发环境

前置：Node ≥ 20.12（`.node-version` 固定 22.12.0，建议 volta）、Rust stable、宿主机已装 gws。

```bash
npm install
npm run tauri dev        # 开发模式（Vite devServer :1420，改动热更新）
```

常用脚本（`package.json`）：

```bash
npm run dev              # 只跑前端 devServer（联调 UI 用）
npm run build            # vue-tsc 类型检查 + vite 产物
npm run tauri build      # 完整构建安装包（先跑 beforeBuildCommand: npm run build）
```

---

## 测试

```bash
# 前端：232 用例 / 23 文件（vitest + happy-dom，手写 createApp 挂载）
npx vitest run

# Rust：54 用例（单元 + tauri::test::mock_app 集成，mock runtime 直调 spawn_stream，
# 不必篡改进程级 PATH，可与并行测试共存）
cargo test --manifest-path src-tauri/Cargo.toml
```

测试约定：业务弹窗（CmdDialog、各 Dialog、TopBar、tabs）都有对应 `*.test.ts`；Rust 集成测试在 `src-tauri/tests/`，用 mock 可执行文件验证事件编排与确认交互。改代码请同步补/跑测试。

---

## 构建发布

产物位于 `src-tauri/target/...`（git 忽略，注意备份）。两个平台均未做代码签名（Windows 首启有 SmartScreen 提示，macOS 首次打开需右键「打开」）。

**macOS（Apple Silicon）**

```bash
npm run tauri build
# → src-tauri/target/release/bundle/dmg/GwsDesk_0.1.0_aarch64.dmg（+ .app）
```

**Windows x64（从 ARM64 虚拟机交叉编译）**

macOS 上无法直接产 Windows 包，做法是在 Windows 11 ARM64 虚拟机里交叉编译：

- 宿主工具链：`stable-aarch64-pc-windows-gnullvm`（自带 CRT，可链接宿主侧 build script）；
- 目标：`x86_64-pc-windows-msvc`（用 MSVC 的 Hostarm64→x64 link.exe）；
- 需补 sysroot 导入库，并自写 `windres` 壳（把 `--input/--output/-D/-I` 翻译成 `rc.exe /nologo /r /fo`，产出 x64 静态 CRT 单文件）；
- 完整步骤、坑与脚本记录在本地 `docs/manual-acceptance.md`（第十五节，不随仓库发布）。

产物：`src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/GwsDesk_0.1.0_x64-setup.exe`

**发布流程**：打包 → 本机自测 →（可选）拷到 Windows 虚拟机 C:\tools 自测 → `gh release create` 上传两个安装包（历史发布见 GitHub Releases）。

---

## 编码约定

- **界面文案与文档均为中文**；commit 信息风格参考 `git log`：中文，`feat:` / `fix:` / `docs:` / `chore:` 前缀；
- 代码注释默认不写；写注释时必须说明 WHY（隐藏约束、微妙不变量、某个 bug 的 workaround），参考 `cmd.ts` / `gws_runner.rs` 里的事件时序注释；
- 前端所有 IPC 必须经 `lib/gws-bridge.ts`，不直接 `invoke`；新命令要同步补 bridge 函数、store 调用、测试；
- 错误信息面向用户用中文；Rust 侧返回 `Result<_, String>`（中文文案），不 resolve 空值冒充成功；
- `docs/manual-acceptance.md` 与 `docs/x-turbo/` **仅本地保留**（含私有环境信息），新功能验收记录写在那里，不要提交到开源仓库。
