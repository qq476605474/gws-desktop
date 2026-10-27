//! 工作区生命周期：创建 / 增删模块 / 删除。
//!
//! ## 事务性
//!
//! `new` 采用**全有或全无**：先把所有模块预检一遍，任何一个不过就整体拒绝，
//! 什么都不建；建到一半失败则回滚已建的。旧版同款行为——
//! 半残的工作区比没有工作区更难收拾。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::git::Git;
use crate::hub::Hub;
use crate::name;
use crate::repo::{self, Outcome, Repo, RepoOutcome};
use crate::workspace::{Workspace, WorkspaceMeta, META_FILE};
use crate::{Error, Result};

// ══ 基线解析 ═══════════════════════════════════════════════════════════
//
// 基线链按优先级逐个来源检查该模块是否有对应分支，第一个命中的作为基线；
// 全不命中自动兜底主干（无需在链里写主干）。旧版同款逻辑。

/// 一次基线解析的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaseResolution {
    /// 实际使用的 ref，如 `origin/feature-20260901-smart-reach`。
    pub reference: String,
    /// 因为「该模块没有此分支」而跳过的来源（空 = 首选即命中）。
    pub missed: Vec<String>,
    /// 是否降级到了主干。
    pub fell_back_to_main: bool,
}

impl BaseResolution {
    /// 给用户看的一句话说明（如「按优先级回退（阶段1 无此分支）」）。
    pub fn note(&self) -> Option<String> {
        if self.missed.is_empty() {
            None
        } else if self.fell_back_to_main {
            Some(format!(
                "上游全部无此分支（{}），降级主干",
                self.missed.join("、")
            ))
        } else {
            Some(format!(
                "按优先级回退（{} 无此分支）",
                self.missed.join("、")
            ))
        }
    }
}

/// 基线链里表示「默认（主干）」的写法。
///
/// 写进 `.workspace.json` 只会让人以为「填了个东西」，而语义上它就是空的 ——
/// 所以一律不存。`resolve_base` 仍然认这些写法（手改文件的人可能这么写）。
fn is_trunk_keyword(s: &str) -> bool {
    matches!(s, "主干" | "main" | "master" | "MAIN" | "MASTER")
}

/// **读**路径用的宽松清洗：丢掉空白与「主干」这类等价写法，去掉重复。
///
/// 绝不返回错误 —— 一个字段不规范不该让整个需求打不开。
pub fn sanitise_base_chain(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in raw {
        let s = item.trim();
        if s.is_empty() || is_trunk_keyword(s) {
            continue;
        }
        if !out.iter().any(|e| e == s) {
            out.push(s.to_string());
        }
    }
    out
}

/// **写**路径用的严格校验：错就报错，并说清错在哪一项。
///
/// 实测反馈：填入的内容要校验 —— 不能重复、命名要合法（例如分支名里不能有空格）。
/// 静默接受再在后续某个操作里以奇怪的方式失败，比当场说清楚难查得多。
pub fn normalise_base_chain(raw: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for item in raw {
        let s = item.trim();
        if s.is_empty() || is_trunk_keyword(s) {
            continue; // 「主干」= 默认，不存
        }
        let target = s.strip_prefix("ws:").map(str::trim).unwrap_or(s);
        if target.is_empty() {
            return Err(Error::other(
                "基线来源里有一项只写了 `ws:`，后面要跟上游需求的分支名".to_string(),
            ));
        }
        check_ref_name(target)
            .map_err(|why| Error::other(format!("基线来源「{s}」不合法：{why}")))?;
        if out.iter().any(|e| e == s) {
            return Err(Error::other(format!(
                "基线来源里有重复项：「{s}」只能写一次"
            )));
        }
        out.push(s.to_string());
    }
    Ok(out)
}

/// 粗略按 git 的引用名规则校验（足够挡住手误，不追求穷尽 `check-ref-format`）。
fn check_ref_name(name: &str) -> std::result::Result<(), String> {
    if name.chars().any(|c| c.is_whitespace()) {
        return Err("分支名里不能有空格".to_string());
    }
    if name.contains("..") {
        return Err("不能包含连续的两个点 `..`".to_string());
    }
    if name.contains("@{") {
        return Err("不能包含 `@{`".to_string());
    }
    if name.contains("//") {
        return Err("不能包含连续的两个斜杠".to_string());
    }
    if let Some(c) = name.chars().find(|c| "~^:?*[\\]".contains(*c)) {
        return Err(format!("不能包含字符 `{c}`"));
    }
    if name.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}') {
        return Err("不能包含控制字符".to_string());
    }
    if name.starts_with('-') || name.starts_with('/') {
        return Err("不能以 `-` 或 `/` 开头".to_string());
    }
    if name.ends_with('/') || name.ends_with('.') {
        return Err("不能以 `/` 或 `.` 结尾".to_string());
    }
    if name.ends_with(".lock") {
        return Err("不能以 `.lock` 结尾".to_string());
    }
    Ok(())
}

/// 解析某模块的基线。
pub fn resolve_base(hub: &Hub, module: &str, chain: &[String]) -> Result<BaseResolution> {
    let repo = Repo::open(hub, module)?;
    let git = repo.git();
    let main_ref = format!("origin/{}", repo.main_branch()?);

    let mut missed = Vec::new();
    for raw in chain {
        let src = raw.trim();
        if src.is_empty() {
            continue;
        }

        let candidate = match src {
            "主干" | "main" | "master" => main_ref.clone(),
            s if s.starts_with("ws:") => {
                let upstream = &s[3..];
                let dir = hub.ws_dir().join(upstream);
                if !dir.join(META_FILE).is_file() {
                    missed.push(src.to_string());
                    continue;
                }
                // 上游工作区的分支名要从它的元数据读，而不是从目录名猜
                let up = Workspace::from_dir(&dir)?;
                format!("origin/{}", up.branch())
            }
            s if s.starts_with("origin/") => s.to_string(),
            s => format!("origin/{s}"),
        };

        if git.ref_exists(&candidate) {
            return Ok(BaseResolution {
                reference: candidate,
                missed,
                fell_back_to_main: false,
            });
        }
        missed.push(src.to_string());
    }

    Ok(BaseResolution {
        reference: main_ref,
        missed,
        fell_back_to_main: true,
    })
}

/// 解析「对比代码」用的基线：**配置链里最后配置的那个**可用基线。
///
/// 与 [`resolve_base`] 的区别：
/// - `resolve_base` 从链头开始取（建 worktree 时用，行为不能变）
/// - 这里**倒着走配置链**，取第一个在该模块存在的 ref —— 后配置的优先。
///   不按分支名里的日期猜：日期只说明建得早晚，不代表谁是谁的基线。
///
/// 全部不存在时兜底主干的 `origin/<main>`。
pub fn resolve_latest_base(hub: &Hub, module: &str, chain: &[String]) -> Result<BaseResolution> {
    let repo = Repo::open(hub, module)?;
    let git = repo.git();
    let _ = git.fetch();
    let main_ref = format!("origin/{}", repo.main_branch()?);

    // (原始写法, 候选 ref)
    let mut candidates: Vec<(String, String)> = Vec::new();
    let mut missed = Vec::new();

    for raw in chain {
        let src = raw.trim();
        if src.is_empty() || is_trunk_keyword(src) {
            continue;
        }
        let reference = match src {
            s if s.starts_with("ws:") => {
                let upstream = s[3..].trim();
                let dir = hub.ws_dir().join(upstream);
                if !dir.join(META_FILE).is_file() {
                    missed.push(src.to_string());
                    continue;
                }
                let up = Workspace::from_dir(&dir)?;
                format!("origin/{}", up.branch())
            }
            s if s.starts_with("origin/") => s.to_string(),
            s => format!("origin/{s}"),
        };
        candidates.push((src.to_string(), reference));
    }

    // 配置链倒序：最后写的那个最先比较
    for (raw, reference) in candidates.iter().rev() {
        if git.ref_exists(reference) {
            return Ok(BaseResolution {
                reference: reference.clone(),
                missed,
                fell_back_to_main: false,
            });
        }
        missed.push(raw.clone());
    }

    Ok(BaseResolution {
        reference: main_ref,
        missed,
        fell_back_to_main: true,
    })
}

// ══ 预检失败 ═══════════════════════════════════════════════════════════

/// 创建工作区前的预检结果。任何一项非空都表示不能创建。
///
/// 可序列化是为了让桌面端**分类展示**并给不同的下一步建议，而不是把整段
/// 中文文本塞进一个弹窗。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct PrecheckFailure {
    /// 仓库不在 `repos/` 里。
    pub missing_repos: Vec<String>,
    /// 目标分支已存在于某仓库的远程 → 这多半是别人的需求，应该用 `gws get` 拉。
    pub remote_branch_exists: Vec<String>,
    /// 目标分支已存在于本地（多半是删目录没清干净）。
    pub local_branch_exists: Vec<String>,
    /// 基线解析不到（模块、基线 ref）。
    pub base_missing: Vec<(String, String)>,
}

impl PrecheckFailure {
    pub fn is_empty(&self) -> bool {
        self.missing_repos.is_empty()
            && self.remote_branch_exists.is_empty()
            && self.local_branch_exists.is_empty()
            && self.base_missing.is_empty()
    }
}

impl std::fmt::Display for PrecheckFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "预检未通过，工作区未创建（没有留下任何残留）")?;
        if !self.missing_repos.is_empty() {
            writeln!(f, "\n以下仓库不存在于 repos/：")?;
            for m in &self.missing_repos {
                writeln!(f, "  ✗ {m}")?;
            }
            writeln!(
                f,
                "  添加仓库: gws repo add <git地址>    查看已有: gws repo ls"
            )?;
        }
        if !self.remote_branch_exists.is_empty() {
            writeln!(f, "\n分支已存在于远程，可能他人的需求用同名分支：")?;
            for m in &self.remote_branch_exists {
                writeln!(f, "  ✗ {m}")?;
            }
            writeln!(f, "  若这是他人已创建的需求，改用: gws get <分支名>")?;
            writeln!(f, "  若要另建新需求，换个分支名")?;
        }
        if !self.local_branch_exists.is_empty() {
            writeln!(f, "\n分支已存在于本地（可能是此前删目录未清理干净）：")?;
            for m in &self.local_branch_exists {
                writeln!(f, "  ✗ {m}")?;
            }
            writeln!(f, "  查看: gws ls    清理: gws clean")?;
        }
        if !self.base_missing.is_empty() {
            writeln!(f, "\n基线不存在：")?;
            for (m, b) in &self.base_missing {
                writeln!(f, "  ✗ {m}: {b}")?;
            }
        }
        Ok(())
    }
}

// ══ 创建 ═══════════════════════════════════════════════════════════════

/// 创建需求工作区的参数。
#[derive(Debug, Clone)]
pub struct NewOptions {
    /// 完整分支名（同时是目录名）。
    pub branch: String,
    /// 中文备注。留空则用分支名。
    pub title: String,
    /// 分支前缀，仅用于记录。
    pub prefix: String,
    /// 基线链，顺序即优先级。
    pub base: Vec<String>,
    /// 涉及的模块（仓库名）。
    pub modules: Vec<String>,
    /// doc-hub 需求分类中文名。`None` 表示暂不纳入文档管理。
    pub doc_family: Option<String>,
    /// 自由备注（建的时候就能写，不用建完再进「需求设置」补）。
    pub notes: String,
    /// 自定义信息（tapd / 飞书 …）。同样是建的时候就能填。
    pub links: Vec<crate::workspace::Link>,
}

/// 创建结果。
#[derive(Debug, Clone, Serialize)]
pub struct NewOutcome {
    pub branch: String,
    pub created: Vec<String>,
    pub base_resolved: BTreeMap<String, String>,
    /// 基线降级说明：`模块 → 说明`。
    pub base_notes: BTreeMap<String, String>,
    /// 这次要不要挂文档（= 模块列表里勾了 doc-hub）。
    ///
    /// 单独报出来，是因为「挂文档」这一步在 `new` 之外（要联网克隆 doc-hub，
    /// 失败不该让工作区创建失败）。调用方据此决定要不要走那一步。
    pub wants_docs: bool,
}

/// 创建工作区。**全有或全无**：预检不过什么都不建，建到一半失败则回滚。
pub fn new(hub: &Hub, opts: &NewOptions) -> Result<NewOutcome> {
    name::validate_component(&opts.branch, "分支名")?;
    if opts.modules.is_empty() {
        return Err(Error::other("至少要指定一个模块"));
    }

    let dir = hub.ws_dir().join(&opts.branch);
    if dir.exists() {
        return Err(Error::WorkspaceExists(opts.branch.clone()));
    }

    // 文档库（doc-hub）**不是普通模块**：它有自己那套挂载流程（建 doc-hub 分支的
    // worktree、建期次目录、写 meta.json、认需求分类）。但它出现在模块列表里、
    // 默认选中（一般需求都需要文档），所以要在这里分流：
    //   - 从「要建 worktree 的模块」里剔掉
    //   - 出现即表示这次要挂文档（`doc_family` 仍然负责给族命名）
    let want_docs = opts.modules.iter().any(|m| hub.is_dochub(m));
    let mut modules: Vec<String> = Vec::new();
    let mut seen = BTreeSet::new();
    for m in &opts.modules {
        if hub.is_dochub(m) {
            continue;
        }
        name::validate_component(m, "模块名")?;
        if seen.insert(m.clone()) {
            modules.push(m.clone());
        }
    }
    // 只勾了文档、一个代码模块都没有：仍允许（有些需求只改文档）
    // ——但至少要有一样东西，否则这个工作区没有意义
    if modules.is_empty() && !want_docs {
        return Err(Error::other("至少要指定一个模块"));
    }

    // 清掉手工删目录 / 中断残留的 worktree 元数据，避免 add 撞已存在路径
    prune_all_repos(hub)?;

    // 基线先按严格规则归一化 + 校验（去「主干」这类等价写法、查重复、查命名），
    // 之后所有解析都用这个干净的值 —— 存进元数据的也必须是它。
    // 实测反馈 #4：以前清空基线会写入字面量「主干」，界面看着像填了个东西。
    let base = normalise_base_chain(&opts.base)?;

    // ── 预检：全部模块通过才开始创建 ──────────────────────────────────
    // 分支已存在于远程时，本地 `worktree add -b` 仍会成功，却建出与远程同名而
    // 内容无关的分支，直到 push 才暴露分叉——所以必须动手前查远程。
    let mut fail = PrecheckFailure::default();
    let mut resolved: BTreeMap<String, BaseResolution> = BTreeMap::new();

    for m in &modules {
        let Ok(repo) = Repo::open(hub, m) else {
            fail.missing_repos.push(m.clone());
            continue;
        };
        let git = repo.git();
        let _ = git.fetch();

        if git.remote_branch_exists(&opts.branch) {
            fail.remote_branch_exists.push(m.clone());
            continue;
        }
        if git.local_branch_exists(&opts.branch) {
            fail.local_branch_exists.push(m.clone());
            continue;
        }

        let res = resolve_base(hub, m, &base)?;
        if !git.ref_exists(&res.reference) {
            fail.base_missing.push((m.clone(), res.reference.clone()));
            continue;
        }
        resolved.insert(m.clone(), res);
    }

    if !fail.is_empty() {
        return Err(Error::Precheck(Box::new(fail)));
    }

    // ── 创建 ──────────────────────────────────────────────────────────
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(dir.clone(), e))?;

    let mut created: Vec<String> = Vec::new();
    let mut failure: Option<(String, String)> = None;

    for m in &modules {
        let repo = Repo::open(hub, m)?;
        let base = &resolved[m].reference;
        let wt = dir.join(m);

        match repo.git().worktree_add(&wt, &opts.branch, true, Some(base)) {
            Ok(()) => {
                // 从 origin/<主干> 建分支时 git 会把上游设成主干，必须解除，
                // 否则 IDE 的 Push 会推向 origin/main 而不是需求分支
                let _ = Git::new(&wt).unset_upstream();
                created.push(m.clone());
            }
            Err(e) => {
                failure = Some((m.clone(), e.to_string()));
                break;
            }
        }
    }

    if let Some((module, reason)) = failure {
        rollback(hub, &dir, &opts.branch, &created);
        return Err(Error::other(format!(
            "模块 {module} 创建失败（已回滚 {} 个已建模块，无残留）: {reason}",
            created.len()
        )));
    }

    // ── 落元数据与 CLAUDE.md ──────────────────────────────────────────
    let mut meta = WorkspaceMeta::new(
        opts.branch.clone(),
        opts.title.clone(),
        opts.prefix.clone(),
        base.clone(),
        created.clone(),
    );
    meta.base_resolved = resolved
        .iter()
        .map(|(k, v)| (k.clone(), v.reference.clone()))
        .collect();
    // 备注与自定义信息：建的时候就能填（实测反馈：原来只能建完再进设置补）
    meta.notes = opts.notes.clone();
    meta.links = opts.links.clone();

    if want_docs {
        // 没填分类就按年月（`2026-10`）—— 与 `DocHub::resolve_family` 同一套规则。
        // 以前这里拿标题当族名预登记，`attach` 一看"已登记"就直接用了，
        // 于是没填分类的需求被归进了 `test` 这种标题族（实测反馈）。
        let family = opts
            .doc_family
            .clone()
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| crate::dochub::DocHub::month_family_name(&meta.created));
        meta.doc = Some(crate::workspace::DocRef::new(family, opts.branch.clone()));
    }
    meta.save(&dir.join(META_FILE))?;

    let _ws = Workspace::from_dir(&dir)?;

    let base_notes = resolved
        .iter()
        .filter_map(|(m, r)| r.note().map(|n| (m.clone(), n)))
        .collect();

    Ok(NewOutcome {
        branch: opts.branch.clone(),
        created,
        base_resolved: meta.base_resolved.clone(),
        base_notes,
        wants_docs: want_docs,
    })
}

fn rollback(hub: &Hub, dir: &Path, branch: &str, created: &[String]) {
    for m in created {
        if let Ok(repo) = Repo::open(hub, m) {
            let _ = repo.git().worktree_remove(&dir.join(m), true);
            let _ = repo.git().status(&["branch", "-D", branch]);
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// 清理所有仓库的僵尸 worktree 元数据。
pub fn prune_all_repos(hub: &Hub) -> Result<()> {
    for r in repo::list(hub)? {
        let _ = r.git().worktree_prune();
    }
    Ok(())
}

// ══ 增删模块 ═══════════════════════════════════════════════════════════

/// 把文档库当作「模块」加进来 —— 语义等同于「挂上文档」。
///
/// 实测反馈：界面上的「挂上文档」按钮虽然好，但有些人习惯在模块列表里统一
/// 管理 —— 那就别逼他只能走那一个入口，两条路应该等价。
fn add_docs_module(hub: &Hub, ws: &mut Workspace, module: &str) -> RepoOutcome {
    if ws.meta().doc.is_some() {
        return RepoOutcome {
            name: module.to_string(),
            outcome: Outcome::Skipped("文档已挂在本需求上".into()),
        };
    }
    match crate::dochub::DocHub::ensure_local(hub).and_then(|dh| dh.attach(ws, None).map(|_| ())) {
        Ok(()) => RepoOutcome {
            name: module.to_string(),
            outcome: Outcome::Ok,
        },
        Err(e) => RepoOutcome {
            name: module.to_string(),
            outcome: Outcome::Failed(e.to_string()),
        },
    }
}

/// 往已有工作区里加模块。逐个回报，单个失败不影响其余。
pub fn add_modules(hub: &Hub, ws: &mut Workspace, modules: &[String]) -> Result<Vec<RepoOutcome>> {
    let branch = ws.branch().to_string();
    let mut results = Vec::new();
    let mut added = Vec::new();

    for m in modules {
        // 文档库要**允许**加进来，只是走它自己那套挂载流程（建期次目录、写
        // meta.json、认需求分类），而不是当普通仓库建 worktree。
        //
        // 实测反馈：界面上的「挂上文档」按钮虽然好，但有些人习惯在模块列表里
        // 统一管理 —— 那就别逼他只能走那一个入口，两条路应该等价。
        if hub.is_dochub(m) {
            results.push(add_docs_module(hub, ws, m));
            continue;
        }
        if ws.meta().has_module(m) && ws.module_path(m).exists() {
            results.push(RepoOutcome {
                name: m.clone(),
                outcome: Outcome::Skipped("已在工作区中".into()),
            });
            continue;
        }
        let Ok(repo) = Repo::open(hub, m) else {
            results.push(RepoOutcome {
                name: m.clone(),
                outcome: Outcome::Failed("仓库不存在".into()),
            });
            continue;
        };

        let wt = ws.module_path(m);
        let git = repo.git();
        let outcome = if git.local_branch_exists(&branch) {
            // 该模块此前参与过这个需求，分支已存在 → 直接检出
            match git.worktree_add(&wt, &branch, false, None) {
                Ok(()) => Outcome::Ok,
                Err(e) => Outcome::Failed(e.to_string()),
            }
        } else {
            match resolve_base(hub, m, &ws.meta().base) {
                Ok(res) => match git.worktree_add(&wt, &branch, true, Some(&res.reference)) {
                    Ok(()) => {
                        let _ = Git::new(&wt).unset_upstream();
                        ws.meta_mut()
                            .base_resolved
                            .insert(m.clone(), res.reference.clone());
                        Outcome::Ok
                    }
                    Err(e) => Outcome::Failed(e.to_string()),
                },
                Err(e) => Outcome::Failed(e.to_string()),
            }
        };

        if outcome == Outcome::Ok {
            added.push(m.clone());
        }
        results.push(RepoOutcome {
            name: m.clone(),
            outcome,
        });
    }

    // 依据实际存在的模块目录回写列表（可能有模块之前被手工删掉）
    sync_modules(ws);
    ws.save_meta()?;

    Ok(results)
}

/// 把某模块移出工作区（删 worktree 与分支）。
///
/// 有未提交改动时**拒绝**（除非 `force`）——旧版同款守卫。
pub fn drop_module(hub: &Hub, ws: &mut Workspace, module: &str, force: bool) -> Result<Outcome> {
    let wt = ws.module_path(module);
    if !wt.exists() {
        return Ok(Outcome::Skipped("不在工作区中".into()));
    }

    let git = Git::new(&wt);
    let dirty = git.dirty_count()?;
    if dirty > 0 && !force {
        return Ok(Outcome::Failed(format!(
            "有 {dirty} 个未提交改动，先提交或加 --force 丢弃"
        )));
    }

    let repo = Repo::open(hub, module)?;
    if let Err(e) = repo.git().worktree_remove(&wt, true) {
        return Ok(Outcome::Failed(e.to_string()));
    }
    let _ = repo.git().status(&["branch", "-D", ws.branch()]);

    sync_modules(ws);
    ws.save_meta()?;
    Ok(Outcome::Ok)
}

/// 清理「元数据登记了但目录已不在」的模块。
///
/// 对应旧版 `gws drop`（无参数）：用户直接 `rm -rf` 掉模块目录后留下的悬空
/// worktree 元数据，需要 prune 掉；分支则按「是否已推到远程」决定删不删
/// （有未推送提交时保留，避免丢代码）。旧版逻辑。
pub fn drop_missing_modules(hub: &Hub, ws: &mut Workspace) -> Result<DropMissingOutcome> {
    let branch = ws.branch().to_string();
    let mut cleaned = Vec::new();
    let mut kept = Vec::new();

    for m in ws.meta().modules.clone() {
        if ws.module_path(&m).exists() {
            continue;
        }
        let Ok(repo) = Repo::open(hub, &m) else {
            continue;
        };
        let git = repo.git();
        let _ = git.worktree_prune();

        if git.remote_branch_exists(&branch) {
            // 已推送到远程 → 删本地分支不会丢提交
            let _ = git.status(&["branch", "-D", &branch]);
            cleaned.push(m);
        } else {
            // 从未推送：只有确认没有领先主干的提交，删分支才安全。
            // 这里读不到计数会直接返回错误而不是当成 0——当成 0 就是删掉人家
            // 还没推的代码。
            let main = repo.main_branch()?;
            let ahead = git.count_ahead(&branch, &format!("origin/{main}"))?;
            if ahead > 0 {
                kept.push((m, format!("{ahead} 个提交未推送")));
            } else {
                let _ = git.status(&["branch", "-D", &branch]);
                cleaned.push(m);
            }
        }
    }

    sync_modules(ws);
    ws.save_meta()?;
    Ok(DropMissingOutcome { cleaned, kept })
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DropMissingOutcome {
    pub cleaned: Vec<String>,
    pub kept: Vec<(String, String)>,
}

/// 依据**磁盘上实际存在**的模块目录回写 `modules` 字段。
///
/// 必须扫目录而不是过滤旧列表——`add_modules` 刚加的模块还没登记进元数据。
/// 旧版同款做法。
pub fn sync_modules(ws: &mut Workspace) {
    let present = ws.scan_module_dirs();
    ws.meta_mut().modules = present;
}

// ══ 删除工作区 ═════════════════════════════════════════════════════════

/// 删除前的风险项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Risk {
    pub module: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RemoveOutcome {
    pub branch: String,
    pub removed_modules: Vec<String>,
    /// 检出但被忽略的风险项。
    pub risks: Vec<Risk>,
    /// 是否带着 `--force` 执行的。
    ///
    /// **必须与 `risks` 分开看**：`risks` 非空既可能意味着"因此没删"，
    /// 也可能意味着"带着风险删了"。只看 `risks` 渲染会把后者报成"未删除"，
    /// 而东西其实已经没了——这是会让人以为数据还在的严重误导。
    pub forced: bool,
}

/// 删除整个需求工作区。
///
/// `force=false` 时，存在未保存的工作就整体拒绝并列出风险项。
pub fn remove(hub: &Hub, branch: &str, force: bool) -> Result<RemoveOutcome> {
    let ws = Workspace::open(hub, branch)?;
    let dir = ws.dir().to_path_buf();

    // 元数据先读全——后面 rm -rf 掉目录就读不到了
    let modules = ws.meta().modules.clone();

    let mut risks = Vec::new();
    for m in &modules {
        let wt = ws.module_path(m);
        if !wt.exists() {
            continue;
        }
        let git = Git::new(&wt);
        let dirty = git.dirty_count()?;
        if dirty > 0 {
            risks.push(Risk {
                module: m.clone(),
                reason: format!("{dirty} 个未提交改动"),
            });
        }
        if let Some(reason) = lost_work(hub, &wt, m, branch)? {
            risks.push(Risk {
                module: m.clone(),
                reason,
            });
        }
    }

    let forced = !risks.is_empty() && force;
    if !risks.is_empty() && !force {
        return Ok(RemoveOutcome {
            branch: branch.to_string(),
            removed_modules: vec![],
            risks,
            forced: false,
        });
    }

    let mut removed = Vec::new();
    for m in &modules {
        if !ws.module_path(m).exists() {
            continue;
        }
        if let Ok(repo) = Repo::open(hub, m) {
            let _ = repo.git().worktree_remove(&ws.module_path(m), true);
            let _ = repo.git().status(&["branch", "-D", branch]);
            removed.push(m.clone());
        }
    }

    // 文档 worktree 也要摘掉：它在 doc-hub 的 worktree 注册表里，直接删目录会
    // 留下悬空引用，之后 doc-hub 上的 `worktree add` 可能撞上失效路径。
    detach_docs(hub, &dir);

    remove_dir_verified(&dir)?;

    Ok(RemoveOutcome {
        branch: branch.to_string(),
        removed_modules: removed,
        risks,
        forced,
    })
}

/// 判断删掉本地分支会不会丢提交；`None` 表示不会丢。
///
/// 本地远程引用可能过时：其他人刚把 MR 合了、远程源分支被删，而这边还没 fetch。
/// 所以**判出风险时先短超时 fetch 复核一次**再决定——旧引用不该拦人；
/// 网络不通就按本地判断（宁可多问一次，也不静默放行）。
fn lost_work(hub: &Hub, wt: &Path, module: &str, branch: &str) -> Result<Option<String>> {
    let git = Git::new(wt);
    let local = lost_work_reason(&git, hub, module, branch)?;
    if local.is_some() && git.fetch_quick() {
        return lost_work_reason(&git, hub, module, branch);
    }
    Ok(local)
}

fn lost_work_reason(git: &Git, hub: &Hub, module: &str, branch: &str) -> Result<Option<String>> {
    if git.remote_branch_exists(branch) {
        let unpushed = git.count_ahead("HEAD", &format!("origin/{branch}"))?;
        return Ok((unpushed > 0).then(|| format!("{unpushed} 个提交未推送")));
    }

    // 远程没有这条分支：可能是从没推过，也可能是 **MR 合并后远程分支被删了**
    // （GitLab 勾了「合并后删除源分支」，或跑过 `gws branch-prune`）。
    // 不能再一律报「从未推送到远程」——按内容判断：本地提交全在主干里
    // （含 squash / rebase 合并）就没有东西可丢。
    let Ok(repo) = Repo::open(hub, module) else {
        // 仓库都打不开就查不了有没有未推送提交，按风险处理而不是放行
        return Ok(Some("仓库打不开，无法确认是否有未推送提交".into()));
    };
    let trunk = format!("origin/{}", repo.main_branch()?);
    let ahead = git.count_ahead("HEAD", &trunk)?;
    if ahead == 0 {
        return Ok(None);
    }
    // `cherry` 读不到时保守按 ahead 算——宁可多问一次，也不静默丢提交
    let lost = git.count_not_upstream(&trunk).unwrap_or(ahead);
    Ok((lost > 0).then(|| format!("{lost} 个提交未推送也未并入主干")))
}

/// 把工作区挂着的 doc-hub worktree 摘掉（如果挂了）。
///
/// 失败不阻断删除——工作区本身才是主体，文档 worktree 的元数据可以
/// 事后用 `git worktree prune` 清掉。
fn detach_docs(hub: &Hub, ws_dir: &Path) {
    let docs = ws_dir.join(hub.dochub_name());
    if !docs.join(".git").exists() {
        return;
    }
    let doc_hub = hub.dochub_dir();
    if !doc_hub.join(".git").is_dir() {
        return;
    }
    let git = Git::new(&doc_hub);
    let _ = git.worktree_remove(&docs, true);
    let _ = git.worktree_prune();
}

/// 删除目录并**校验它真的消失了**。
///
/// 旧版最典型的缺陷（ 无错误检查，紧接着无条件打印「✓ 工作区已删除」）：
/// `rm -rf` 是尽力而为的，碰到被占用或无权限的文件会删掉能删的、跳过错过的，
/// 然后返回非零——脚本不看返回值照样报成功。作者的 hub 里因此攒了残留目录。
pub fn remove_dir_verified(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let _ = std::fs::remove_dir_all(path);
    if !path.exists() {
        return Ok(());
    }

    // 常见原因是文件被占用或权限不足：去掉写保护再试一次
    make_writable_recursive(path);
    let _ = std::fs::remove_dir_all(path);
    if path.exists() {
        return Err(Error::RemoveFailed(path.to_path_buf()));
    }
    Ok(())
}

fn make_writable_recursive(path: &Path) {
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.permissions().readonly() {
            make_writable_one(&p, &meta);
        }
        if meta.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&p) {
                stack.extend(entries.flatten().map(|e| e.path()));
            }
        }
    }
}

/// 只给**属主**加写权限。不整体 `set_readonly(false)`，因为那在 Unix 上会把
/// 权限放宽到 0o222（组和其他人也可写）——虽然马上要删，但没必要。
#[cfg(unix)]
fn make_writable_one(path: &Path, meta: &std::fs::Metadata) {
    use std::os::unix::fs::PermissionsExt as _;
    let mut perm = meta.permissions();
    perm.set_mode(perm.mode() | 0o200);
    let _ = std::fs::set_permissions(path, perm);
}

/// Windows 上「只读」是 `FILE_ATTRIBUTE_READONLY`，与 Unix 的 mode 位无关；
/// 带这个属性的文件同样会让删除失败，必须单独清掉。
///
/// `clippy::permissions_set_readonly_false` 担心的是「在 Unix 上这么写会让文件
/// 对所有人可写」—— 这个函数只在 Windows 编译，那条担心不适用，所以显式 allow。
#[cfg(windows)]
#[allow(clippy::permissions_set_readonly_false)]
fn make_writable_one(path: &Path, meta: &std::fs::Metadata) {
    let mut perm = meta.permissions();
    perm.set_readonly(false);
    let _ = std::fs::set_permissions(path, perm);
}

// ══ 导入他人的需求（get）═══════════════════════════════════════════════
//
// 对应旧版 `gws get`。与 `new` 的区别：模块集合不靠人工指定，而是**扫描所有
// 仓库看谁有该远程分支**；每个模块的上游指向 `origin/<分支>`（协作场景下
// IDE 里直接 pull/push 该分支），而 `new` 是解除上游（避免误推主干）。

/// 导入需求的参数。
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct GetOptions {
    pub branch: String,
    /// 中文备注。留空则用分支名（后续可由 doc-hub 的 meta.json 补全，§4.2）。
    pub title: String,
    pub doc_family: Option<String>,
    /// 自由备注 / 自定义信息（导入时就能填，跟新建一致）。
    pub notes: String,
    pub links: Vec<crate::workspace::Link>,
    /// 基线来源链（从其他人那段配置里带过来的）。空 = 主干兜底。
    pub base: Vec<String>,
    /// 只认这些模块。`None` = 扫全部仓库（老行为）。
    ///
    /// 粘贴对方发来的配置时用它：那份配置里已经写明涉及哪些仓库，
    /// 不必再去扫一遍全部（22 个仓库的 fetch 要十几秒），而且**集合以那份为准**。
    pub modules: Option<Vec<String>>,
}


#[derive(Debug, Clone, Serialize)]
pub struct GetOutcome {
    pub branch: String,
    /// 扫描出并成功拉取的模块。
    pub modules: Vec<String>,
    /// 复用了本地已有同名分支的模块。
    pub reused: Vec<String>,
    /// 本地没有该远程分支的仓库（正常——说明这个需求没改动它们）。
    pub not_involved: Vec<String>,
}

/// 报错信息只留第一行 —— git 的 stderr 常常是多行，塞进进度条里没人看。
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

/// 从远程导入他人（或其他电脑）已推送的需求。
///
/// **并发扫描所有仓库**：每个都要 `fetch` 才能确认远程分支是否存在，
/// 串行会卡住几十秒（需求 #8 的卡顿同源）。
pub fn get(hub: &Hub, opts: &GetOptions, cb: crate::progress::Progress<'_>) -> Result<GetOutcome> {
    name::validate_component(&opts.branch, "分支名")?;

    let dir = hub.ws_dir().join(&opts.branch);
    if dir.exists() {
        return Err(Error::WorkspaceExists(opts.branch.clone()));
    }

    prune_all_repos(hub)?;

    // 只探测这次真正关心的模块（配置里点名的那几个），而不是全部仓库。
    // `None` 时退化成老行为：全部扫一遍，靠远程分支反推谁参与了这个需求。
    let scan_list: Vec<String> = match &opts.modules {
        Some(list) => {
            let mut out: Vec<String> = Vec::new();
            let known: Vec<String> = repo::list(hub)?.into_iter().map(|r| r.name).collect();
            for m in list {
                if !known.contains(m) {
                    return Err(Error::other(format!(
                        "配置里写着模块 {m}，但本地 repos/ 下没有它。\n\n\
                         粘贴对方发来的配置时应当先把缺的仓库补上（界面会自动做）；\
                         若你手工调的，先 `gws repo add <地址>`"
                    )));
                }
                if !out.contains(m) {
                    out.push(m.clone());
                }
            }
            out
        }
        None => repo::list(hub)?.into_iter().map(|r| r.name).collect(),
    };

    let total = scan_list.len();
    let mut rep =
        crate::progress::Reporter::start(cb, total, format!("扫描哪些仓库有 {}", opts.branch));
    let branch = opts.branch.clone();
    let scanned = repo::map_parallel(hub, |r| {
        if !scan_list.contains(&r.name) {
            // 不在范围内 → 不探测、也不算「没有该分支」
            return Ok(None);
        }
        let git = r.git();
        let _ = git.fetch();
        Ok(Some(git.remote_branch_exists(&branch)))
    })?;

    let mut involved = Vec::new();
    let mut not_involved = Vec::new();
    for (module, found) in scanned {
        match found {
            // 不在这次的范围内：既不算参与也不算没参与，直接不提
            Ok(None) => continue,
            Ok(Some(true)) => involved.push(module),
            Ok(Some(false)) => not_involved.push(module),
            // 单个仓库 fetch 失败（网络抖动）不该让整个导入失败，但要如实报出来
            Err(e) => not_involved.push(format!("{module}（探测失败: {e}）")),
        }
    }
    involved.sort();
    not_involved.sort();
    for m in &involved {
        rep.ok(format!("{m}（有该分支）"));
    }
    for m in &not_involved {
        rep.ok(format!("{m}（无该分支，跳过）"));
    }
    rep.finish();

    if involved.is_empty() {
        return Err(Error::other(format!(
            "没有任何仓库存在远程分支 {}\n\
             确认分支名是否写错；若刚推送过，可先跑 gws sync 刷新远程引用",
            opts.branch
        )));
    }

    std::fs::create_dir_all(&dir).map_err(|e| Error::io(dir.clone(), e))?;

    // 建 worktree 这一段**也要报进度**：大仓库每建一个都要 checkout 出完整工作树，
    // 五个仓库可能就是几分钟。只有扫描那段报进度的话，界面会停在「5/5」
    // 一动不动 —— 看着像卡死（实测反馈：还以为是在重复扫描旧分支）。
    let mut rep = crate::progress::Reporter::start(
        cb,
        involved.len(),
        format!(
            "拉取 {} 个模块（每个都要检出完整工作树，大仓库慢）",
            involved.len()
        ),
    );

    let mut created = Vec::new();
    let mut reused = Vec::new();
    let mut failure: Option<(String, String)> = None;

    for m in &involved {
        let repo = Repo::open(hub, m)?;
        let git = repo.git();
        let wt = dir.join(m);

        // 先记下是否已有本地分支：决定 worktree add 的形态，也决定它算"新建"还是"复用"
        let had_local = git.local_branch_exists(&opts.branch);
        let added = if had_local {
            git.worktree_add(&wt, &opts.branch, false, None)
        } else {
            let r = git.worktree_add(
                &wt,
                &opts.branch,
                true,
                Some(&format!("origin/{}", opts.branch)),
            );
            if r.is_ok() {
                // 协作场景：上游就该指向 origin/<分支>，方便 IDE 直接 pull/push
                let _ = Git::new(&wt).set_upstream(&opts.branch);
            }
            r
        };

        match added {
            Ok(()) => {
                rep.ok(if had_local {
                    format!("{m}（复用本地分支）")
                } else {
                    m.clone()
                });
                created.push(m.clone());
                if had_local {
                    reused.push(m.clone());
                }
            }
            Err(e) => {
                rep.fail(m.clone(), first_line(&e.to_string()));
                failure = Some((m.clone(), e.to_string()));
                break;
            }
        }
    }

    rep.finish();

    if let Some((module, reason)) = failure {
        rollback(hub, &dir, &opts.branch, &created);
        return Err(Error::other(format!(
            "模块 {module} 拉取失败（已回滚 {} 个已建模块，无残留）: {reason}",
            created.len()
        )));
    }

    let mut meta = WorkspaceMeta::new(
        opts.branch.clone(),
        opts.title.clone(),
        prefix_of(&opts.branch),
        // 基线链从其他人那段配置里带过来（空 = 主干兜底）
        normalise_base_chain(&opts.base)?,
        created.clone(),
    );
    // 导入时填的备注与自定义信息
    meta.notes = opts.notes.clone();
    meta.links = opts.links.clone();
    if let Some(family) = &opts.doc_family {
        meta.doc = Some(crate::workspace::DocRef::new(
            family.clone(),
            opts.branch.clone(),
        ));
    }
    meta.save(&dir.join(META_FILE))?;

    let _ws = Workspace::from_dir(&dir)?;

    Ok(GetOutcome {
        branch: opts.branch.clone(),
        modules: created,
        reused,
        not_involved,
    })
}

/// 从分支名里取前缀（`feature-20261010-x` → `feature`），取不到就用 `feature`。
fn prefix_of(branch: &str) -> String {
    branch
        .split('-')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("feature")
        .to_string()
}

// ══ 清理残留（clean）═══════════════════════════════════════════════════
//
// 对应需求 #6 的另一半：旧版 `rm` 失败时静默报成功，作者的 hub 里因此攒下了
// 没有元数据的残留目录。`clean` 把它们找出来。

#[derive(Debug, Clone, Default, Serialize)]
pub struct CleanOutcome {
    /// `ws/` 下没有 `.workspace.json` 的残留目录。
    pub leftovers: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
}

/// 扫描（并可选删除）`ws/` 下的残留目录。
///
/// `apply=false` 时只列出，不动任何东西——清理类操作默认必须是预览。
pub fn clean(hub: &Hub, apply: bool) -> Result<CleanOutcome> {
    let mut out = CleanOutcome::default();
    let dir = hub.ws_dir();
    if !dir.is_dir() {
        return Ok(out);
    }

    let entries = std::fs::read_dir(&dir).map_err(|e| Error::io(dir.clone(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir.clone(), e))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // 有元数据 = 正常的工作区，跳过
        if path.join(META_FILE).is_file() {
            continue;
        }
        out.leftovers.push(path.clone());
        if apply {
            match remove_dir_verified(&path) {
                Ok(()) => out.removed.push(path),
                Err(e) => out.failed.push((path, e.to_string())),
            }
        }
    }

    out.leftovers.sort();
    Ok(out)
}

// ══ 模块目录 ═══════════════════════════════════════════════════════════

// 需求目录下**不再**生成 `CLAUDE.md`：约束与说明统一放在 **hub 根目录**那一份
// （`Hub::write_claude_md`）。理由：那些内容（目录语义、改代码规则、提交约定）
// 对**每个需求都一样**，逐份生成只是把同一段话复制 N 遍；而 hub 那份还能随版本
// 更新，不必逐个需求去刷。

/// 工作区目录里所有模块的绝对路径（供批量 git 操作）。
///
/// 用 `op_candidates`（**含文档库**）而不是 `present_modules()`：文档库在工作区里
/// 就是一块真实的 worktree，凡是「对这一堆目录跑 git」的地方都该带上它。
/// 文档库漏在 `present_modules()` 之外这个坑，这个仓库里已经踩过四次
/// （推送/合并 → MR → 检查远程 → 撤销改动），这里先把下一次堵掉。
pub fn module_dirs(hub: &Hub, ws: &Workspace) -> Vec<(String, PathBuf)> {
    crate::workspace::sync::op_candidates(ws, &hub.dochub_name())
        .into_iter()
        .map(|m| (m.clone(), ws.module_path(&m)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precheck_display_lists_every_category() {
        let fail = PrecheckFailure {
            missing_repos: vec!["ghost".into()],
            remote_branch_exists: vec!["a".into()],
            local_branch_exists: vec!["b".into()],
            base_missing: vec![("c".into(), "origin/nope".into())],
        };
        let msg = fail.to_string();
        assert!(msg.contains("ghost"), "{msg}");
        assert!(msg.contains("gws get"), "{msg}");
        assert!(msg.contains("origin/nope"), "{msg}");
        assert!(!fail.is_empty());
        assert!(PrecheckFailure::default().is_empty());
    }

    #[test]
    fn base_resolution_note_only_when_degraded() {
        let hit = BaseResolution {
            reference: "origin/x".into(),
            missed: vec![],
            fell_back_to_main: false,
        };
        assert!(hit.note().is_none());

        let partial = BaseResolution {
            reference: "origin/x".into(),
            missed: vec!["阶段1".into()],
            fell_back_to_main: false,
        };
        assert!(partial.note().unwrap().contains("阶段1"));

        let fell = BaseResolution {
            reference: "origin/main".into(),
            missed: vec!["阶段1".into()],
            fell_back_to_main: true,
        };
        assert!(fell.note().unwrap().contains("降级主干"));
    }

    #[test]
    fn remove_dir_verified_reports_success_and_is_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("gone");
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested/f.txt"), "x").unwrap();

        remove_dir_verified(&dir).unwrap();
        assert!(!dir.exists());
        // 已经不存在时再删一次不算错
        remove_dir_verified(&dir).unwrap();
    }

    #[test]
    fn remove_dir_verified_clears_readonly_files() {
        // 只读文件会让 rm -rf 失败——旧版正是在这里静默失败还报成功
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("ro");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("readonly.txt");
        std::fs::write(&f, "x").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o444)).unwrap();
            // 目录本身不可写会让 remove_dir_all 连条目都删不掉
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        }

        remove_dir_verified(&dir).unwrap();
        assert!(!dir.exists(), "只读文件应被清理掉");
    }
}
#[cfg(test)]
mod base_chain_tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn trunk_keywords_are_not_stored() {
        // 实测反馈 #4：清空基线时以前会写入字面量「主干」，界面就把它当成一个
        // 真填过的值显示出来。语义上它就是空的，不该存。
        for kw in ["主干", "main", "master", " 主干 "] {
            assert_eq!(
                normalise_base_chain(&v(&[kw])).unwrap(),
                Vec::<String>::new(),
                "{kw}"
            );
            assert!(sanitise_base_chain(&v(&[kw])).is_empty(), "{kw}");
        }
    }

    #[test]
    fn duplicates_are_rejected_on_write_but_deduped_on_read() {
        let dup = v(&["feature-a", "feature-a"]);
        let err = normalise_base_chain(&dup).unwrap_err().to_string();
        assert!(err.contains("重复"), "{err}");
        assert!(err.contains("feature-a"), "报错要点名是哪一项：{err}");

        // 读路径不报错 —— 一个字段不规范不该让整个需求打不开
        assert_eq!(sanitise_base_chain(&dup), v(&["feature-a"]));
    }

    #[test]
    fn illegal_names_are_rejected_with_a_readable_reason() {
        // 用户点名的例子：中间有空格
        let err = normalise_base_chain(&v(&["feature a"]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("空格"), "{err}");
        assert!(err.contains("feature a"), "要把出错的原文带出来：{err}");

        for bad in [
            "a..b", "a~b", "a^b", "a:b", "a?b", "a*b", "a[b", "a\\b", "-lead", "trail.", "x.lock",
        ] {
            assert!(normalise_base_chain(&v(&[bad])).is_err(), "{bad} 应被拒");
        }
    }

    #[test]
    fn ws_prefix_and_origin_prefix_are_accepted() {
        assert_eq!(
            normalise_base_chain(&v(&["ws:feature-upstream"])).unwrap(),
            v(&["ws:feature-upstream"])
        );
        assert_eq!(
            normalise_base_chain(&v(&["origin/feature-x"])).unwrap(),
            v(&["origin/feature-x"])
        );
        // 「ws:」后面什么都没写要说清楚
        let err = normalise_base_chain(&v(&["ws:"])).unwrap_err().to_string();
        assert!(err.contains("ws:"), "{err}");
    }

    #[test]
    fn blank_entries_and_order_are_handled() {
        // 留空即主干；多余的空项丢掉，**顺序保持不变**（顺序就是优先级）
        assert_eq!(
            normalise_base_chain(&v(&["", "  ", "b-base", "主干", "a-base"])).unwrap(),
            v(&["b-base", "a-base"])
        );
    }
}
