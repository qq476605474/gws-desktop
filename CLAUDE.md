# gws 开发约定

面向在本仓库工作的 AI 助手。完整设计见 `docs/设计方案.md`。

## 提交约定

**本仓库是 gws 自身的开发仓库，AI 可以正常 `git commit`。**

> ⚠️ **注意区分**：本项目产出的工具里有一条「AI 只 `git add`，不得 `commit` /
> `push`」的规则（会写进每个需求工作区的 `AGENTS.md`）。
> **那条规则约束的是「用本工具开发业务需求」的场景**，即使用者让 AI 改
> `ws/<分支名>/` 下的业务代码时 —— 使用者的痛点是 AI 未经 review 就把业务代码
> 推上了远程。**它不是对本仓库开发的约束。**

推送到远程前仍建议与维护者确认。

## 架构约束

改动前请先确认不破坏这几条：

1. **`crates/core` 不依赖 Tauri、不依赖 clap。** 上层怎么用是上层的事。
2. **`crates/core` 不直接打印到 stdout。** 所有操作返回 `serde::Serialize` 的结构化
   结果，由调用方决定渲染成人类可读文本还是 JSON。早期版本让前端正则解析 ANSI
   彩色文本，是主要的脆弱来源，本版明确消除。
3. **依赖方向单向**：`cli` → `core` ← `src-tauri`。不允许反向。
4. **凭据与身份都跟着 hub 走，没有「全局配置」。** GitLab 地址、token、提交身份
   都存在 hub 里（见 `hub::config`）。新增任何「要用网络 / 要以某个身份写 git」
   的功能时，**必须先取 hub**（`Client::for_hub`、`git::apply_identity`），
   不要复活一个全局入口 —— 同一台机器上多套凭据/身份串起来极难排查。

## 设计纪律

本项目的核心逻辑是**参照一套已在真实团队里跑通的行为**重写的，不是凭空设计。
改核心行为前：

- 先读 `docs/设计方案.md` §8「行为对照与必搬踩坑清单」——那些是实际踩过的坑，
  对应的注释必须留在实现里。
- 若确实要改变既有行为，在设计方案里补一条说明，不要静默改。

## 错误处理

早期实现最典型的缺陷是**失败还报成功**（设计方案 §8 第 11 条：`rm -rf` 无检查却
无条件打印「已删除」）。因此：

- 所有可能失败的操作必须返回 `Result`。
- 删除类操作**必须删除后校验**目标是否真的消失。
- 不允许 `let _ = ...` 吞掉错误后再打印成功。

## 测试

- 核心逻辑用**临时 hub** 做集成测试（`core` 无 UI 依赖，可直接测）。
- 涉及 git 的测试在 `tempfile` 目录里建真实的裸仓库 + worktree，不要 mock git。
- **不要假设环境变量是干净的**：开发机上完全可能 `export GITLAB_TOKEN=...`。
  测试里凡是断言「没配凭据」的地方，都会被这种机器打成假红 —— 改为直接断言
  文件内容（`read_file`）或注入依赖，而不是断言 `resolve()` 的结果。
- 涉及「地址/身份从哪来」的测试，用**直接写 hub 配置**的方式造夹具
  （`hub.config_mut().gitlab.base_url = ...` + `save_config()`），
  因为 `Hub::init` 的绑定路径会联网校验。

## 打包的一个偶发坑

`npm run tauri build` 偶尔会死在最后一步：

```
failed to bundle project: error running bundle_dmg.sh
```

**`.app` 其实已经构建好了**（dmg 是最后一步），代码没问题。
原因：`bundle_dmg.sh` 会用 AppleScript 设置 dmg 窗口的外观，那一步需要
GUI 会话；在没有交互会话的上下文里（后台任务、CI 的 headless runner）
偶发失败。

应对：**重跑一次**即可（实测：失败一次后原样重跑就成功了）。
判断 `dist` 有没有真出问题，看 `.app` 在不在、时间戳新不新 ——
另加 `dmg` 时手动跑一遍 `target/release/bundle/dmg/bundle_dmg.sh` 也能出。

## 三端打包

**发布走 GitHub Actions**（`.github/workflows/release.yml`）：打 `vX.Y.Z` 的 tag，
三个平台各在自己的 runner 上构建（macOS dmg / Windows nsis / Ubuntu deb），
产物汇总后由最后一个 job 生成 `sha256sums.txt` 并建 Release。
**产物名必须与 `crates/core/src/update.rs` 的 `desktop_asset_name()` /
`asset_name()` 一致** —— 差一个字符，用户点「更新」就报「该 Release 里没有适配
当前平台的资产」。CI 里有一步专门做重命名，别删。

本地只出 macOS 包：`npm run tauri build`（`--bundles app,dmg`）。

### 如果你要在自己的 Windows / Linux 机器上本机构建

这几条是实测踩过的，与构建环境有关、与代码无关：

- **从 macOS 传源码过去必须加 `COPYFILE_DISABLE=1`**：
  ```sh
  COPYFILE_DISABLE=1 tar --exclude=./target --exclude=./node_modules \
    --exclude=./.git --exclude=./dist -czf /tmp/gws-src.tgz .
  ```
  macOS 的 `bsdtar` 会把扩展属性转成归档里的 `._*` 条目（**磁盘上根本没有这些
  文件**，所以本机 `find` 查不出来）。解到别的系统就成了真文件，而 Tauri 会把
  `src-tauri/capabilities/` 下的**每一个**文件当 capability 读，于是构建死在：
  `failed to read file 'capabilities/._default.json': stream did not contain valid UTF-8`。
  这个坑踩过两次（Linux 和 Windows 各一次）。
- 传文件后**核对 sha256**：Windows 上 scp 覆盖同名文件出现过静默不生效，
  只比「命令退出码 0」会误判成传成功了。
- Windows 上的 `.ps1` / `.bat` 一律用**纯 ASCII**。PowerShell 5.1 按 ANSI（中文系统
  上是 GBK）读 `.ps1`，cmd 同样按 ANSI 读 `.bat` —— 无 BOM 的 UTF-8 中文会变成
  语法错误，而且报错是乱码，很难看出是编码问题。这个坑踩过两次。
- Windows 上跑长任务（下载工具链、编译）**不要用 `Start-Process` 或挂在 SSH 会话上** ——
  SSH 一断进程就被回收。用 `schtasks /Create /TR <批处理> /SC ONCE` 再 `/Run`，
  跑在计划任务服务下才真正脱离会话。
- **ARM64 机器出 x64 包：换 x64 工具链，不是加 `--target`。**
  ARM 版 Windows 的 MSVC 库里**只有 `x64`/`x86`，没有 `arm64`**
  （`VC\Tools\MSVC\<ver>\lib\` 下没有 arm64 子目录）。而 **build script 是给 host
  编的**，host 是 aarch64 时连脚本都链不出来：
  `rust-lld: error: could not open 'msvcrt.lib': no such file or directory`。
  所以光加 `--target x86_64-pc-windows-msvc` 没用，得让 **host 本身就是 x64**：
  ```bat
  rustup toolchain install stable-x86_64-pc-windows-msvc --profile minimal --force-non-host
  set "RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc"
  npm run tauri build -- --bundles nsis
  ```
  `--force-non-host` 是必须的（rustup 认为 x64 工具链「跑不了」，其实靠
  `C:\Windows\System32\xtajit64.dll` 那层 x64 仿真跑得好好的）。
  host == target 之后就不需要 `--target` 了。
- **rustup 不肯修复半个工具链**：装到一半中断后再装，它会说
  `unchanged - (error reading rustc version)` 而**不补下缺失的组件**。
  得先 `rmdir /s /q %USERPROFILE%\.rustup\toolchains\<toolchain>` 再装。
  另外 `RUSTUP_DIST_SERVER` 在国内极慢，必要时换镜像。