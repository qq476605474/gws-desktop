# GWS

**多仓库需求开发管理：拉代码、合环境、提 MR、写文档。**

面向「一个需求横跨多个仓库」的团队。一个需求一个工作区，只放这次会改的仓库，
各仓库都停在同一需求分支上 —— **全程不用切分支**。

> 📖 [使用教程](docs/使用教程.md) · 🏗 [设计方案](docs/设计方案.md) · 📦 [Releases](https://github.com/qq476605474/gws-desktop/releases)

---

## 它解决什么

一个需求常常横跨七八个仓库：每个仓库开一个分支、切来切去、忘了推、合环境时漏了一个、
提 MR 时把主干名写错（`main` / `master` 混用）……

这个工具把「一个需求涉及的所有仓库」收进**一个工作区（hub）**：

```
<hub>/
├── repos/<仓库>/        生产干净代码（主干镜像，只读）
├── envs/<环境>/<仓库>/   测试环境代码（各环境检出的分支可按仓库单独覆盖）
├── ws/<分支名>/<仓库>/   需求目录 —— 改代码只看这里
└── repos/<文档库>/      文档库（需求文档按分支管在这里；目录名取自你填的地址）
```

- **进 hub 先读根目录的 `AGENTS.md`**：目录语义 + 给 AI 的编码约束（工具生成，随打开刷新）。
  `CLAUDE.md` 只是一句指向它的指针 —— Claude Code 认前者、Codex / Cursor 认后者，
  正文只维护一份，不会再出现两份不同步
- 同一个仓库在 hub 里有好几份副本，**用途完全不同** —— 别改错地方

配套两件东西，内嵌同一份核心逻辑：**桌面端**（不需要额外装任何东西）与 **`gws` CLI**。
桌面端还自带同版本的 CLI 与 AI 技能，安装/升级后启动时自动同步到本机。

---

## 装哪个

从 [Releases 页](https://github.com/qq476605474/gws-desktop/releases) 下载：

| 系统 | 文件 |
|---|---|
| macOS（Apple Silicon / M 系列） | `gws-desktop_x.y.z_aarch64.dmg` |
| Windows 64 位 | `gws-desktop_x.y.z_x64-setup.exe` |
| Linux（Debian / Deepin） | `gws_x.y.z_amd64.deb` |
| macOS 命令行 | `gws-aarch64-apple-darwin` |

装完程序名是「GWS」。几点要知道：

- **macOS 包没有签名**：首次打开请**右键 → 打开**（直接双击会被拦）。
  或执行一次 `xattr -dr com.apple.quarantine /Applications/GWS.app`
- **Linux 的包名是 `gws`** —— Debian 不允许非 ASCII 的包名，所以包名是英文。

命令行版也可以脚本安装（自动选平台资产、校验 sha256）：

```bash
curl -fsSL https://raw.githubusercontent.com/qq476605474/gws-desktop/main/install.sh | sh
gws update --apply     # 之后自更新（有防降级）
```

---

## 第一次用

打开 → 新建 hub，填三样东西：

| 填什么 | 说明 |
|---|---|
| **GitLab 地址** | 你的实例根地址，如 `https://gitlab.example.com` |
| **GitLab token** | Personal Access Token，在 GitLab 的「用户设置 → 访问令牌」里创建，勾 `api` 权限 |
| **文档库地址** | 一个普通的 git 仓库（需求文档按分支存在里面）；没有就先建个空仓库 |

⚠️ **地址与文档库地址创建后就不可改了** —— 它们是「哪些仓库能纳管」和文档挂载位置的
依据。**token 随时可以换**（「设置 → GitLab」或 `gws gitlab token`）。

创建时会先连一次 GitLab 校验地址与 token，通过之后才建目录 ——
校验不通过时磁盘上什么都不会留下。

**不预设任何仓库**：建完到「仓库」页按需添加（支持从 GitLab 检索你有权限的仓库）。
之后照 [使用教程](docs/使用教程.md) 第 3 节走一遍日常流程。

---

## 命令行

桌面端能做的基本都能命令行做，脚本化时更顺手：

```bash
gws init <目录> --gitlab-url https://gitlab.example.com \
                --doc https://gitlab.example.com/team/docs.git
                                    # 建 hub（token 会提示输入，或加 --gitlab-token）
gws gitlab bind --url <地址> --doc <文档库>   # 给已存在的 hub 补绑
gws gitlab token                    # 换 token（地址不可改）
gws gitlab status                   # 看当前绑定与连接状态
gws identity --name 张三 --email z@example.com   # 设提交身份（见下）
gws repo add <git地址>...            # 加仓库（自动检出到各环境）
gws env add <环境>...                # 加环境
gws sync                            # 同步主干与所有环境

gws new [<分支后缀>|--branch <完整分支名>] [--modules a,b] [--from 基线] [--title "需求名称"] [--category 需求分类]
                                    # 不写 --modules 时只挂文档库（与界面默认一致）
gws get <远程分支>                   # 拉别人的需求
gws ls | gws st                      # 列出需求 / 各模块状态
gws pull | gws push                  # 拉/推本需求所有仓库
gws merge <环境> [--push]            # 合到环境（默认不推）
gws mr [--create]                    # 提 MR（先预览）
gws done                             # 检查是否已全部并入主干
gws doc new <类型> | ls | status | sync [--push]
```

全部命令见 `gws --help`；用法与常见问题见 [使用教程](docs/使用教程.md)。

---

## 🔑 GitLab 与凭据（重要）

**凭据跟着 hub 走，没有「全局配置」。**

- **地址**写在 hub 的 `gws.json` 里，建 hub 时固定、**不可改**。
  它同时决定「哪些仓库能被纳管」—— 从别的 GitLab 拉代码会被拒绝。
- **token** 存在 hub 目录下的 `gitlab-token`（Unix 权限 600），
  hub 根的 `.gitignore` 会自动挡住它。
- 环境变量仍然优先：`GWS_GITLAB_TOKEN` > `GITLAB_TOKEN` > hub 凭据文件。
  **但地址不认环境变量** —— 否则「地址不可改」就形同虚设。

这样同一台机器上可以有多个 hub 指向不同实例 / 不同账号，互不干扰。

> 升级过旧版本的注意：早期版本的 token 存在全局的 `~/.config/gws/gitlab-token`。
> 现在每个 hub 一份，打开 hub 后在「设置 → GitLab」里重新填一次即可。

## ✍️ 提交身份（`user.name` / `user.email`）

**也跟 hub 绑定**，与 git 全局配置解耦 —— 同一台机器上有多套身份（公司 / 个人 /
不同 GitLab 账号）时不会串。

```bash
gws identity --name 张三 --email zhangsan@example.com
gws identity            # 查看
gws identity --clear    # 清除，回到 git 全局配置
```

工具会把它写进 hub 里每个仓库的 local config。hub 内的 worktree 与主 clone 共享
同一份 config，所以**你在 `ws/<分支>/<模块>/` 里手工 `git commit` 用的也是这个身份**
—— 这正是「读到错误身份」的主要来源。

---

## 给 AI 用的技能（`skills/`）

`skills/` 下是给 AI 助手用的技能。装进你所用 AI CLI 的 skills 目录后，
AI 就知道该怎么在这个 hub 里干活 —— 改代码只改 `ws/`、文档写哪儿、提 MR 怎么提。
桌面端会同步到本机**已安装**的 CLI：Claude Code `~/.claude/skills`、OpenCode
`~/.config/opencode/skills`、Qoder CLI `~/.qoder/skills`、Codex `~/.codex/skills`
（没有的目录不新建）。

| 技能 | 用途 |
|---|---|
| **`gws`** | 工作区 / 需求 / 环境 / MR / 文档的完整说明，以及 AI 在这个 hub 里的行为约束 |

技能里写死了**与界面一致的约束**：所有字段先问用户（含选填，说明「不填用默认」）、
模块 / 分类必须来自真实列表、危险操作先预览再确认。AI 不该自己编分支名或分类来
「把命令跑通」。

### 怎么安装

**桌面端用户不用手动装**：桌面端内置同版本的 CLI 与技能，安装/升级后启动时
自动覆盖本机旧版本（被改过的技能先备份成 `SKILL.md.gws-backup`；本机版本更新时
不降级；版本号相同但内容变了也会覆盖）。设置页也有「同步 CLI 与 Skills」可以手动跑一次。

没有桌面端 / 想手工装时：

```bash
git clone https://github.com/qq476605474/gws-desktop.git
cd gws-desktop
mkdir -p ~/.claude/skills
cp -R skills/gws ~/.claude/skills/
```

放好后重启 Claude Code（或新开一个会话），技能列表里就会出现 `gws`。

### 怎么用

不用记命令 —— **用平常的话说就行**，技能靠描述里的关键词自动触发：

- 「用 gws 建个需求，改 ad 和 payment 两个仓库」
- 「把这个需求合到 dev，合完推上去」
- 「提 MR，评审人换成 xxx」
- 「拉一下其他人那个 feature-20261010-xxx 的需求」

技能会带着 AI 用 `gws` 的对应命令，并遵守 hub 里那套约定：
**AI 只 `git add`，不 `commit` / `push`**；提交由人 review 后执行。

---

## 开发

```bash
cargo test --workspace                    # Rust 测试（core 无 UI 依赖，可直接测）
npm test                                  # 前端测试（vitest）
npm run check:vue                         # 模板组件导入 / CSS / 类型检查
cargo clippy --workspace --all-targets
npm run tauri dev                         # 开发时启动桌面端
npm run tauri build                       # 打包当前平台
```

改行为前请先读 [开发约定](CLAUDE.md) 与 [设计方案](docs/设计方案.md)
（尤其 §8 的踩坑清单 —— 那 30 多条都是实际踩过的坑）。

依赖方向单向：`cli` → `core` ← `src-tauri`。
`core` **不依赖 Tauri / clap，也不打印 stdout** —— 所有操作返回结构化结果，由调用方渲染。

## 发布

打 `vX.Y.Z` 的 tag 触发 GitHub Actions，三端各自构建（macOS dmg / Windows nsis /
Linux deb + 三个平台的 CLI 二进制），汇总后生成 `sha256sums.txt` 一起挂到 Releases。
桌面端的「一键更新」与 `install.sh` 都按那份校验和验包，对不上就拒绝安装。

## 作者

- GitHub：[@qq476605474](https://github.com/qq476605474)
- B 站：[space.bilibili.com/29222259](https://space.bilibili.com/29222259)
- 项目主页：<https://github.com/qq476605474/gws-desktop>

用着有问题、或者想要什么功能，欢迎提 issue。

## 许可

MIT