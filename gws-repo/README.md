# gws — 多仓库需求工作区管理器

**gws** = **g**it **w**ork**s**pace，即「用 Git 管理工作空间」的命令行工具。

基于 `git worktree` 构建，面向「一个需求横跨多个仓库」的微服务团队。这里的**工作空间**指一个需求所对应的完整开发上下文：它涉及哪些仓库、各仓库停在哪个分支、配套文档放在何处，均作为一个整体被管理。

**核心模型：一个需求 = 一个工作区 = 一组 worktree，永不切分支。**

零依赖单文件 Bash 脚本，无需运行时、无需配置文件。

---

## 1. 背景与设计动机

### 1.1 问题场景

在多仓库（microservices / 多前端）架构下，一个业务需求通常需要同时改动若干个仓库。传统的「每个仓库一份检出 + 按需切分支」工作流存在以下问题：

| 问题 | 具体表现 |
|---|---|
| **需求无法并行** | 同一份工作副本同时只能停在一个分支上，插入紧急需求必须 stash 或提交半成品 |
| **切分支成本高** | 跨 N 个仓库的需求，切换上下文需执行 N 次 `checkout`，且极易遗漏某个仓库 |
| **IDE 负载过重** | 为覆盖需求涉及的模块，往往需要一次性打开全部工程，索引耗时且占用大量内存 |
| **构建产物污染** | 切分支不会清理 `target/`、`node_modules/` 等未跟踪产物，导致难以定位的构建异常 |
| **依赖关系难表达** | 分阶段需求（阶段 2 基于未上线的阶段 1）缺乏统一的基线管理方式 |

### 1.2 解决思路

`gws` 使用 `git worktree` 为每个需求创建独立的工作目录，其中**仅包含该需求实际会改动的仓库**：

- **物理隔离** — 需求之间互不干扰，可任意数量并行，切换需求即切换目录
- **范围收敛** — IDE 打开的目录即改动范围，索引范围与认知负担同步收窄
- **分支固化** — 工作区内各模块永久停在同一 feature 分支，从机制上消除误切分支
- **对象库共享** — 所有 worktree 共用 `repos/` 下的单一 `.git`，磁盘开销远低于重复克隆
- **生命周期完整** — 需求的创建、模块增删、阶段依赖、环境合并、归档删除均由单一命令入口覆盖

---

## 2. 安装

`gws` 是单文件 Bash 脚本，零依赖部署：复制即可用，可管理任意数量的 hub。

**环境要求**：Bash 4+、Git 2.17+（`worktree` 完整支持；`gws rename` 需 2.30+ 的 `worktree repair`）。`gws update` 需要 `curl`。

```bash
# 1. 安装（macOS / Linux，需 curl）
mkdir -p ~/.local/bin
curl -fsSL https://raw.githubusercontent.com/qq476605474/gws/main/gws -o ~/.local/bin/gws
chmod +x ~/.local/bin/gws
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc && source ~/.zshrc

#    或一键安装（等价）：
#    curl -fsSL https://raw.githubusercontent.com/qq476605474/gws/main/install.sh | sh
#
#    也可以直接拿到别人给的 gws 文件：复制到 ~/.local/bin/ 并 chmod +x 即可，
#    之后同样能用 gws update 升级到最新版。

# 2. 创建 hub（省略目录参数则交互式询问）
gws init ~/Documents/dev/myhub
cd ~/Documents/dev/myhub

# 3. 添加仓库（支持一次传入多个地址）
gws repo add git@git.example.com:group/order-service.git \
              git@git.example.com:group/user-service.git

# 4. 按需添加环境分支（init 默认仅创建 dev），并建立环境 worktree
gws env add pre
gws sync

# 5. 开始工作：新建需求，或拉取他人已推送的需求
gws new checkout-revamp
gws get feature-20260818-checkout-revamp
```

升级到最新版本：`gws update`；查看版本：`gws version`。脚本来源于 GitHub（`github.com/qq476605474/gws`），`GWS_UPDATE_URL` 可覆盖更新源。

> **术语：hub** — 由 `gws init` 创建的根目录，统一存放仓库、环境副本与需求工作区。下文示例中以 `myhub` 指代。

---

## 3. 目录结构

`gws` 脚本位于 `~/.local/bin/gws`，**独立于 hub 之外**，因此 hub 可以自由重建、迁移，或多个并存。

```
myhub/                        # 由 gws init 创建
├── .gws-hub                  # 标记文件：gws 据此向上定位 hub 根目录，勿删
├── repos/                    # 唯一的 .git 存放处，由 gws repo add 添加
│   ├── order-service/            停在主干，不做任何开发
│   ├── user-service/
│   └── admin-web/
├── envs/                     # 环境副本，一个子目录 = 一个环境分支
│   ├── dev/                      内含全部仓库的 worktree，恒等于 origin/dev
│   │   ├── order-service/
│   │   └── ...
│   └── pre/
├── ws/                       # 需求工作区，每个需求一个目录
│   └── checkout-revamp/          仅包含该需求会改动的模块
│       ├── .workspace.json       需求元数据（模块列表、分支、基线）
│       ├── order-service/        固定在 feature-20260818-checkout-revamp
│       ├── user-service/
│       └── docs -> ../../docs/2026-08-18-checkout-revamp/
└── docs/                     # 独立 git 仓库，集中管理文档
    ├── 2026-08-18-checkout-revamp/
    └── _archive/                 需求删除后文档归档至此，不物理删除
```

**hub 定位规则**：`gws init` / `gws migrate` 可在任意目录执行；其余命令必须在 hub 目录内（自动向上查找 `.gws-hub`），或显式指定 `--hub <路径>`。

### 3.1 三条核心约定

1. **`repos/` 仅作为对象库，不在其中开发。** 所有 worktree 共用这里的 `.git`，各仓库恒定停留在主干。
2. **`envs/<环境>` 是远程同名分支的只读镜像。** `gws sync` 会无条件 `reset --hard`，因此上游分支被重建也能自动恢复。**不在 envs 内编写业务代码或手工提交**（唯一例外见 §10.2）。
3. **`ws/<需求>/` 内各模块固定在同一 feature 分支，不切分支。** IDE 打开工作区目录，检索范围即改动范围。

---

## 4. 命令参考

> 除 `init` / `migrate` 外，所有命令需在 hub 内执行；工作区级命令（`st` / `push` / `merge` 等）还需先 `cd ws/<需求名>/`。

### 4.1 hub / 仓库 / 环境管理

| 命令 | 作用 |
|---|---|
| `gws init [目录] [--envs dev,pre]` | 创建标准 hub 结构（repos/envs/ws/docs + `.gws-hub` + docs git）。不写目录则交互询问。**默认只建 dev 环境**。目标已是 hub、或位于其他 hub 内部（禁止嵌套）时拒绝创建；目录非空会要求确认 |
| `gws migrate <新hub> [--move\|--clone]` | 迁移到新 hub。`--move`（默认）直接移动 repos，快；`--clone` 按 remote 重新 clone，最干净。**ws/ 需求不迁移** |
| `gws repo add <git地址>...` | clone 仓库到 `repos/`（可一次多个），完成后提示跑 `gws sync` |
| `gws repo ls` / `gws repo rm <仓库名>...` | 列出 / 移除仓库（仍被工作区占用时拒删） |
| `gws env add <环境>...` | 增加环境（如 `pre`、`dev1`、`dev_tmp`），自动建各仓库 worktree。远程无该分支会先警告 |
| `gws env ls` / `gws env rm <环境>...` | 列出 / 移除环境。移除前检查未推送的合并成果 |
| `gws version` / `gws update` | 查看版本 / 从 GitHub 拉取最新脚本自替换（需 `curl`；仓库 `github.com/qq476605474/gws`，主源不可达时自动走镜像，`GWS_UPDATE_URL` 可覆盖源） |

> 环境列表 = `envs/` 下的目录，每个 hub 各自独立，无配置文件。

### 4.2 需求生命周期

| 命令 | 作用 |
|---|---|
| `gws new <名称> [--modules a,b,c] [--from 基线[,基线]...] [--title "中文标题"] [--prefix <前缀>] [--branch <分支名>]` | 建工作区。不写 `--modules` 默认**全部仓库**。每模块创建一个 `feature-YYYYMMDD-<名称>` 分支（**未推送**），从主干或 `--from` 基线拉取。**`--from` 可填多个基线，逗号分隔，顺序即优先级**——逐个检查该模块是否有对应分支，命中即用，全不命中自动兜底主干（见 §7.3） |
| `gws get <feature分支> [--name <工作区名>] [--title "标题"]` | **拉取同事/其他电脑已推送的需求**。自动扫描哪些仓库有该远程分支来确定模块集合，upstream 绑定到 `origin/<feature>`。工作区名默认从分支名反推 |
| `gws rename <旧名> <新名> [--branch <新分支名>] [--title "新标题"]` | 工作区改名：目录、feature 分支、`.workspace.json`、docs 目录与软链、worktree 元数据一并处理；其他需求对它的 `ws:` 依赖也自动改指向 |
| `gws add <模块>...` | 中途增加模块（复用已有同名 feature 分支，或从工作区基线新建；基线不存在则降级主干） |
| `gws drop [<模块>...]` | 移除模块。**带参数**：有未提交改动/领先主干提交会警告要确认；**无参数**：清理被直接 `rm -rf` 删掉目录的缺失模块（见 §7.2） |
| `gws ls` | 列出所有工作区（名称/标题/阶段/模块数/分支） |
| `gws st` | 当前工作区各模块状态：改动数 / vs远程(↑↓) / vs主干领先数 |
| `gws rm <名称>... [--force]` | 删除工作区（可一次删多个）：文档自动归档到 `docs/_archive/`；有未保存工作（未提交/未推送）则拒删，`--force` 强删。若远程分支仍在会提示清理命令 |
| `gws done` | 校验各模块是否均已并入主干，未并入者会逐一列出 |

### 4.3 协作与合并发布

| 命令 | 作用 |
|---|---|
| `gws pull [--rebase]` | **协作场景必备**：拉取他人推送到同一 feature 分支的提交并合并到本地各模块。存在未提交改动的模块会跳过并提示；冲突直接在工作区内解决 |
| `gws push` | 推送所有模块的 feature 分支到远程（有未提交改动会先拒推） |
| `gws merge <环境>` | 合入环境分支。**默认只合并到本地 envs（不推、不询问）**，review 后再推送。只从 `origin/<feature>` 合并，未 push 则合不进去；envs 先 `reset --hard origin/<环境>` + clean 再合并 |
| `gws merge <环境> --push` | 合并并推送，一条命令完成。`--yes` 用于脚本 / CI |
| `gws merge <环境> --force` | 丢弃 envs 本地未推送的合并成果，从远程重新合并 |
| `gws sync-main [--from <ref>] [--yes]` | 将来源分支的最新代码合入当前工作区（长周期需求防冲突）。默认取创建基线：`主干` → `origin/<主干>`；`ws:<上游>` → 上游工作区的 feature 分支。可用 `--from` 覆盖，基线缺失时降级主干。**有未提交改动不跳过**，会列出模块并询问一次，确认后自动 stash → 合并 → 回填（`--yes` 跳过询问）。冲突处理见 §6.3 |
| `gws sync` | repos 与 envs 全部 reset 到对应远程分支；**新增仓库自动补建各环境 worktree**。逐模块列出代码前进/回退的提交数；有模块失败时返回非 0（见 §6.4）|

### 4.4 文档（详见 §8）

| 命令 | 作用 |
|---|---|
| `gws doc new <文件名>` | 新建带 frontmatter 的 MD（含 `confluence.page_id` 空位），自动 commit 纳管 |
| `gws doc ls` | 列出当前工作区文档及 wiki 同步状态（● 已上传 / ○ 未上传） |
| `gws doc push [文件]` | 自动纳入 git 并上传 Confluence（哈希未变跳过上传，成功后回写映射） |
| `gws doc commit [说明]` | 把手动/AI 写入工作区 `docs/` 的文档纳入 docs git 仓库（只纳管不上传） |

---

## 5. 完整工作流（一个需求的全程）

### ① 建工作区

```bash
gws new checkout-revamp --modules order-service,user-service --title "结算流程改版"

# 或先纳入全部仓库，再删去用不到的模块：
#   gws new checkout-revamp --title "结算流程改版"
#   gws drop admin-web report-service
```

创建后输出分支名 `feature-YYYYMMDD-checkout-revamp`、文档目录及后续提示。

### ② 开发

```bash
cd ws/checkout-revamp/
```

用 IDE 打开该目录：其中只有本需求会改动的模块，各模块固定在同一 feature 分支，**无需也不应切分支**。需要并行多个需求时重复执行 `gws new` 即可。

### ③ 编写文档

```bash
gws doc new 技术方案.md     # 或直接在 docs/ 下新建文件
gws doc push                # 纳入 git 并同步至 Confluence
```

### ④ 联调

另开一个 IDE 窗口指向 `envs/dev/` 运行完整环境。该目录是远程分支的镜像，**不在其中开发**；需要刷新时执行 `gws sync`。

### ⑤ 提交并推送

```bash
gws st        # 查看各模块改动状态
gws push      # 推送所有模块的 feature 分支
```

### ⑥ 合并至 dev

```bash
gws merge dev          # 仅合并到本地 envs/dev，不推送
                       # 在 envs/dev 中 review 变更
gws merge dev --push   # 确认无误后推送
```

若无需 review，`gws merge dev --push` 可一步完成合并与推送。

冲突处理见 §6。

### ⑦ 提测与缺陷修复

缺陷修复回到工作区进行，随后 `gws push` → `gws merge dev --push`。**不在 envs 内直接修改**。

### ⑧ 灰度发布

```bash
gws merge pre --push
```

### ⑨ 上线

通过代码托管平台发起合并请求（Merge / Pull Request）并完成评审，`gws` 不介入这一环节。合并后执行 `gws done` 校验各模块是否均已并入主干。

### ⑩ 收尾

```bash
gws rm checkout-revamp     # 文档自动归档至 docs/_archive/
```

---

## 6. 冲突处理

`gws merge <环境>` 遇到冲突时会中止并列出冲突模块与文件路径（位于 `envs/<环境>/<模块>/`）：

1. 用 IDE 打开提示的目录
2. 使用其合并工具解决冲突（提交与否均可）
3. 回到需求工作区重新执行 `gws merge <环境> --push`

第二次执行时会自动判断状态：已提交则直接推送；已解决但未提交则自动补齐 merge commit 后推送；仍有未解决文件则再次列出，不破坏现场。因此**整个流程只需记住 `gws merge <环境> [--push]` 一条命令**。

两种可选节奏：

| 方式 | 适用场景 |
|---|---|
| `gws merge dev` → 检查 → `gws merge dev --push` | 需要在推送前 review 合并结果 |
| `gws merge dev --push` | 变更简单，合并与推送一次完成 |

`--yes` 为非交互确认，供脚本与 CI 使用。

### 6.1 安全保障

- **未推送的合并成果不会被静默丢弃。** 再次执行 `gws merge` 时会列出「待推送成果」，由使用者决定 `--push` 推送或 `--force` 丢弃重来，不会 `reset --hard` 覆盖已完成的冲突解决工作。
- **部分解决冲突时不会破坏现场。** 命令会停下并列出仍未解决的文件。
- **上游分支被重建（强推至分叉基线）时自动识别并重置。** 此时本地副本已失效，命令会直接 `reset --hard` 与 `clean` 同步至新的远程状态，不会误判为「待推送成果」而提示推回失效代码。

### 6.2 边界约定

在 envs 内解决合并冲突是允许的——冲突解决本就是合并动作的组成部分。但**选错合并方案、编译不通过一类的修复必须回到 feature 分支进行**，然后重新执行合并流程。envs 中不应存在任何业务代码改动。

### 6.3 `gws sync-main` 的冲突

与 `gws merge` 不同，`sync-main` 是把来源分支合进**你正在写代码的工作区**，因此允许工作树带着未提交改动执行——效果等同于你自己在该目录下 `git merge origin/main`。命令会先列出有未提交改动的模块并询问一次（`--yes` 跳过），确认后按模块执行 `git merge --autostash`：先替你 stash，合并，再把改动回填。

结果分三类，处理方式不同，**看命令输出的括号标注即可对号入座**：

| 输出标注 | 含义 | 处理方式 |
|---|---|---|
| `(合并冲突)` | 来源的提交与你**已提交**的提交改了同一处 | IDE 解决后 `git commit`。你原先未提交的改动此刻被 git 暂存在 `MERGE_AUTOSTASH`，**工作树里暂时看不见是正常的**，commit 完会自动回填 |
| `(改动回填冲突)` | 合并本身已成功，是你**未提交**的改动与合进来的内容撞在同一处 | IDE 解决后 `git add` 即可，**不要 commit**（没有待完成的 merge）。git 出于安全会留下 stash 条目，再执行 `git stash drop` 清掉 |
| `(未开始合并)` | 合并根本没启动，工作树未被改动 | 多为本地**未跟踪文件**与来源新增文件同名（`--autostash` 不管未跟踪文件）。按输出中 git 的原始提示处理后重跑 |

三类都会在命令末尾汇总列出模块名与对应提示，退出码非 0。

> **注意**：`(合并冲突)` 时 `git commit` 而非 `git add` 是关键——merge 尚未完成，只 `git add` 会让工作区停在半合并状态，且未提交改动不会回填。

### 6.4 `gws sync` 的失败与自愈

`gws sync` 是**唯一**更新 envs 代码的命令，没有别的入口。它对每个环境的每个模块执行 `reset --hard origin/<环境>`，并逐模块报出代码走了多远：

```
▸ 同步 envs (reset 到远程，新增仓库自动补建)
✓ envs/dev   更新 3 个 / 新增 0 / 已最新 17
     qlchat-woman-university +4
     xqd-java-ad             -2  (远程回退)
     xqd-payment             +2 -1  (远程已重建/强推，本地按远程为准)
```

**负数是正常的**：envs 是远程的纯镜像，别人删分支重建、或强推回退时，本地跟着回退才是正确行为。之所以把它显式打出来，是因为「提交被抹掉了」这件事不该悄无声息地发生——看到 `-N` 时确认一下是不是预期内的重建即可。

`reset` 失败时（残留 `index.lock`、worktree 元数据损坏等）分两种处理：

| 情形 | 行为 |
|---|---|
| 该模块**无本地改动、无未推送提交** | 自动删目录 → `worktree prune` → 重新 add 自愈，标注 `(reset 失败，已自动重建)`，无需人工介入 |
| 该模块**有未推送的成果** | **不动现场**，打印 git 原始报错并提示：先 `gws merge <环境> --push` 推走，或确认无需保留后 `rm -rf envs/<环境>/<模块> && gws sync` |

只要有任一模块失败，命令返回非 0（`repos` 的 fetch 失败同样计入）。

> 之所以要区分，是因为 §6.2 允许在 envs 内解决合并冲突——那里可能存着尚未推送的冲突解决成果，无条件重建会把它抹掉。

---

## 7. 常见场景

### 7.1 中途需要增加模块

```bash
gws add payment-service
```

### 7.2 移除不需要的模块

```bash
# 方式一：显式移除（存在未提交改动或领先主干的提交时会要求确认）
gws drop admin-web

# 方式二：直接删除目录，再执行无参数清理
rm -rf admin-web
gws drop      # 清理 worktree 元数据、删除分支并回写模块列表
```

> 方式二中，若被删目录的分支尚有未推送的提交，则**保留该分支不删除**（可通过 `gws add` 恢复），以免丢失代码。

### 7.3 分阶段需求（阶段依赖）

阶段 2 需要建立在尚未上线的阶段 1 之上时，以阶段 1 的分支作为基线：

```bash
gws new checkout-phase2 --modules order-service,user-service,payment-service \
                        --from ws:checkout-phase1
```

**模块集合不一致时自动降级。** 若阶段 1 只涉及 `order-service` 与 `user-service`，而阶段 2 还需改动 `payment-service` —— 后者不存在阶段 1 的分支，将自动以主干为基线，并在输出中标注：

```
✓ order-service    ← origin/feature-20260801-checkout-phase1
✓ user-service     ← origin/feature-20260801-checkout-phase1
✓ payment-service  ← origin/main (上游无此分支，降级主干)
```

`gws add` 中途增加模块、`gws sync-main` 同步基线时遵循同一规则。

**三级（或更长）阶段链条。** 阶段 3 可能同时依赖阶段 2 与阶段 1——不同模块在不同阶段才被引入（如阶段 1 动了 A/B，阶段 2 动了 B/C，阶段 3 还要加 D）。此时按优先级列出基线即可，主干作为最终兜底**无需填写**：

```bash
gws new checkout-phase3 --modules order-service,user-service,admin-web,report-service \
                        --from ws:checkout-phase2,ws:checkout-phase1
```

各模块按顺序取**第一个分支存在**的来源，输出会标注每一处回退：

```
✓ order-service    ← origin/feature-20260815-checkout-phase1  (按优先级回退（ws:checkout-phase2 无此分支）)
✓ user-service     ← origin/feature-20260815-checkout-phase2
✓ admin-web        ← origin/feature-20260815-checkout-phase2
✓ report-service   ← origin/main                              (上游全部无此分支，降级主干)
```

该链同时写入 `.workspace.json` 的 `base` 字段，`gws sync-main` 默认按**同一条链**逐模块取第一命中的来源同步，上游阶段上线后可显式缩链（如改为 `--from main`）。

### 7.4 长周期需求防冲突

定期将来源分支的最新代码合入工作区。**来源默认取创建时的基线**（`gws new` 的 `--from`），无需重复指定：

```bash
gws sync-main              # 普通需求 → origin/<主干>
                           # 阶段依赖需求 → 上游工作区的 feature 分支
```

分阶段需求依靠该命令保持同步：上游阶段尚未上线但持续变更时，在下游阶段执行 `gws sync-main` 即可合入其最新代码。待上游阶段上线后，显式切回主干：

```bash
gws sync-main --from main              # 上游已上线，改以主干为来源
gws sync-main --from release-2026-09   # 也可指定任意分支
```

**不必为了同步先提交代码。** 手头有写到一半的改动时照常执行即可，命令会列出这些模块并问一句：

```
⚠ 以下模块有未提交改动:
     order-service: 3 个未提交改动
     会自动 stash → 合并 → 回填（git merge --autostash）；
     若与来源改到同一处，冲突会留在工作树里等你手动解决
     是否继续? [y/N]
```

答 `y` 后即使冲突也**会把来源代码合下来**，把冲突留在工作树里由你在 IDE 中解决，不会把你挡在门外。三类冲突的具体处理方式见 §6.3。脚本 / CI 场景加 `--yes` 跳过询问；非交互环境下不加 `--yes` 则直接退出、不动工作树。

### 7.5 新增仓库

```bash
gws repo add git@git.example.com:group/payment-service.git   # clone 至 repos/
gws sync                                                      # 补建各环境 worktree
gws new <需求> --modules payment-service                      # 即可使用
```

手动 `git clone` 到 `repos/` 同样有效——仓库列表在运行时通过读取 `repos/` 目录动态识别，无需注册。

### 7.6 多人协作同一需求

当需求已由他人创建并推送时，拉取到本地参与开发：

```bash
gws get feature-20260818-checkout-revamp    # 自动确定涉及哪些仓库
cd ws/checkout-revamp/
```

此后在每次开始工作前同步他人的提交：

```bash
gws pull            # fetch 并 merge origin/<feature> 至各模块
gws pull --rebase   # 需要线性历史时使用
```

冲突文件直接列在工作区内（无需进入 envs），用 IDE 解决并提交即可。

**协作时的标准顺序：**

```
gws pull  →  改代码 / 提交  →  gws push  →  gws merge <环境>
```

`gws push` 与 `gws merge` 均会在执行前检查本地 feature 分支是否落后于远程。若他人已推送新提交，命令会中止并提示先执行 `gws pull`，以免用较旧的代码覆盖其成果。

> `gws get` 创建的工作区已将上游绑定至 `origin/<feature>`，因此 IDE 内置的 Pull / Push 操作可直接使用。

### 7.7 重建或迁移 hub

当 hub 目录需要整理或迁移位置时：

```bash
gws migrate ~/Documents/dev/myhub-new            # 默认 --move，直接移动 repos
gws migrate ~/Documents/dev/myhub-new --clone    # 按 remote 重新 clone，状态最干净
cd ~/Documents/dev/myhub-new && gws sync
```

**`ws/` 下的需求工作区不会迁移**，因为其 worktree 元数据绑定于原 hub。已推送的需求在新 hub 中通过 `gws get <分支>` 恢复；尚未推送的需求应在迁移前执行 `gws push`。

### 7.8 环境分支被重建或强推

上游环境分支被重建时，两条路径均可自动恢复：

- `gws sync` — 无条件 `reset --hard origin/<环境>`，使 envs 恒等于远程状态
- `gws merge <环境>` — 检测到本地 HEAD 不再包含远程 tip 时，自动重置该模块后再合并

feature 分支不受影响。

### 7.9 自定义分支命名（hotfix 等）

默认分支名为 `feature-YYYYMMDD-<需求名>`。紧急修复、发布分支等场景可通过两种方式调整：

```bash
# 方式一：更换前缀，保留「前缀-日期-名称」结构（推荐）
gws new login-crash --prefix hotfix --title "登录闪退修复"
#   → hotfix-20260821-login-crash

# 方式二：完全自定义分支名
gws new login-crash --branch hotfix/APP-1234-login-crash
#   → hotfix/APP-1234-login-crash
```

`--prefix` 仅接受英文字母（如 `feature`、`hotfix`、`release`）；同时指定时 `--branch` 优先。

**两种方式的差异**在于后续 `gws rename` 的行为：

| 分支形态 | `gws rename old new` |
|---|---|
| 符合 `<前缀>-YYYYMMDD-<名称>` | 自动保留前缀与日期，仅替换名称 |
| 完全自定义 | 无法推导，需显式 `--branch <新分支名>` |

`gws get` 反推工作区名时同样兼容任意前缀：`hotfix-20260821-login-crash` → 工作区 `login-crash`。

### 7.10 创建前的预检与原子性

`gws new` 采用**先全量预检、再统一创建**的两阶段流程，避免出现「部分仓库建成、部分失败」的残缺工作区。预检覆盖三类情况，任一不通过即整体中止且不产生任何副作用：

| 预检项 | 处理 |
|---|---|
| 分支已存在于**远程** | 中止，并提示改用 `gws get <分支>` 拉取该需求 |
| 分支已存在于**本地** | 中止，并给出清理命令（通常是此前删目录未清理干净） |
| 仓库不在 `repos/` 中 | 中止，并提示 `gws repo add` |

远程检查尤为必要：若他人已创建同名分支，本地 `git worktree add -b` 仍会成功，却建出与远程同名而内容无关的分支，直到 `gws push` 才暴露分叉。

预检通过后若仍有模块创建失败（如基线缺失），**已创建的模块会被整体回滚**——删除 worktree、分支、工作区目录与文档目录，不留残留。

---

## 8. 文档管理（Confluence ↔ 本地）

### 8.1 文档存放位置

**所有文档写入当前工作区的 `docs/` 目录**，经软链自动落位到 `<hub>/docs/<日期>-<需求名>/`，不应写入模块目录或其他位置。

该约定同样适用于 AI 辅助生成文档的场景：只需指明写入工作区的 `docs/`，无需关心实际路径。

### 8.2 纳管与上传

- `gws doc commit [说明]` — 将 `docs/` 下的新增与修改纳入文档 git 仓库
- `gws doc push [文件]` — 先纳管，再上传至 Confluence，成功后回写 frontmatter：

```yaml
---
title: 技术方案
workspace: checkout-revamp
created: 2026-08-18
confluence:
  page_id: "123456"
  url: https://confluence.example.com/pages/viewpage.action?pageId=123456
  synced_at: 2026-08-18T12:21:18+08:00
  synced_sha256: <sha256>
---
```

已记录 `page_id` 的文件再次上传时无需重复指定；**正文哈希未变化则跳过上传**，避免产生无意义的版本记录。

> **文档上传属可选集成。** `gws doc push` 调用外部上传脚本，路径通过环境变量配置：
>
> ```bash
> export GWS_DOC_UPLOADER=/path/to/upload.py    # 接收 markdown 文件路径作为参数
> ```
>
> 未配置时 `gws doc push` 会给出提示；仅需本地版本管理时使用 `gws doc commit` 即可。

### 8.3 归档

执行 `gws rm` 删除需求时，其文档目录自动移入 `docs/_archive/`，**不做物理删除**。

---

## 9. 注意事项与 FAQ

| 现象 / 疑问 | 说明 |
|---|---|
| 如何拉取他人在本需求分支上的提交 | `gws pull`（相当于对所有模块执行 git pull）。协作顺序见 §7.6 |
| `gws push` / `gws merge` 提示本地落后于远程 | 他人已推送新提交。执行 `gws pull` 同步后再操作，避免用旧代码覆盖其成果 |
| `gws merge` 提示需要先 `gws push` | 合并仅从 `origin/<feature>` 拉取，未推送则无法合入。此为有意设计，避免合入与远程不一致的代码 |
| 在 envs 中的改动丢失 | envs 是远程镜像，`gws sync` 会 `reset --hard`。**已合并但未推送的成果不会被 merge 覆盖**（见 §6.1），但业务改动必须在 feature 分支进行 |
| 合并到一半需要放弃 | `gws merge <环境> --force` 丢弃 envs 本地未推送的成果，从远程重新合并 |
| 执行 `gws merge` 后未推送 | 默认行为即只合并到本地，供 review。确认后执行 `gws merge <环境> --push` 推送 |
| 能否用 IDE 完成合并提交 | 可以。在 IDE 中解决冲突（提交与否均可），再执行一次 `gws merge <环境>` 收尾 |
| 主干名是 `main` 还是 `master` | 自动逐仓库探测：`origin/main` → `origin/master` → 仍无则查远程 HEAD 默认分支（如 `develop`），无需配置 |
| 手动删了 envs/ 或 ws/ 下的目录 | 僵尸 worktree 元数据由各命令自动执行 `worktree prune` 清理；`gws sync` 会补建缺失的 env 模块，ws 模块清理用无参数 `gws drop` |
| 名称里带了空格 / 斜杠 / `..` | `gws new`、`gws rename`、`gws get --name` 会拒绝含空格、斜杠、`..`、以 `.` 开头的名称，避免建出无法被分支命名与目录系统正常处理的路径 |
| 构建产物（target/、node_modules/） | 每个 worktree 独立存在，天然隔离（同一仓库的需求/环境各有各的产物）。`gws sync` 对 envs 的部分产物会随 `reset --hard` 保留，不主动清理 |
| `gws new` 提示工作区已存在 | 名称重复。更换名称，或先 `gws rm` 删除，亦可用 `gws rename` 改名 |
| IDE 推送被拒绝 / 推向主干 | feature 分支上游被错误绑定至主干。`gws new` 已自动解除，历史工作区可执行 `git branch --unset-upstream` 修正。**推荐使用 `gws push`**，其显式指定分支，不受上游配置影响 |
| 模块名输入错误 | `gws add` 会提示相近的仓库名，`gws drop` 会列出当前工作区的模块 |
| 手动删除了模块目录 | 目录删除本身无妨，但需执行一次无参数的 `gws drop` 清理残留的 worktree 元数据 |

---

## 10. 设计说明

以下是几处容易引发疑问的实现取舍，供二次开发或排障时参考。

### 10.1 为什么合并只从 `origin/<feature>` 拉

`gws merge` 合并的是远程分支而非本地分支。代价是「忘记 push 就合不进去」，收益是**进入环境分支的代码必然与远程可见的代码一致**，杜绝「本地合进去了但同事拉不到」的状态分歧。

### 10.2 为什么 envs 允许在其中解决冲突

`envs/<环境>` 定位为远程分支的本地镜像，`gws sync` 会无条件 `reset --hard`。但冲突解决本身属于合并动作的一部分，必须在合并发生的位置完成，因此这是唯一允许在 envs 内产生提交的场景。

为避免这些提交被后续操作误删，`gws merge` 在重置前会检测未推送的合并成果并停下等待确认（详见 §6 安全保障）。**业务代码的修改仍必须回到 feature 分支进行。**

### 10.3 为什么 feature 分支要解绑上游

`git worktree add -b <branch> <path> origin/main` 会触发 Git 的 DWIM 行为，自动将新分支的上游设为 `origin/main`。此时在 IDE 中点击 Push 会尝试推送到主干并被拒绝。`gws new` / `gws add` 在创建后立即执行 `branch --unset-upstream` 消除这一隐患。

例外是 `gws get`：协作场景下拉取的分支已存在于远程，上游会主动绑定到 `origin/<feature>`，使 IDE 的 Pull / Push 可以直接工作。

### 10.4 基线降级策略

`--from ws:<上游需求>` 指定的基线分支未必存在于所有模块中。典型情况是阶段 2 引入了阶段 1 未涉及的新模块。此时 `gws new` / `gws add` / `gws sync-main` 会自动将该模块降级到主干作为基线，并在输出中标注，而非直接失败。

---

## 11. 版本与分发

### 11.1 使用者：获取与升级

`gws` 是单文件脚本，三种方式都可以拿到：

- 从 GitHub 安装（见 §2）
- 一键脚本 `curl -fsSL .../install.sh | sh`
- **直接拷贝他人的 `gws` 文件**到 `~/.local/bin/` 并 `chmod +x`

无论用哪种方式，之后都可以用 `gws update` 升级——更新源写在脚本内，指向公开仓库，不依赖获取渠道。

```bash
gws version    # 查看当前版本
gws update     # 升级到最新版（内容无变化则跳过）
```

主源 `raw.githubusercontent.com` 若无法连接，会自动尝试 jsDelivr 与代理镜像；也可自行指定源：

```bash
export GWS_UPDATE_URL=https://your-mirror/gws
```

### 11.2 维护者：发布新版本

发布是维护者专属操作（`gws release`），需要 `gh` 已登录且对仓库有写权限。非维护者执行会被直接拒绝，命令也不出现在其 `gws --help` 中（本机 `~/.gws-maintainer` 标记文件控制显示）。

```bash
cd <hub 目录>                      # 在 hub 内执行，README 会一并同步发布
gws release --note "修复了 xxx"     # 默认 patch 位 +1
```

命令内部依次执行：语法检查 → 计算新版本号 → clone 仓库 → 更新脚本与 README → **列出改动待确认** → commit / push / 打 tag → 回写本机版本号。任一步失败即中止，不会出现「本机版本号已提升但远程未发布」的错位。

| 命令 | 版本变化 | 适用 |
|---|---|---|
| `gws release` | 0.2.0 → 0.2.1 | 缺陷修复、小幅调整 |
| `gws release --minor` | 0.2.0 → 0.3.0 | 新增命令或参数 |
| `gws release --major` | 0.2.0 → 1.0.0 | 不兼容变更 |
| `gws release --version 1.0.0` | 指定版本 | 手工定版 |

`--yes` 可跳过确认步骤，供脚本或 CI 使用。