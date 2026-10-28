//! doc-hub 集成（设计方案 §4，需求 #2 / #5 / #17 / #20）。
//!
//! # 模型：文档与代码同构
//!
//! ```text
//! 建需求  → doc-hub 建同名分支
//! 开发中  → 文档写进 ws/<分支>/docs/workspaces/<族>/<分支>/，gws doc sync 推该分支
//! 上线前  → 与代码 MR 一起提文档 MR（分支 → main）
//! 合并    → 人工审核后合 main，成为历史
//! ```
//!
//! **没有 main 直推权限**，所有文档变更走 MR。
//!
//! # 目录契约
//!
//! ```text
//! <hub>/doc-hub/                             ← 主 clone，读历史用（切 main）
//! └── workspaces/
//!     └── <需求分类中文名>/                      ← 族目录：中文名，给人找
//!         └── <完整分支名>/                    ← 期次目录：分支名，绝对唯一
//!             ├── meta.json
//!             └── 需求文档.md / 技术方案.md / …
//!
//! ws/<分支>/docs/                            ← 该需求分支的 worktree
//! └── workspaces/<族>/<分支>/                 ← 实际写文档的位置
//! ```
//!
//! # 三条设计原则
//!
//! 1. **族目录用中文名** → V1 / V2 / bugfix 归到一处，方便找
//! 2. **期次目录用完整分支名** → 绝对唯一，V1/V2 不会撞车
//! 3. **元数据下沉到每期的 `meta.json`，不设中心索引** → 每个需求分支只写
//!    `workspaces/<族>/<自己分支名>/` 整棵子树，**多人多需求并行、MR 合到 main
//!    时零冲突**。中心索引文件会被所有分支同时修改，是必然的冲突热点。

pub mod meta;

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::git::{self, Git};
use crate::hub::Hub;
use crate::workspace::Workspace;
use crate::{Error, Result};

pub use meta::{
    doc_type, template, DocType, PeriodMeta, CLAUDE_FILE, DOC_TYPES, META_FILE, SCHEMA,
};

/// 文档库地址的默认值。
///
/// **开源版没有默认值**：每个人的文档库都放在自己那儿，地址不可能预设。
/// 建 hub 时必填（见 `crate::hub::config::DocHubConfig`）；空 = 没配，
/// 用到文档库的操作会报一条可操作的错，而不是拿一个想当然的地址去 clone。
pub const DEFAULT_URL: &str = "";

/// 族目录名（`workspaces/`）与期次目录名。
const WORKSPACES_DIR: &str = "workspaces";

/// 从中文备注里认版本号：`示例需求v2` → `V2`，`增长二期` → `V2`。
///
/// 认不出来返回 `None`（交给顺序编号）。**只在明确写了才认** ——
/// 猜错比不猜更糟：用户会以为系统知道，从而不去纠正。
fn version_from_title(title: &str) -> Option<String> {
    let lower = title.to_ascii_lowercase();
    if let Some(i) = lower.rfind('v') {
        let digits: String = lower[i + 1..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !digits.is_empty() {
            return Some(format!("V{digits}"));
        }
    }
    const CN: [(char, u32); 9] = [
        ('一', 1),
        ('二', 2),
        ('三', 3),
        ('四', 4),
        ('五', 5),
        ('六', 6),
        ('七', 7),
        ('八', 8),
        ('九', 9),
    ];
    for (ch, n) in CN {
        if title.contains(&format!("{ch}期")) {
            return Some(format!("V{n}"));
        }
    }
    None
}

// ══ 目录布局迁移（实测反馈 #9）════════════════════════════════════════
//
// 老布局 → 新布局：
//   <hub>/doc-hub/          →  <hub>/repos/doc-hub/
//   ws/<分支>/docs/          →  ws/<分支>/doc-hub/
//
// 两个目录都是 **git worktree 体系的一部分**，直接 `mv` 会把链接断开
// （worktree 里的 `.git` 是个文件，内容是主 clone 里 gitdir 的**绝对路径**；
// 主 clone 里的 `.git/worktrees/<n>/gitdir` 又反过来记着 worktree 的绝对路径）。
// 所以：
//   1. worktree 用 `git worktree move`（它自己会维护链接）
//   2. 主 clone 用普通 `mv`
//   3. 最后 `git worktree repair <每个 worktree>` 把两个方向都修正回来
//
// 这三步在临时仓库上实测验证过（两边都移动后 repair 仍能修好，
// `worktree list` / `rev-parse` / `status` 全部正常）。
//
// **失败要能回退**：迁移到一半失败而留下断链的 worktree，比不迁移糟糕得多。

/// 迁移结果（供界面/CLI 告知用户「我动了什么」）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MigrationOutcome {
    /// 实际发生的移动：`(旧相对路径, 新相对路径)`。
    pub moved: Vec<(String, String)>,
    /// 迁移后校验失败的项目（非空 = 已回退，没有留下半迁移状态）。
    pub failed: Vec<String>,
    /// 文档 worktree 的修复说明（空 = 没修什么）。
    #[serde(default)]
    pub doc_repairs: Vec<String>,
    /// 从需求元数据里**摘掉的残留模块登记**：`(分支, 模块名)`。
    ///
    /// 旧布局把文档当普通模块记在 `.workspace.json` 的 `modules` 里（名字是
    /// `docs`）。新布局下它不再是模块条目 —— 不清掉的话，界面会一直显示
    /// 「有 1 个模块目录缺失」，而「清理残留」扫的是「没有元数据的目录」，
    /// 根本不是同一回事，点了也没用（实测反馈 #2）。
    pub pruned_modules: Vec<(String, String)>,
}

impl MigrationOutcome {
    pub fn did_nothing(&self) -> bool {
        self.moved.is_empty()
            && self.failed.is_empty()
            && self.pruned_modules.is_empty()
            && self.doc_repairs.is_empty()
    }
}

/// 把 hub 里的文档库目录**对齐到配置里的名字**（`repos/<仓库名>`）。**幂等**。
///
/// # 为什么需要它
///
/// 早期版本把文档库目录名**写死**成 `doc-hub`：用户填的明明是 `x-doc-hub`，
/// hub 里却建出个 `doc-hub` 目录 —— 名字对不上，界面看着就像「文档没拉下来」
/// （实测反馈）。现在目录名从配置里的 URL 推导，这个函数负责把已经建出来的
/// 旧名字改成新名字。
///
/// 改名只动**目录名**、不碰内容：主 clone 直接 `rename`，各需求里的 worktree
/// 目录跟着改名；断掉的链接交给 `repair_broken_worktrees` 修 —— 它在
/// `Hub::open` 里紧跟着迁移调用。
pub fn migrate_layout(hub: &Hub) -> MigrationOutcome {
    // 这两件事与目录名无关，每次打开都跑（幂等、零开销）
    let mut out = MigrationOutcome {
        pruned_modules: prune_legacy_module_entries(hub),
        doc_repairs: repair_doc_worktrees_behind_remote(hub),
        ..Default::default()
    };

    align_docdir_name(hub, &mut out);
    out
}

/// 把文档库目录改成配置里那个名字。已经对齐时零开销。
fn align_docdir_name(hub: &Hub, out: &mut MigrationOutcome) {
    if !hub.repos_dir().is_dir() {
        return; // 还没拉过任何仓库，没什么可对齐的
    }
    // ⚠️ **hub 还没绑文档库地址时什么都不做。**
    //
    // 那时候 `dochub_name()` 返回的是兜底名 `doc-hub`，而它**同时也在
    // `LEGACY_DOCHUB_NAMES` 里**（旧名列表必须有它，否则老 hub 的残留改不掉）。
    // 两者相遇的后果：某个需求/环境里**恰好有个普通模块叫 `x-doc-hub`** 时，
    // 它会被当成"文档库的旧名"改名 —— 而 `repair_worktrees_after_rename` 只修
    // 文档库那一个路径，这个被改名的普通 worktree 链接就没人修了。
    // 没绑定地址 = 没有"该对齐的名字"，不动最安全。
    if hub.config().dochub.url.trim().is_empty() {
        return;
    }
    let new_name = hub.dochub_name();

    // ① 主 clone：`repos/<旧名>` → `repos/<新名>`（已对齐就跳过）
    let target = hub.dochub_new_dir();
    if !target.join(".git").is_dir() {
        if let Some(old) = crate::hub::LEGACY_DOCHUB_NAMES
            .iter()
            .map(|n| hub.repos_dir().join(n))
            .find(|p| p.join(".git").is_dir())
        {
            let old_name = old
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if !old_name.is_empty() && old_name != new_name {
                // git 仓库不记自己的绝对路径，直接 rename 是安全的
                if let Err(e) = std::fs::rename(&old, &target) {
                    out.failed
                        .push(format!("文档库目录改名失败（{old_name} → {new_name}）: {e}"));
                    return;
                }
                out.moved.push((old_name, new_name.clone()));
            }
        }
    }

    // ② 其余位置（各需求、各环境）里的旧名一律改成新名。
    //
    // **这一段每次打开都跑**，判据不能只看「主 clone 是否刚改过」——
    // 主 clone 可能早就是新名了，而 envs / ws 里还留着旧名。那样会出现
    // 「主干叫新名、环境叫旧名」，比不改还让人困惑（实测反馈）。
    let mut places: Vec<(String, PathBuf)> = Vec::new();
    for ws in crate::workspace::list(hub).unwrap_or_default() {
        places.push((ws.branch().to_string(), ws.dir().to_path_buf()));
    }
    for env in &hub.config().envs {
        places.push((format!("envs/{}", env.id), hub.env_dir(&env.id)));
    }

    for (label, dir) in places {
        for old_name in crate::hub::LEGACY_DOCHUB_NAMES {
            if old_name == new_name {
                continue; // 新旧同名就没有「残留」可言
            }
            let o = dir.join(old_name);
            let n = dir.join(&new_name);
            if o.exists() && !n.exists() {
                if let Err(e) = std::fs::rename(&o, &n) {
                    out.failed.push(format!("{label}/{old_name} 改名失败: {e}"));
                }
            }
        }
    }

    // ③ 改名会**断开 worktree 链接**（链接两处写的都是绝对路径），必须当场修回来。
    //
    // 不能指望 `repair_broken_worktrees`：它靠「从旧路径里解析出仓库名」去找主
    // clone，而这个名字**刚好被我们改掉了**，它必然找不到（实测：envs 里的检出
    // 变成 `not a git repository`）。所以这里直接用 git 自己的 `worktree repair`，
    // 把各 worktree 的**新路径**传进去 —— 两个方向的链接它都会修。
    repair_worktrees_after_rename(hub);
}

/// 改名之后修 worktree 链接。**没有 worktree 时零开销**（只读几个小文件）。
fn repair_worktrees_after_rename(hub: &Hub) {
    let main = hub.dochub_new_dir();
    if !main.join(".git").is_dir() {
        return;
    }
    let name = hub.dochub_name();

    let mut paths: Vec<String> = Vec::new();
    for ws in crate::workspace::list(hub).unwrap_or_default() {
        let p = ws.dir().join(&name);
        if p.join(".git").is_file() {
            paths.push(p.to_string_lossy().to_string());
        }
    }
    for env in &hub.config().envs {
        let p = hub.env_dir(&env.id).join(&name);
        if p.join(".git").is_file() {
            paths.push(p.to_string_lossy().to_string());
        }
    }
    if paths.is_empty() {
        return;
    }

    let mut args: Vec<&str> = vec!["worktree", "repair"];
    args.extend(paths.iter().map(String::as_str));
    // 失败不阻断打开：修不了也只影响那几个 worktree，报出来即可
    if let Err(e) = Git::new(&main).status(&args) {
        eprintln!("⚠ 文档库 worktree 链接修复失败：{e}");
    }
}


/// 把旧布局遗留在 `.workspace.json` 的 `modules` 里的文档条目摘掉。
///
/// 幂等，每次打开 hub 都会跑（很便宜）。**不动别的模块** —— 目录真的缺失的
/// 那些要留给人自己决定（可能是手工删了想找回）。
fn prune_legacy_module_entries(hub: &Hub) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(list) = crate::workspace::list(hub) else {
        return out;
    };
    for mut ws in list {
        let legacy: Vec<String> = ws
            .meta()
            .modules
            .iter()
            .filter(|m| *m == crate::hub::LEGACY_DOCS_DIR || hub.is_dochub(m))
            .cloned()
            .collect();
        if legacy.is_empty() {
            continue;
        }
        let branch = ws.branch().to_string();
        {
            let meta = ws.meta_mut();
            meta.modules.retain(|m| !legacy.contains(m));
        }
        if ws.save_meta().is_ok() {
            for m in legacy {
                out.push((branch.clone(), m));
            }
        }
    }
    out
}

/// 一个 worktree 目录里的 git 还能不能用。
///
/// 迁移最怕「目录挪过去了、git 认不出」——那种状态比不迁移糟糕得多，
/// 所以每一步都拿真实 git 命令验回来，而不是只看目录在不在。
/// 修「文档 worktree 从主干误建」留下的脏数据（实测反馈）。
///
/// # 坏掉的形态
///
/// 其他人（或同一台机器）在文档已经推上去**之后**才拉这个需求时：
///
/// ```text
/// ws/<分支>/doc-hub  ← 本地分支是从 origin/main 建的（旧代码写死了它）
///                        于是正文一个都没有，attach 又生成了
///                        CLAUDE.md / AGENTS.md / meta.json
///                        界面上 =「什么都没改却提示 3 个改动」
/// origin/<分支>        ← 正文其实在这里
/// ```
///
/// # 判据（全部问 git，不猜）
///
/// 1. 本地分支**没有自己的提交**（`origin/main..HEAD` 为空）—— 有提交就不是"误建"，别动
/// 2. HEAD 是 `origin/<分支>` 的**祖先**（只是落后）—— 分叉的更不能动
/// 3. 确实落后（`HEAD != origin/<分支>`）
///
/// 三条都满足 → 快进到 `origin/<分支>`。快进前先把那几个**会挡路的工具生成文件**
/// 删掉：它们在目标提交里也存在，git 不肯用仓库里的版本覆盖未跟踪的同名文件。
///
/// 返回一句给用户看的说明（空 = 没什么要修的）。
fn repair_doc_worktrees_behind_remote(hub: &Hub) -> Vec<String> {
    let mut notes = Vec::new();
    let Ok(workspaces) = crate::workspace::list(hub) else {
        return notes;
    };
    for ws in workspaces {
        let docs = ws.dir().join(hub.dochub_name());
        if !docs.join(".git").exists() {
            continue;
        }
        let git = Git::new(&docs);
        let branch = git.current_branch().unwrap_or_default();
        if branch.is_empty() {
            continue;
        }
        let remote = format!("origin/{branch}");
        if !git.ref_exists(&remote) {
            continue; // 远程还没有这条分支（这一期还没推过）→ 没什么可拉
        }
        // **不 fetch**：这里是"打开 hub 时顺带跑"，一次网络往返 × 每个需求就是
        // 肉眼可见的卡顿（实测反馈）。用本地已有的 `origin/*` 引用判断就够了 ——
        // 真坏掉的那种是"刚拉下来的需求"，那时引用本来就是新的。
        //
        // ① 本地分支不能有自己的提交
        if git.count_range("origin/main..HEAD").unwrap_or(1) != 0 {
            continue;
        }
        // ② 只是落后，不是分叉；③ 确实落后
        if !git.is_ancestor("HEAD", &remote) || git.sha("HEAD").ok() == git.sha(&remote).ok() {
            continue;
        }
        // ④ 清掉会挡路的工具生成文件（只在"本地这份是误建的"前提下才走到这）
        let period_rel = ws
            .meta()
            .doc
            .as_ref()
            .map(|d| format!("{WORKSPACES_DIR}/{}/{}", d.family, branch));
        let mut removed = Vec::new();
        if let Some(rel) = &period_rel {
            for name in ["CLAUDE.md", "AGENTS.md", META_FILE] {
                let p = docs.join(rel).join(name);
                if p.exists() && std::fs::remove_file(&p).is_ok() {
                    removed.push(name);
                }
            }
        }
        match git.reset_hard(&remote) {
            Ok(()) => notes.push(format!(
                "文档 {branch}：本地是从主干误建的（正文没拉下来），已拉取到最新（清掉了 {} 个本地生成的文件）",
                removed.len()
            )),
            Err(e) => notes.push(format!("文档 {branch} 想拉取正文但失败: {e}")),
        }
    }
    notes
}

/// 把文档库**主 clone** 的工作树快进到 `origin/main`。返回一句给用户看的说明
/// （`None` = 没什么可做的，什么都不动）。
///
/// # 为什么必须做
///
/// 主 clone 是**看已归档文档**的地方：已并入主干的期次只存在于 `main` 上，
/// 而界面是**按工作树路径**去读文件的。只 `fetch` 的话，ref 更新了、git 对象
/// 有了，列表里于是能看见那一期；但工作树还停在旧提交上，点开就是空的 ——
/// 实测反馈「点了同步最新文档，同步下来了但我这里看不到」。
///
/// # 只在能安全快进时动它
///
/// 任何一条不满足就**原样不动**并说明：
///
/// - 当前不在 `main` 上（有人手工切过）→ 切回去可能打断他正在做的事
/// - 工作树有改动 → 快进会失败或冲突，那是人的东西，不该被「同步」搅了
/// - 本地 `main` 已经有自己的提交（跟远程分叉）→ 只能人来决定怎么办
///
/// 这不是「什么都不做」的保守：**快进是真的会动文件**，而这个 clone 的
/// 定位只是「一份给人看的镜像」，没有一条理由值得为了刷它去动人本地的东西。
fn fast_forward_main(git: &Git) -> Result<Option<String>> {
    if git.current_branch().unwrap_or_default() != "main" {
        return Ok(None);
    }
    if !git.ref_exists("origin/main") {
        return Ok(None);
    }
    // 读不到状态时按「脏」处理 —— 这正是 `is_dirty` 返回 Result 的理由
    if git.is_dirty().unwrap_or(true) {
        return Ok(None);
    }
    let (ahead, behind) = git.ahead_behind("origin/main")?;
    if behind == 0 || ahead != 0 {
        return Ok(None);
    }
    git.merge_ff_only("origin/main")?;
    Ok(Some(format!("已更新到最新（+{behind} 个提交）")))
}

/// 一个需求分类：中文名 + 它下面各期次的元数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Family {
    pub name: String,
    /// 按期次日期排序。
    pub periods: Vec<PeriodWithState>,
}

/// 期次 + 它在文档库里的状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PeriodWithState {
    #[serde(flatten)]
    pub meta: PeriodMeta,
    /// 该期次所在分支**是否已并入 main**。
    ///
    /// `true` = 已归档的历史文档（只读，AI 不要改）；
    /// `false` = 正在开发、还没合并的文档。
    pub merged: bool,
    /// 除了工具生成的脚手架（`meta.json` / `CLAUDE.md` / `AGENTS.md`）
    /// 之外，**里面有没有真正的文档**。
    ///
    /// 只挂了文档、一个字没写的需求，不该在文档页里占一行。
    pub has_content: bool,
    /// 文档页**打不打得开**：主 clone 或本需求工作区里有没有这一期目录。
    ///
    /// 只在远程分支 / 环境 worktree 里存在的期次，文档页读不到（点开是空的），
    /// 列出来只会让人以为文档丢了（实测反馈：`巨量单元状态同步`）。
    pub available: bool,
}

impl PeriodWithState {
    pub fn branch(&self) -> &str {
        &self.meta.branch
    }
}

/// 期次目录里有没有**真正的文档**（脚手架与系统垃圾不算）。
pub fn period_has_content(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if is_scaffold_or_junk(&name) {
            continue;
        }
        if e.path().is_dir() {
            if period_has_content(&e.path()) {
                return true;
            }
        } else {
            return true;
        }
    }
    false
}

fn is_scaffold_or_junk(name: &str) -> bool {
    matches!(
        name,
        "meta.json" | "CLAUDE.md" | "AGENTS.md" | ".DS_Store" | "Thumbs.db" | "desktop.ini"
    ) || name.starts_with("._")
}

/// 某个分支上的期次目录里有没有正文（不看工作树）。
fn rev_period_has_content(git: &Git, rev: &str, family: &str, branch: &str) -> bool {
    let prefix = format!("{WORKSPACES_DIR}/{family}/{branch}/");
    let Ok(out) = git.out(&[
        "-c",
        "core.quotePath=false",
        "ls-tree",
        "-r",
        "--name-only",
        rev,
        "--",
        &prefix,
    ]) else {
        // 读不出来时宁可显示，也不要误藏用户的文档
        return true;
    };
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .any(|line| {
            let name = line.rsplit('/').next().unwrap_or(line);
            !is_scaffold_or_junk(name)
        })
}

/// 在这个文档 worktree 里找**属于本分支的那一期** —— **不限于哪个分类**。
///
/// # 为什么要"不限于分类"
///
/// 期次目录是 `workspaces/<分类>/<分支>/`，而**分类可能是猜的**（指定 /
/// 已登记 / 按年月）。猜错的时候，按猜出来的路径
/// 一看"没有这一期"，attach 就会**重新生成一份** —— 而远程那份正文其实一直在，
/// 只是躺在另一个分类目录下。表现出来就是"导入的需求文档没下来 + 平白多出
/// 3 个未提交改动"（实测反馈）。
///
/// 所以以**目录里实际有的**为准：扫 `workspaces/*/<分支>/meta.json`，
/// 谁有就用谁 —— 远程那份 `meta.json` 里写的分类才是准的。
///
/// 多个分类下都有同名分支（不该发生）时按分类名排序取第一个：**结果稳定**，
/// 不随 `read_dir` 的顺序变。
fn find_period_for_branch(docs: &Path, branch: &str) -> Option<PathBuf> {
    let wsdir = docs.join(WORKSPACES_DIR);
    let mut hits: Vec<PathBuf> = std::fs::read_dir(&wsdir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.path().join(branch).join(META_FILE))
        .filter(|p| p.is_file())
        .collect();
    hits.sort();
    hits.into_iter().next()
}

/// doc-hub 的操作入口。
pub struct DocHub {
    root: PathBuf,
    url: String,
    /// 文档库在 hub 里的目录名（从配置的 URL 推导，见 `Hub::dochub_name`）。
    /// 存下来是因为不少方法只有 `&self`、拿不到 hub。
    name: String,
    /// 所属 hub 的根目录。
    ///
    /// 存下来是为了推送文档时能取到**这个 hub** 的 GitLab token ——
    /// 凭据跟着 hub 走，而 `DocHub` 自己只有文档库的路径和地址。
    hub_root: PathBuf,
}

/// [`DocHub::attach`] 的结果。
#[derive(Debug, Clone, Serialize)]
pub struct AttachOutcome {
    pub family: String,
    pub label: String,
    /// 期次目录（相对 doc-hub 仓库根）：`workspaces/<族>/<分支>/`。
    pub relative_dir: String,
    /// 本次是否新建（`false` = 之前已挂过，原样返回）。
    pub created: bool,
    /// **真的挂上了没有**。
    ///
    /// [`AttachMode::ReuseOnly`] 下、且这条分支上没有任何一期文档时是 `false`
    /// ——那是"这次不挂"，不是失败（见 `AttachMode` 的注释）。
    ///
    /// 不需要 `#[serde(default)]`：这个结构只会被**序列化**给前端
    /// （`Create` 那条路永远给 `true`），不会反序列化回来。
    pub attached: bool,
}

/// 挂载方式。
///
/// 分成两种是因为**同一个动作在两种场景下的正确行为不一样**：
///
/// - [`AttachMode::Create`]（新建需求 / 手动「挂上文档」）：这一期是**本地起头**的，
///   期次目录和说明文件该由我们生成。
/// - [`AttachMode::ReuseOnly`]（导入需求）：那一期要么在其他人推上来的**远程分支**上
///   （worktree 一拉下来就完整），要么**还不存在**。后者绝不该凭空造几个文件出来 ——
///   名字是确定性的（`meta.json` / `CLAUDE.md` / `AGENTS.md`），远程一旦也有了
///   同名分支，本地这几个**未跟踪**文件就会挡住 `git pull`
///   （`untracked working tree files would be overwritten by merge`）。
///   实测反馈：「导入的需求 doc-hub 多出 3 个改动，而且因为我无法拉取代码」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachMode {
    Create,
    ReuseOnly,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DocSyncOutcome {
    pub changed: Vec<String>,
    pub committed: bool,
    pub pushed: bool,
}

impl DocHub {
    /// 打开或克隆 doc-hub。已存在则顺带 fetch 一次（失败不致命）。
    pub fn ensure(hub: &Hub) -> Result<Self> {
        Self::ensure_with(hub, None)
    }

    /// **只打开本地已有的 doc-hub，不联网**。
    ///
    /// 读操作（列需求分类、读某期文档）走这条。原来它们也走 `ensure_with`，
    /// 于是每次切到文档页都跑一次 `git fetch`——界面就卡在那转圈，
    /// 而用户只是要看本地已有的文档（实测反馈）。
    ///
    /// 还没克隆时报错而不是自动克隆：克隆要几十秒，不该藏在一个「顺便」
    /// 的读操作里。界面会提示去点「拉取文档库」。
    pub fn ensure_local(hub: &Hub) -> Result<Self> {
        let root = hub.dochub_dir();
        if !root.join(".git").is_dir() {
            return Err(Error::other(
                "文档库还没拉到本地。点「拉取文档库」先拉一次（首次会比较慢）",
            ));
        }
        Ok(Self::finish(root, hub.config().dochub.url.clone(), hub))
    }

    /// 带进度的版本。克隆文档库可能要几十秒，界面必须能看到在跑。
    pub fn ensure_with(hub: &Hub, cb: crate::progress::Progress<'_>) -> Result<Self> {
        let root = hub.dochub_dir();
        let url = hub.config().dochub.url.clone();
        // **必须带 hub 的 token**：文档库常常是私有仓库 —— 建 hub 时就是用 token
        // 校验的「存在 + 有权限」，这里匿名拉只会得到
        // `could not read Username for 'https://…'`，而文档库地址建完**不可改**，
        // 用户会直接卡死在「拉取文档库」这一步。取不到 token 就按匿名试（公开仓库）。
        let token = crate::gitlab::token::resolve(hub.root()).ok();

        if root.join(".git").is_dir() {
            let mut rep = crate::progress::Reporter::start(cb, 2, "刷新文档库");
            let git = match token.as_deref() {
                Some(t) => Git::new(&root).with_token(t),
                None => Git::new(&root),
            };
            // fetch 失败不阻断：离线也要能读写本地已有的文档
            match git.fetch() {
                Ok(()) => rep.ok("文档库"),
                Err(e) => rep.fail("文档库", e.to_string()),
            }
            // **fetch 完还得把工作树拉上去**：已归档的文档是**按工作树路径**读的
            //（见 `docs_page_dir`），只更新 ref 的话列表里有那一期、点开却是空的
            // —— 实测反馈「点了同步最新文档，同步下来了但我这里看不到」。
            match fast_forward_main(&git) {
                Ok(Some(what)) => rep.ok(format!("已归档文档 {what}")),
                Ok(None) => {}
                Err(e) => rep.fail("已归档文档", e.to_string()),
            }
            rep.finish();
            return Ok(Self::finish(root, url, hub));
        }

        if root.exists() {
            // 目录在但不是 git 仓库（半途失败留下的空目录）→ 清掉重来
            crate::workspace::lifecycle::remove_dir_verified(&root)?;
        }
        if let Some(parent) = root.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent.to_path_buf(), e))?;
        }

        let mut rep = crate::progress::Reporter::start(cb, 1, "克隆文档库（首次会比较慢）");
        // 带 token 失败再退回匿名：token 过期、或该 token 没有这个仓库权限时，
        // 公开仓库不带凭据反而拉得下来 —— 不能因为一个坏 token 把公开仓库也拖下水
        //（与 `repo::add` 同一套策略）。
        let mut cloned = git::clone_with_token(&url, &root, token.as_deref());
        if cloned.is_err() && token.is_some() {
            let _ = std::fs::remove_dir_all(&root);
            cloned = git::clone_with_token(&url, &root, None);
        }
        match cloned {
            Ok(()) => rep.ok("文档库"),
            Err(e) => {
                rep.fail("文档库", e.to_string());
                rep.finish();
                return Err(e);
            }
        }
        rep.finish();

        Ok(Self::finish(root, url, hub))
    }

    /// 组装 `DocHub`，并把 hub 的提交身份写进文档库的 local config。
    ///
    /// 文档库是**唯一**不在 `repo::add` 里 clone 的仓库（它由这里单独拉），
    /// 所以身份要在这里补一次 —— 否则用户在文档目录里提交会用错身份。
    /// 身份没配时 `apply_identity` 会清掉 local config 里的旧值，同样是对的。
    fn finish(root: PathBuf, url: String, hub: &Hub) -> Self {
        let dh = Self {
            root,
            url,
            name: hub.dochub_name(),
            hub_root: hub.root().to_path_buf(),
        };
        if let Err(e) = crate::git::apply_identity(&dh.root, &hub.config().git) {
            // 失败只提示：文档库本身是好的，不该因此让整个操作失败
            eprintln!("⚠ 写入提交身份失败（文档库）：{e}");
        }
        dh
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    // ── 已归档文档的读路径 ──────────────────────────────────────────────
    //
    // 已并入主干的期次只存在于 `main` 上，界面读它们时走的是**主 clone 的
    // 工作树路径**（`<hub>/repos/doc-hub/workspaces/<族>/<分支>/`）。
    // 所以主 clone 的工作树必须是新的 —— 只 fetch 不更新工作树的话，
    // 列表里看得到那一期（那条走 git 对象），点开却是空的。
    //
    // 实测反馈：「点了同步最新文档，同步下来了但我这里看不到」。
    // 修法见 `fast_forward_main`。

    pub fn url(&self) -> &str {
        &self.url
    }

    /// 列出所有需求分类及其期次（按族名排序，期次按日期排序）。
    ///
    /// # 为什么不能只扫工作树
    ///
    /// 期次文档在**合并进 `main` 之前只存在于需求分支上**——而那是开发期间的
    /// 常态。只扫主 clone 的工作树（还停在 `main`）会看不见任何未合并的期次，
    /// 于是「同一需求的 V2 该归到 V1 那族」这条规则直接失效。
    ///
    /// 所以扫三处并按分支名去重：
    /// 1. 工作树（`main` 上已合并的内容）
    /// 2. 所有**本地**分支
    /// 3. 所有**远程**分支（其他人建的、还没合并的期次也能看见）
    pub fn families(&self) -> Result<Vec<Family>> {
        let mut seen: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        let mut all: Vec<(PeriodMeta, bool)> = Vec::new();

        let git = Git::new(&self.root);

        // ① 各 worktree 的**工作目录**
        //
        // 必须扫这里，而不是只扫分支：`attach` 建出的期次目录在 `doc sync`
        // 之前是未提交状态，`ls-tree <分支>` 根本看不到。而「刚 attach 完
        // 就要判断该归到哪个族」正是最常见的路径。
        for wt in self.worktree_paths(&git) {
            for (m, has_content) in scan_worktree(&wt.join(WORKSPACES_DIR)) {
                merge_period(&mut all, &mut seen, m, has_content);
            }
        }

        // ② + ③ 各分支上**已提交**的内容（其他人建的、已推上来的期次）
        for rev in self.all_revs(&git) {
            for path in meta_paths_in(&git, &rev).unwrap_or_default() {
                let Ok(text) = git.out(&["show", &format!("{rev}:{path}")]) else {
                    continue;
                };
                if let Ok(m) = serde_json::from_str::<PeriodMeta>(&text) {
                    let has_content = rev_period_has_content(&git, &rev, &m.family, &m.branch);
                    merge_period(&mut all, &mut seen, m, has_content);
                }
            }
        }

        // 标出哪些期次已经并入主干 —— 界面据此区分「已归档的历史文档」
        // 与「还在开发、没合并的文档」
        let archived = self.archived_in_main(&git);

        let mut by_family: std::collections::BTreeMap<String, Vec<PeriodWithState>> =
            std::collections::BTreeMap::new();
        // 界面能打开的两份目录：主 clone（已归档）与需求工作区（开发中）。
        // 环境 worktree 不算 —— 文档页不读那里，列出来点开就是空的（实测反馈）。
        let hub_root = self.root.parent().and_then(|p| p.parent());
        for (m, has_content) in all {
            // 判据是**内容在主干的树里**，键是「族/分支」
            let merged = archived.contains(&format!("{}/{}", m.family, m.branch));
            let at = |base: &std::path::Path| {
                base.join(WORKSPACES_DIR)
                    .join(&m.family)
                    .join(&m.branch)
                    .is_dir()
            };
            let available = at(&self.root)
                || hub_root
                    .map(|h| {
                        at(&h
                            .join(crate::hub::WS_DIR)
                            .join(&m.branch)
                            .join(&self.name))
                    })
                    .unwrap_or(false);
            by_family
                .entry(m.family.clone())
                .or_default()
                .push(PeriodWithState {
                meta: m,
                merged,
                    has_content,
                    available,
            });
        }

        let mut out: Vec<Family> = by_family
            .into_iter()
            .map(|(name, mut periods)| {
                periods.sort_by(|a, b| {
                    a.meta
                        .date
                        .cmp(&b.meta.date)
                        .then(a.meta.branch.cmp(&b.meta.branch))
                });
                Family { name, periods }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

/// 主干里**实际存在**的期次目录，键为 `族/分支`。
    ///
    /// # 为什么不用 `git branch --merged`
    ///
    /// 这里换过一次判据，把原因记下来。
    ///
    /// 原来用 `git branch --merged <主干>` ——即「该分支的所有提交是否都在主干里」。
    /// 但这个问法对**自己还没有任何提交**的分支平凡成立：空集是任何集合的子集。
    /// 实测踩到：一个刚建出来、文档还没提交的期次分支，恰好与主干同一个提交，
    /// 于是被判成「已归档」——用户看到的是一份**正在写的**文档被列进了
    /// 「已并入主干、只读、不要改」那一栏。
    ///
    /// 现在改问一个更贴近语义的问题：**这一期的内容在不在主干的树里**。
    /// 「已归档」要表达的就是「文档已经进了主干」，而不是「这个分支没有额外提交」。
    ///
    /// 用 `ls-tree` 读**主干这个 ref 的树**，而不是看工作树——工作树停在哪个
    /// 分支、有没有未提交内容都会影响结果，那种判据不可靠（attach 出来的期次
    /// 目录在 `doc sync` 之前根本没提交，`ls-tree` 也看不到，属于正常的
    /// 「还没归档」）。
    fn archived_in_main(&self, git: &Git) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        // **两个都看**：`origin/main` 是权威（别人在 GitLab 上合的结果），
        // 本地 `main` 是「我本地合了但还没推」的情况。只看其中一个都会漏。
        for base in ["origin/main", "main"] {
            if !git.ref_exists(base) {
                continue;
            }
            // `-c core.quotePath=false` 不能省：git 默认把非 ASCII 路径转义成
            // 八进制（`"workspaces/\347\247\201..."`），族目录名是中文名，
            // 转义后切出来的「族」永远匹配不上，判定会静默全错。
            // （同一个坑 `sync` 那边也踩过并记了注释。）
            let Ok(s) = git.out(&[
                "-c",
                "core.quotePath=false",
                "ls-tree",
                "-r",
                "--name-only",
                base,
                "--",
                &format!("{WORKSPACES_DIR}/"),
            ]) else {
                continue;
            };
            for line in s.lines() {
                // workspaces/<族>/<分支>/<文件…>
                let mut it = line.trim().splitn(4, '/');
                let _root = it.next();
                let (Some(family), Some(branch)) = (it.next(), it.next()) else {
                    continue;
                };
                if !family.is_empty() && !branch.is_empty() {
                    out.insert(format!("{family}/{branch}"));
                }
            }
        }
        out
    }

    /// doc-hub 的所有 worktree 路径（含各需求分支的 `docs` 目录）。
    ///
    /// 用 git 自己的 worktree 注册表，而不是去 `hub/ws/*/docs` 找——
    /// DocHub 不该假设工作区一定长在 hub 的哪个位置。
    fn worktree_paths(&self, git: &Git) -> Vec<PathBuf> {
        git.out(&["worktree", "list", "--porcelain"])
            .map(|s| {
                s.lines()
                    .filter_map(|l| l.strip_prefix("worktree "))
                    .map(|p| PathBuf::from(p.trim()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// doc-hub 里所有值得扫的分支：本地 + 远程（去掉 `origin/HEAD` 这类符号引用）。
    fn all_revs(&self, git: &Git) -> Vec<String> {
        git.out(&[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads",
            "refs/remotes/origin",
        ])
            .map(|s| {
                s.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty() && !l.ends_with("/HEAD"))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

/// 没填需求分类时，新建的族目录叫**年月**（`2026-10`）。
///
/// 用工作区的**创建日期**而不是"今天"：需求可能建完过几天才挂文档，
/// 按月归组要看的是"这个需求是哪个月的活"。取不到日期才退回"今天"。
pub fn month_family_name(created: &str) -> String {
    let m = created.trim();
    if m.len() >= 7 && m.as_bytes().get(4) == Some(&b'-') {
        return m[..7].to_string();
    }
    let today = crate::date::today(); // YYYY-MM-DD
    today[..7.min(today.len())].to_string()
}

    /// 确定某工作区应该落在哪个族，并说明依据。
    ///
    /// 按优先级：
    /// 1. **显式指定**（`--doc-family` / 新建时的「分类」）——人的判断最准
    /// 2. 工作区元数据里已登记的（已挂过，保持一致）
    /// 3. **按年月**（`2026-10`）——没填分类时唯一的结果
    ///
    /// # 为什么不再自动猜「同一需求的另一期」
    ///
    /// 以前会拿中文名/分支 slug 跟已有族做前缀匹配，想把 V1 / V2 / bugfix
    /// 自动归到一处。实测它**猜错过**：没填分类的需求被一个内容无关的旧族
    /// 吸走（`test` 这种），用户明确要求"没填分类就按年月"。
    /// 现在归组只由人决定：要放在一起，就在新建时填同一个分类。
    pub fn resolve_family(&self, ws: &Workspace, hint: Option<&str>) -> FamilyChoice {
        if let Some(h) = hint.map(str::trim).filter(|h| !h.is_empty()) {
            return FamilyChoice {
                family: h.to_string(),
                reason: "指定",
            };
        }
        if let Some(doc) = &ws.meta().doc {
            if !doc.family.trim().is_empty() {
                return FamilyChoice {
                    family: doc.family.clone(),
                    reason: "已登记",
                };
            }
            }
        FamilyChoice {
            family: Self::month_family_name(&ws.meta().created),
            reason: "按年月",
        }
    }

    /// 该需求分支的文档 worktree 路径：`ws/<分支>/docs/`。
    /// 需求里的文档目录：`ws/<分支>/doc-hub`。
    ///
    /// 跟普通模块**同一个层级、同一个命名方式**（实测反馈 #9）——
    /// 它本来就是 doc-hub 在该需求分支上的 worktree，没有理由特殊。
    pub fn docs_dir(&self, ws: &Workspace) -> PathBuf {
        ws.dir().join(&self.name)
    }

    /// 期次目录：`ws/<分支>/docs/workspaces/<族>/<分支>/`。
    pub fn period_dir(&self, ws: &Workspace) -> PathBuf {
        let family = self.resolve_family(ws, None).family;
        self.docs_dir(ws)
            .join(WORKSPACES_DIR)
            .join(family)
            .join(ws.branch())
    }

    /// 把 doc-hub 挂到工作区上：建分支 worktree + 期次目录 + 元数据。
    ///
    /// 幂等：已挂过就原样返回。
    ///
    /// 等价于 [`DocHub::attach_with`] 的 [`AttachMode::Create`]。
    pub fn attach(&self, ws: &mut Workspace, hint: Option<&str>) -> Result<AttachOutcome> {
        self.attach_with(ws, hint, AttachMode::Create)
    }

    /// 同上，但可以指定[挂载方式](AttachMode)。
    pub fn attach_with(
        &self,
        ws: &mut Workspace,
        hint: Option<&str>,
        mode: AttachMode,
    ) -> Result<AttachOutcome> {
        let branch = ws.branch().to_string();
        let family = self.resolve_family(ws, hint).family;

        let docs = self.docs_dir(ws);

        // ── 0. **先把 worktree 拉到本地**，再看里面有没有那一期 ──────────
        //
        // ⚠️ 这两步的**顺序**是要紧的：原来先判 `meta.json` 在不在、再建 worktree ——
        // 而"导入别人推上来的需求"时，`meta.json` 是**建 worktree 才下来的**。
        // 于是判断必然说"没有"，接着就**生成**了一份新的把远程那份覆盖掉，
        // 还多出 CLAUDE.md / AGENTS.md 两个文件 —— 界面上就成了
        // 「什么都没改却提示 3 个改动」（实测反馈），而且远程的正文也没了。
        self.ensure_worktree(&docs, &branch)?;

        // ── 1. **以目录里实际有的为准**找本分支的那一期 ────────────────────
        //
        // 分类（`family`）可能是猜的：指定 / 已登记 / 按年月。猜错时上面那个
        // 路径就指到别处去了，于是"远程明明有这一期，这里却看不见"，
        // 接着就会**重新生成一份**（实测反馈的那三个脏文件）。
        //
        // 所以先扫 `workspaces/*/<分支>/meta.json`：谁有就用谁的分支与分类。
        // 远程那份 `meta.json` 里写的分类才是准的。
        if let Some(found) = find_period_for_branch(&docs, &branch) {
            if let Ok(existing) = PeriodMeta::load(&found) {
                if ws.meta().doc.is_none() {
                    ws.meta_mut().doc = Some(crate::workspace::DocRef::new(
                        existing.family.clone(),
                        existing.branch.clone(),
                    ));
                    ws.save_meta()?;
                }
                return Ok(AttachOutcome {
                    family: existing.family.clone(),
                    label: existing.label.clone(),
                    relative_dir: existing.relative_dir(),
                    created: false,
                    attached: true,
                });
            }
            // 读不出来（旧格式 / 被人手改坏）接着往下走：
            // Create 模式下会按本地重新生成一份，ReuseOnly 模式下只当没找到。
        }

        let period = docs.join(WORKSPACES_DIR).join(&family).join(&branch);
        let meta_path = period.join(META_FILE);

        // ── 2. 本地解析出来的那个位置正好有 → 复用，什么都不写 ──────────
        if meta_path.is_file() {
            // 读不出来（旧格式 / 缺字段 / 被人手改坏）**不能让整个 attach 失败** ——
            // 那等于"这一期的文档整个打不开"。降级成按本地重新生成一份。
            let Ok(existing) = PeriodMeta::load(&meta_path) else {
                let mut m = PeriodMeta::new(branch.clone(), family.clone(), ws.title());
                m.repos = ws.meta().modules.clone();
                m.save(&meta_path)?;
                ws.meta_mut().doc = Some(crate::workspace::DocRef::new(
                    family.clone(),
                    branch.clone(),
                ));
                ws.save_meta()?;
                return Ok(AttachOutcome {
                    family: family.clone(),
                    label: m.label.clone(),
                    relative_dir: m.relative_dir(),
                    created: false,
                    attached: true,
                });
            };
            if ws.meta().doc.is_none() {
                ws.meta_mut().doc = Some(crate::workspace::DocRef::new(
                    existing.family.clone(),
                    existing.branch.clone(),
                ));
                ws.save_meta()?;
            }
            let outcome = AttachOutcome {
                family: existing.family.clone(),
                label: existing.label.clone(),
                relative_dir: existing.relative_dir(),
                created: false,
                attached: true,
            };
            return Ok(outcome);
        }

        // ── 3. 一条分支上哪儿都没有这一期：只有 Create 模式才生成 ────────
        //
        // ReuseOnly（导入需求）到这里就**停手**：远程没有这一期的文档，
        // 就不该由我们凭空写出几个文件来（那几个文件会挡住之后的 `git pull`，
        // 见 `AttachMode` 的注释）。worktree 已经拉好了，人就当"这个需求还没文档"；
        // 之后想写文档，界面上「挂上文档」走的是 Create，那才是本地起头的地方。
        if mode == AttachMode::ReuseOnly {
            return Ok(AttachOutcome {
                family,
                label: String::new(),
                relative_dir: String::new(),
                created: false,
                attached: false,
            });
        }
        std::fs::create_dir_all(&period).map_err(|e| Error::io(period.clone(), e))?;

        let mut m = PeriodMeta::new(branch.clone(), family.clone(), ws.title());
        m.label = self.next_label(&family, &branch, ws.title());
        m.repos = ws.meta().modules.clone();
        m.links = ws.meta().links.clone();
        m.notes = ws.meta().notes.clone();
        m.summary = ws.meta().notes.clone();
        m.save(&meta_path)?;
        // 期次说明也一样写两个名字（CLAUDE.md + AGENTS.md）。
        // **已经有了就不覆盖** —— 导入别人推上来的那一期时，目录里是**他写的**那份，
        // 盖掉既丢内容、又平白多出「未提交改动」（实测反馈）。
        crate::hub::write_agent_doc_missing_only(&period, &meta::render_claude_md(&m))?;

        // SQL 之类的多文件类型给个目录，免得用户还得自己 mkdir
        let _ = std::fs::create_dir_all(period.join("sql"));

        // ── 4. 回写工作区元数据 ─────────────────────────────────────────
        ws.meta_mut().doc = Some(crate::workspace::DocRef::new(
            family.clone(),
            branch.clone(),
        ));
        ws.save_meta()?;

        Ok(AttachOutcome {
            family,
            label: m.label.clone(),
            relative_dir: m.relative_dir(),
            created: true,
            attached: true,
        })
    }

    /// 把文档从工作区摘掉：删 worktree、清元数据里的 doc 归属。
    ///
    /// **不删 doc-hub 上的分支与期次目录**——文档还在，只是这个需求不再挂它。
    /// 简单修复类需求不需要文档；想用时 `attach` 能原样加回来（幂等）。
    pub fn detach(&self, ws: &mut Workspace) -> Result<bool> {
        let docs = self.docs_dir(ws);
        if !docs.join(".git").exists() && ws.meta().doc.is_none() {
            return Ok(false);
        }
        let git = Git::new(&self.root);
        let _ = git.worktree_remove(&docs, true);
        let _ = git.worktree_prune();
        // worktree 可能已被手工删过，确保路径真的不在了
        if docs.exists() {
            crate::workspace::lifecycle::remove_dir_verified(&docs)?;
        }
        ws.meta_mut().doc = None;
        ws.save_meta()?;
        Ok(true)
    }

    /// 保证 `ws/<分支>/docs` 是该需求分支的 worktree。
    fn ensure_worktree(&self, docs: &Path, branch: &str) -> Result<()> {
        let dh = Git::new(&self.root);
        // 清掉残留的 worktree 元数据，避免 add 撞「已存在」
        let _ = dh.worktree_prune();

        if docs.join(".git").exists() {
            return Ok(()); // 已挂好
        }
        if docs.exists() {
            // 目录在但不是 worktree（挂到一半）→ 清掉重建
            crate::workspace::lifecycle::remove_dir_verified(docs)?;
        }

        // 从哪建？**优先 `origin/<分支>`** —— 那一期文档的正文（含它自己的
        // CLAUDE.md / meta.json）就在这条远程分支上，是别人（或另一台机器上的你）
        // 推上去的。
        //
        // ⚠️ 原来写死从 `origin/main` 建，于是"拉下来的需求"只有一个空目录：
        // 正文一个都没有，而 attach 随后又**生成**了 CLAUDE.md / AGENTS.md /
        // meta.json —— 界面上就成了「什么都没改却提示 3 个改动」（实测反馈）。
        //
        // 远程没有这条分支（首次挂文档、还没推过）才退回 `origin/main`。
        //
        // ⚠️ **判之前先 fetch 一次**（本地没有这条分支时）：本地那份远程跟踪 ref
        // 可能是旧的，于是"其他人推上来的那一期"看不见 —— 后果是从 `origin/main`
        // 建了个空目录，接着 attach 以为"还没有这一期"而**凭空生成**一份：
        // 那几个未跟踪文件之后会挡住 `git pull`
        // （`untracked working tree files would be overwritten by merge`）。
        // 实测反馈：「导入的需求 doc-hub 多出 3 个改动，而且导致我无法拉取代码」。
        //
        // fetch 失败（离线 / 没配凭据）**不阻断**：那只是"没查到"，
        // 按"远程没有"处理，退回 `origin/main` 就是原来的行为。
        if !dh.local_branch_exists(branch) && !dh.remote_branch_exists(branch) {
            let _ = dh.fetch();
        }
        if dh.local_branch_exists(branch) {
            // 本地已有（此前挂过又删了 worktree）→ 直接检出它，别覆盖本地已有的东西
            dh.worktree_add(docs, branch, false, None)?;
        } else if dh.remote_branch_exists(branch) {
            dh.worktree_add(docs, branch, true, Some(&format!("origin/{branch}")))?;
        } else {
            dh.worktree_add(docs, branch, true, Some("origin/main"))?;
        }
        // ⚠️ **必须清掉上游**。从 `origin/main` 建分支时 git 会把上游设成 `origin/main`，
        // 于是 IDE 里点 Push 会推成 `feature-xxx:main` —— 被 GitLab 按「受保护分支」拒掉
        // （实测报错：`You are not allowed to push code to protected branches`）。
        // 这正是设计方案 §8 记的旧版坑，需求工作区那边修过，**文档这条漏了**。
        // 清掉之后 `push.default=simple` 会推到同名的远程分支，正是我们要的。
        //
        // ⚠️ **必须在 worktree 里跑**（`Git::new(docs)`），不能在 dh（主 clone）上跑 ——
        // 主 clone 检出的是 `main`，在那儿 unset 会把**主干自己的**上游清掉，
        // 而文档分支一点没动（这个错我第一版就写错了，被测试当场抓住）。
        let _ = crate::git::Git::new(docs).unset_upstream();
        Ok(())
    }

    /// 算本期在族内的标签：hotfix/bugfix 分支给 `BUGFIX`，其余按版本编号。
    ///
    /// # 为什么不只数「族里已有几期」
    ///
    /// 顺序编号数的是**文档库里**有几期 —— 上一期如果从没挂到文档库，
    /// 本期就会被编成 `V1`，而人心里它明明是 V2（实测反馈：
    /// 中文备注写着「示例需求v2」，却被识别成 V1）。
    ///
    /// 中文备注是人自己起的，里面写了 `v2` / `二期` 就是最可靠的信号，
    /// 优先用它。认不出来才退回顺序编号。
    fn next_label(&self, family: &str, branch: &str, title: &str) -> String {
        let lower = branch.to_ascii_lowercase();
        if lower.starts_with("hotfix") || lower.starts_with("bugfix") {
            return "BUGFIX".to_string();
        }
        if let Some(v) = version_from_title(title) {
            return v;
        }
        let count = self
            .families()
            .ok()
            .and_then(|fs| fs.into_iter().find(|f| f.name == family))
            .map(|f| f.periods.len())
            .unwrap_or(0);
        format!("V{}", count + 1)
    }

    /// 该需求的文档分支相对 doc-hub 主干**领先多少个提交**。
    ///
    /// 界面上「vs主干」那一列的语义跟代码模块一致：`0` = 已并入主干。
    /// 文档原来那一列是空的 —— 同样是 git 分支，凭什么看不到。
    ///
    /// 直接问 `origin/HEAD`（clone 时设好的、指向远程默认分支）—— **一次** git 调用。
    /// 原来是「探测主干叫什么 + 数一次」，两次；Windows 上每次调用 ~85ms，
    /// 而这里在进需求的必经之路上（实测反馈：进需求要转圈等两秒）。
    pub fn ahead_of_main(&self, ws: &Workspace) -> Result<u64> {
        let git = Git::new(self.docs_dir(ws));
        if let Some((ahead, _)) = git.ahead_behind_opt("origin/HEAD") {
            return Ok(ahead);
        }
        // `origin/HEAD` 缺失（手工 fetch 过、远程没设 HEAD）才回退
        let main = crate::git::detect_main_branch(&self.root)?;
        git.count_ahead("HEAD", &format!("origin/{main}"))
    }

    /// 改期次标签（`V1` / `V2` / `BUGFIX` …）。
    ///
    /// 标签本来是 `attach` 时按「该族第 N 期」自动编的，但**自动编号猜不出人的
    /// 意图**：同族的第二期未必叫 V2（可能是个 bugfix，或者人就想叫「二期」）。
    /// 所以允许改，改动落在期次的 `meta.json` 里（跟着文档走，别人拉下来也一致）。
    ///
    /// 只改文件、**不自动提交**：文档的提交时机由人决定（`doc sync`）。
    /// 改**需求分类**（原「文档族」）：把期次目录整棵挪到新分类下。
    ///
    /// # 为什么必须挪目录
    ///
    /// 分类名**就是目录名**（`workspaces/<分类>/<分支>/`）。只改元数据不挪目录，
    /// 结果是「元数据指向新分类、内容还躺在旧分类」—— 文档页按分类读，就再也找
    /// 不到这些文档，等于**文档丢了**。实测反馈。
    ///
    /// 用 `rename`（同盘是原子操作）：不会留下挪一半的中间态。
    /// 新分类下已有同名期次时**拒绝**（不覆盖别人/自己的东西）。
    ///
    /// 返回给人看的说明（`None` = 没挂文档或无需改动）。
    pub fn rename_family(&self, ws: &Workspace, new_family: &str) -> Result<Option<String>> {
        let Some(doc) = ws.meta().doc.clone() else {
            return Ok(None);
        };
        let new_family = new_family.trim();
        if new_family.is_empty() || new_family == doc.family {
            return Ok(None);
        }
        // 它会被当成目录名，先按目录名的规矩校验（空格、斜杠之类一律拒）
        crate::name::validate_component(new_family, "需求分类")?;

        let root = self.docs_dir(ws).join(WORKSPACES_DIR);
        let from = root.join(&doc.family).join(ws.branch());
        let to = root.join(new_family).join(ws.branch());
        if !from.exists() {
            // 挂了文档但目录不在（还没建/已归档到主干）—— 无事可做，别硬造
            return Ok(None);
        }
        if to.exists() {
            return Err(Error::other(format!(
                "新分类下已经有同名期次：{}（先处理它，这里不覆盖）",
                to.display()
            )));
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent.to_path_buf(), e))?;
        }
        std::fs::rename(&from, &to).map_err(|e| {
            Error::other(format!(
                "挪文档目录失败 {} → {}: {e}",
                from.display(),
                to.display()
            ))
        })?;

        // 旧分类空了就收掉：`workspaces/` 下留一堆空目录，文档页会显示成一堆空分类
        let old_dir = root.join(&doc.family);
        if std::fs::read_dir(&old_dir)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false)
        {
            let _ = std::fs::remove_dir(&old_dir);
        }

        let mut note = format!("文档目录已跟着挪到新分类：{new_family}/{}", ws.branch());
        // 已并入主干的副本在 `main` 上（共享的归档），工具**不替谁改它** ——
        // 改了就等于往主干上写别人的历史。但必须**说出来**，否则人以为全挪干净了。
        let archived = self.archived_in_main(&crate::git::Git::new(&self.root));
        if archived.contains(&format!("{}/{}", doc.family, ws.branch())) {
            note.push_str("（注意：已并入主干的旧副本仍在旧分类下 —— 那属于共享历史，没有动它）");
        }
        Ok(Some(note))
    }

    /// 改**需求中文名**：把期次元数据里的 `name` 一起更新。
    ///
    /// 文档页左列表显示的就是 `name`（比分支名友好），不跟着改就会出现
    /// 「需求列表一个新名、文档页一个旧名」。实测反馈。
    ///
    /// 返回是否真的改了（`false` = 没挂文档 / 名字没变）。
    pub fn sync_title(&self, ws: &Workspace) -> Result<bool> {
        if ws.meta().doc.is_none() {
            return Ok(false);
        }
        let dir = self.period_dir(ws);
        let path = dir.join(META_FILE);
        if !path.is_file() {
            return Ok(false);
        }
        let mut m = PeriodMeta::load(&path)?;
        let want = ws.title().to_string();
        if m.name == want {
            return Ok(false);
        }
        m.name = want;
        m.save(&path)?;
        // 期次说明文件里也带着名字 → 重刷一遍（CLAUDE.md / AGENTS.md 两份一起）
        crate::hub::write_agent_doc(&dir, &meta::render_claude_md(&m))?;
        Ok(true)
    }

    /// 改**涉及模块**：把工作区当前的模块列表写回期次元数据。
    ///
    /// 以前只在挂文档那一刻写一次，之后 `gws add` / `gws drop` 都不会跟着变，
    /// 文档页上「涉及模块」就一直是建期次时的那几个（实测反馈）。
    ///
    /// 返回是否真的改了（`false` = 没挂文档 / 期次目录不在 / 列表没变）。
    pub fn sync_repos(&self, ws: &Workspace) -> Result<bool> {
        if ws.meta().doc.is_none() {
            return Ok(false);
        }
        let path = self.period_dir(ws).join(META_FILE);
        if !path.is_file() {
            return Ok(false);
        }
        let mut m = PeriodMeta::load(&path)?;
        // 去重后再比：旧数据里可能已经有重复项，顺手修掉。
        let mut want = ws.meta().modules.clone();
        let mut seen = std::collections::BTreeSet::new();
        want.retain(|r| seen.insert(r.clone()));
        if m.repos == want {
            return Ok(false);
        }
        m.repos = want;
        m.save(&path)?;
        Ok(true)
    }

    pub fn set_label(&self, ws: &Workspace, label: &str) -> Result<String> {
        let doc = ws
            .meta()
            .doc
            .clone()
            .ok_or_else(|| Error::other("该需求还没挂文档目录，先跑 gws doc attach"))?;
        let dir = self
            .docs_dir(ws)
            .join(WORKSPACES_DIR)
            .join(&doc.family)
            .join(ws.branch());
        let path = dir.join(META_FILE);
        let mut m = PeriodMeta::load(&path)?;
        m.label = label.trim().to_string();
        m.save(&path)?;
        Ok(m.label)
    }

    /// 期次目录里待提交的文档改动（相对路径）。
    pub fn status(&self, ws: &Workspace) -> Result<Vec<String>> {
        let docs = self.docs_dir(ws);
        if !docs.join(".git").exists() {
            return Err(Error::other(format!(
                "该需求还没挂文档目录（{}）。先跑 gws doc attach",
                docs.display()
            )));
        }
        // 走 Git 的**唯一出口**：它负责滤掉 `.DS_Store` 这类系统垃圾。
        // 这里原来自己数 porcelain 行，于是那 4 个 .DS_Store 又变成了「4 个变更」
        // （推送闸门那边修过、这边漏了 —— 实测反馈）。
        Git::new(&docs).status_lines(true)
    }

    /// 提交并推送本文档分支（需求 #17：随代码一起提交）。
    ///
    /// 推送用 **HTTP Basic + token**，凭据通过环境变量注入 git 配置，
    /// 不落在 argv 或 `.git/config` 里（见 [`crate::git::Git::with_token`]）。
    pub fn sync(&self, ws: &Workspace, message: &str, push: bool) -> Result<DocSyncOutcome> {
        let docs = self.docs_dir(ws);
        let branch = ws.branch().to_string();
        if !docs.join(".git").exists() {
            return Err(Error::other("该需求还没挂文档目录，先跑 gws doc attach"));
        }
        let git = Git::new(&docs);
        let mut out = DocSyncOutcome::default();

        git.status(&["add", "-A"])?;
        // core.quotePath=false：git 默认把非 ASCII 路径转义成 \xxx 八进制
        // （`"workspaces/\347\247\201..."`），中文目录名根本没法看
        let staged = git.out(&[
            "-c",
            "core.quotePath=false",
            "diff",
            "--cached",
            "--name-only",
        ])?;
        out.changed = staged
            .lines()
            .map(str::to_string)
            .filter(|l| !l.is_empty())
            .collect();

        if out.changed.is_empty() {
            return Ok(out); // 没有变更就不提交，避免空提交
        }

        let msg = if message.trim().is_empty() {
            "docs: 文档更新"
        } else {
            message
        };
        // **不在这里指定身份**：用 local config 里的那份，也就是 hub 绑定的提交身份
        // （见 `git::apply_identity` / `DocHub::finish`）。原来这里写死
        // `-c user.name=gws -c user.email=gws@local`，会**完全绕过** per-hub 身份 ——
        // 文档提交的作者永远是那个假身份，而使用者看到的却是自己的名字。
        // 没配身份时 git 只会丢一句 `fatal: empty ident name`，先挡下来给人话
        crate::git::Git::ensure_commit_identity(&docs)?;
        git.status(&["commit", "--quiet", "-m", msg])?;
        out.committed = true;

        if push {
            let token = crate::gitlab::token::resolve(&self.hub_root)?;
            Git::new(&docs).with_token(token).push(&branch, true)?;
            out.pushed = true;
        }

        Ok(out)
    }

    /// 在期次目录里新建一个带模板的文档（需求 #2 的文档类型）。
    pub fn new_doc(&self, ws: &Workspace, key: &str) -> Result<PathBuf> {
        let doc = doc_type(key).ok_or_else(|| {
            let names: Vec<&str> = DOC_TYPES.iter().map(|t| t.key).collect();
            Error::other(format!(
                "未知的文档类型: {key}\n  可用类型: {}",
                names.join("、")
            ))
        })?;

        let period = self.period_dir(ws);
        if !period.is_dir() {
            return Err(Error::other("该需求还没挂文档目录，先跑 gws doc attach"));
        }
        let meta = PeriodMeta::load(&period.join(META_FILE))?;
        let path = period.join(doc.file);

        if path.exists() {
            return Err(Error::other(format!("文档已存在: {}", path.display())));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent.to_path_buf(), e))?;
        }
        std::fs::write(&path, meta::template(doc, &meta))
            .map_err(|e| Error::io(path.clone(), e))?;
        Ok(path)
    }
}

/// 族选择的结果：选了哪个 + 依据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FamilyChoice {
    pub family: String,
    /// 依据说明，供界面/命令展示，让人能判断选得对不对。
    pub reason: &'static str,
}

/// 扫一个工作树目录下的 `workspaces/<族>/<期次>/meta.json`。
///
/// 单期元数据损坏只跳过它，不让整个列表失败——一个坏文件不该让人看不到
/// 其它所有需求。
fn scan_worktree(dir: &Path) -> Vec<(PeriodMeta, bool)> {
    let mut out = Vec::new();
    let Ok(families) = std::fs::read_dir(dir) else {
        return out;
    };
    for family in families.flatten() {
        let Ok(periods) = std::fs::read_dir(family.path()) else {
            continue;
        };
        for p in periods.flatten() {
            let meta_path = p.path().join(META_FILE);
            if meta_path.is_file() {
                if let Ok(m) = PeriodMeta::load(&meta_path) {
                    let has_content = period_has_content(&p.path());
                    out.push((m, has_content));
                }
            }
        }
    }
    out
}

/// 同一期次可能同时在工作树和多个分支上出现：元数据取第一个，
/// 但「有没有正文」要**任一处有就算有**（否则工作树是空的、
/// 远程分支上有正文时会被误藏）。
fn merge_period(
    all: &mut Vec<(PeriodMeta, bool)>,
    seen: &mut std::collections::BTreeMap<String, usize>,
    meta: PeriodMeta,
    has_content: bool,
) {
    if let Some(&i) = seen.get(&meta.branch) {
        all[i].1 |= has_content;
    } else {
        seen.insert(meta.branch.clone(), all.len());
        all.push((meta, has_content));
    }
}

/// 某个 rev 上所有期次元数据的路径。
///
/// 期次目录固定是 `workspaces/<族>/<期次>/meta.json` —— 恰好四段。
/// 按段数过滤可以避免把族级别的其它 json 也扫进来。
fn meta_paths_in(git: &Git, rev: &str) -> Result<Vec<String>> {
    Ok(git
        .out(&["ls-tree", "-r", "--name-only", rev, "--", WORKSPACES_DIR])?
        .lines()
        .map(str::trim)
        .filter(|l| l.ends_with(META_FILE) && l.split('/').count() == 4)
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_is_no_builtin_doc_hub_url() {
        // 开源版不预设文档库地址：每个人放哪儿都不一样，建 hub 时必填。
        // 预设一个地址等于替用户做了决定，而且必然指向别人家的仓库。
        assert!(DEFAULT_URL.is_empty(), "不应有内置文档库地址: {DEFAULT_URL}");
    }
}

#[cfg(test)]
mod label_tests {
    use super::version_from_title;

    #[test]
    fn version_is_read_from_the_chinese_title() {
        // 实测反馈：中文备注写着「示例需求v2」，却被按顺序编成了 V1 ——
        // 因为顺序编号数的是**文档库里**有几期，而上一期从没挂进来。
        assert_eq!(version_from_title("示例需求v2").as_deref(), Some("V2"));
        assert_eq!(version_from_title("示例需求V3").as_deref(), Some("V3"));
        assert_eq!(version_from_title("增长二期").as_deref(), Some("V2"));
        assert_eq!(version_from_title("结算改版第三期").as_deref(), Some("V3"));
    }

    #[test]
    fn no_guess_when_the_title_says_nothing() {
        // 没写版本就不猜 —— 猜错比不猜更糟，用户会以为系统知道从而不去改
        assert_eq!(version_from_title("收银台改版"), None);
        assert_eq!(version_from_title("private domain reach"), None);
        // 光是 v 后面没数字不算
        assert_eq!(version_from_title("vip 活动"), None);
    }
}
