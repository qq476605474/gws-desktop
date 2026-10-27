//! 提 Merge Request 到主干（需求 #1）。
//!
//! # 一次提一批，且自动处理三件最容易错的事
//!
//! 一个需求横跨多个仓库，逐个手工建 MR 既慢又容易漏。这里批量创建，并自动处理：
//!
//! 1. **各仓库主干名不统一**——`main` / `master` 混用，逐个探测，绝不写死
//! 2. **前后端评审人不同**——按 group 与模块名自动分配
//! 3. **已合并 / 未推送 / 无改动 的模块要跳过**——且跳过原因要能区分清楚
//!
//! # 幂等
//!
//! 同源分支已有打开的 MR 时**复用而不重复建**。所以这个操作可以安全地重跑，
//! 不用先手工去 GitLab 上确认。
//!
//! # 撤销的语义
//!
//! GitLab 的 MR **不能删除，只能关闭**。`close` 就是「撤销」，且**不动分支**
//! （分支清理是独立功能，见需求 #11）。

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::gitlab::{Client, CreateMrRequest, MergeRequest, User};
use crate::hub::Hub;
use crate::repo::Repo;
use crate::workspace::{sync, ModuleFilter, Workspace};
use crate::{Error, Result};

/// 模块属于前端还是后端。评审人按这个分配。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Side {
    Backend,
    Frontend,
}

impl Side {
    pub fn label(&self) -> &'static str {
        match self {
            Side::Backend => "后端",
            Side::Frontend => "前端",
        }
    }

    /// 判定规则：GitLab group 在**本 hub 配置的** `defaults.frontend_groups` 里，
    /// 或模块名以 `-front` 结尾 → 前端；其余全部算后端。
    ///
    /// ⚠️ 不要往这里加写死的 group 名。原来写死过一个具体值（那是定制版的真实
    /// group），对别人的实例永远不成立 —— 而判错的后果很隐蔽：该侧评审人
    /// 「没被用到」→ 直接被跳过，MR 上就是**少挂一个评审人**，没有任何报错。
    /// 需要按 group 判的话，让用户写进 `gws.json` 的 `defaults.frontend_groups`。
    pub fn of(module: &str, group: &str, frontend_groups: &[String]) -> Side {
        if frontend_groups.iter().any(|g| g == group) || module.ends_with("-front") {
            Side::Frontend
        } else {
            Side::Backend
        }
    }
}

// ══ 评审人 ═════════════════════════════════════════════════════════════

/// 命令行传入的评审人覆盖项（优先级最高）。
#[derive(Debug, Clone, Default)]
pub struct ReviewerOverride {
    /// 所有模块统一指定，不再区分前后端。
    pub all: Option<String>,
    pub frontend: Option<String>,
    pub backend: Option<String>,
}

/// 解析后的评审人配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reviewers {
    pub frontend: String,
    pub backend: String,
    /// 实际生效的来源链，用于展示（如「默认 → 全局配置 → --backend-reviewer」）。
    pub source: String,
}

impl Reviewers {
    pub fn for_side(&self, side: Side) -> &str {
        match side {
            Side::Frontend => &self.frontend,
            Side::Backend => &self.backend,
        }
    }
}

/// 两层配置文件都支持的写法：`{"frontend":..,"backend":..}` 或 `{"reviewers":{..}}`。
#[derive(Debug, Deserialize)]
struct ReviewerFile {
    #[serde(default)]
    frontend: Option<String>,
    #[serde(default)]
    backend: Option<String>,
    #[serde(default)]
    reviewers: Option<Inner>,
}

#[derive(Debug, Deserialize)]
struct Inner {
    #[serde(default)]
    frontend: Option<String>,
    #[serde(default)]
    backend: Option<String>,
}

impl ReviewerFile {
    fn pick(&self) -> (Option<String>, Option<String>) {
        match &self.reviewers {
            Some(inner) => (
                inner.frontend.clone().or_else(|| self.frontend.clone()),
                inner.backend.clone().or_else(|| self.backend.clone()),
            ),
            None => (self.frontend.clone(), self.backend.clone()),
        }
    }
}

/// 按优先级解析评审人（设计方案 §5.3）。
///
/// ```text
/// 命令行 --reviewer / --frontend-reviewer / --backend-reviewer
///   > 需求级 <工作区>/.gws-mr.json
///   > 全局 ~/.config/gws/reviewers.json
/// ```
///
/// **没有内置默认评审人**：三层都没配就是「本次不提评审人」，创建 MR 时不带
/// 评审人字段。开源版不预设任何人 —— 谁评审由用的人自己指定，或写进上面两层
/// 配置文件。
///
/// **配置文件损坏只告警并降级到上一层**，不影响提 MR——一个写错的 JSON
/// 不该让人连 MR 都提不了。但命令行指定的 username **非法时直接报错中止**：
/// 那是本次操作的显式意图，静默忽略会让人以为生效了。
pub fn resolve_reviewers(ws: Option<&Workspace>, over: &ReviewerOverride) -> Reviewers {
    let mut frontend = String::new();
    let mut backend = String::new();
    let mut chain: Vec<String> = Vec::new();

    // 全局配置 → 工作区配置，后者覆盖前者
    let mut candidates: Vec<(&str, std::path::PathBuf)> = Vec::new();
    if let Some(p) = crate::paths::reviewers_config() {
        candidates.push(("全局配置", p));
    }
    if let Some(w) = ws {
        candidates.push(("工作区配置", w.dir().join(".gws-mr.json")));
    }

    for (label, path) in candidates {
        let Some((f, b)) = read_reviewer_file(&path) else {
            continue;
        };
        let mut applied = false;
        if let Some(f) = f {
            frontend = f;
            applied = true;
        }
        if let Some(b) = b {
            backend = b;
            applied = true;
        }
        if applied {
            chain.push(format!("{label}({})", path.display()));
        }
    }

    // 命令行优先级最高
    if let Some(all) = over.all.clone().filter(|s| !s.trim().is_empty()) {
        frontend = all.clone();
        backend = all;
        chain.push("--reviewer".into());
    } else {
        if let Some(f) = over.frontend.clone().filter(|s| !s.trim().is_empty()) {
            frontend = f;
            chain.push("--frontend-reviewer".into());
        }
        if let Some(b) = over.backend.clone().filter(|s| !s.trim().is_empty()) {
            backend = b;
            chain.push("--backend-reviewer".into());
        }
    }

    Reviewers {
        frontend,
        backend,
        source: if chain.is_empty() {
            "未指定".to_string()
        } else {
            chain.join(" → ")
        },
    }
}

/// 读一个评审人配置文件。解析失败只告警并返回 `None`（降级到上一层）。
fn read_reviewer_file(path: &Path) -> Option<(Option<String>, Option<String>)> {
    if !path.is_file() {
        return None;
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("⚠ 评审人配置 {} 读取失败，已忽略: {e}", path.display());
            return None;
        }
    };
    match serde_json::from_str::<ReviewerFile>(&text) {
        Ok(f) => Some(f.pick()),
        Err(e) => {
            eprintln!("⚠ 评审人配置 {} 解析失败，已忽略: {e}", path.display());
            None
        }
    }
}

// ══ 计划与执行 ═════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct MrOptions {
    /// MR 标题。**原样提交**（不再二次加工）。
    /// 空/缺省时用 [`default_title`]。
    pub title: Option<String>,
    /// MR 描述（CLI 读好文件后传入）。空/缺省时用 [`default_description`]。
    pub description: Option<String>,
    pub filter: ModuleFilter,
    pub reviewers: ReviewerOverride,
}

impl Default for MrOptions {
    fn default() -> Self {
        Self {
            title: None,
            description: None,
            filter: ModuleFilter::all(),
            reviewers: ReviewerOverride::default(),
        }
    }
}

/// 界面上预填的标题与描述（不联网）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MrDefaults {
    pub title: String,
    pub description: String,
}

/// 默认 MR 标题：`需求上线-<需求名>`。
///
/// 实测反馈：`feat(...)` 前缀在评审列表里不直观，所以约定用「需求上线-」开头。
pub fn default_title(ws: &Workspace) -> String {
    let name = match ws.title().trim() {
        "" => "需求代码合并",
        n => n,
    };
    format!("需求上线-{name}")
}

/// 默认 MR 描述。`title` 传最终标题，描述里跟着它走（与旧版行为一致）。
pub fn default_description(ws: &Workspace, title: &str) -> String {
    format!(
        "## 需求\n\n{title}\n\n## 说明\n\n- 源分支：`{}`\n\
         - 本需求横跨多个仓库，需**同批次合并**\n",
        ws.branch()
    )
}

/// 弹窗打开时用来填两个输入框的默认值。
pub fn defaults(ws: &Workspace) -> MrDefaults {
    let title = default_title(ws);
    let description = default_description(ws, &title);
    MrDefaults { title, description }
}

fn pick_title(ws: &Workspace, raw: Option<&str>) -> String {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_title(ws))
}

fn pick_description(ws: &Workspace, title: &str, raw: Option<&str>) -> String {
    raw.filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_description(ws, title))
}

/// 一个模块的 MR 情况。
#[derive(Debug, Clone, Serialize)]
pub struct MrRow {
    pub module: String,
    /// GitLab 项目路径（`group/name`）。
    pub project: String,
    pub group: String,
    pub side: Side,
    pub target_branch: String,
    /// 相对主干的改动文件数。
    pub files: u64,
    pub reviewer: String,
    /// 评审人的显示名（通常是中文名），用于清单末尾的 @提醒。
    pub reviewer_name: String,
    pub reviewer_id: u64,
    /// 已存在的打开 MR（复用，不重复建）。
    pub existing: Option<MergeRequest>,
    /// 本次创建的 MR。
    pub created: Option<MergeRequest>,
    pub error: Option<String>,
}

impl MrRow {
    /// 该行最终可用的 MR 链接（新建的或复用的）。
    pub fn url(&self) -> Option<&str> {
        self.created
            .as_ref()
            .or(self.existing.as_ref())
            .map(|m| m.web_url.as_str())
            .filter(|u| !u.is_empty())
    }

    pub fn iid(&self) -> Option<u64> {
        self.created
            .as_ref()
            .or(self.existing.as_ref())
            .map(|m| m.iid)
    }

    pub fn reused(&self) -> bool {
        self.created.is_none() && self.existing.is_some()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MrPlan {
    pub branch: String,
    /// 实际提交的最终标题。
    pub title: String,
    /// 实际提交的最终描述。
    pub description: String,
    pub reviewers: Reviewers,
    pub rows: Vec<MrRow>,
    /// `(模块, 跳过原因)`。原因要能区分「已合并」「未推送」「无改动」。
    pub skipped: Vec<(String, String)>,
    /// 本工作区登记的、不在本次范围内的模块（元数据里的 `mr` 字段）。
    pub registered: BTreeMap<String, u64>,
    pub is_dry_run: bool,
}

impl MrPlan {
    pub fn created_count(&self) -> usize {
        self.rows.iter().filter(|r| r.created.is_some()).count()
    }

    pub fn reused_count(&self) -> usize {
        self.rows.iter().filter(|r| r.reused()).count()
    }

    pub fn failed(&self) -> impl Iterator<Item = &MrRow> {
        self.rows.iter().filter(|r| r.error.is_some())
    }

    /// 需要 @ 的评审人**显示名**（去重，按后端 → 前端顺序）。
    ///
    /// 用显示名而不是 username：清单是要复制到群聊里提醒人的，
    /// `@zhangsan` 通常不会触发提醒，`@张三` 才会。
    pub fn touched_reviewer_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for side in [Side::Backend, Side::Frontend] {
            for r in self
                .rows
                .iter()
                .filter(|r| r.side == side && r.url().is_some())
            {
                if !r.reviewer_name.is_empty() && !out.contains(&r.reviewer_name) {
                    out.push(r.reviewer_name.clone());
                }
            }
        }
        out
    }
}

/// 提 MR。`create=false` 时只做计划（dry-run，不写任何东西）。
pub fn run(
    hub: &Hub,
    ws: &mut Workspace,
    client: &Client,
    opts: &MrOptions,
    create: bool,
    cb: crate::progress::Progress<'_>,
) -> Result<MrPlan> {
    let branch = ws.branch().to_string();
    let modules = selected_modules(ws, &opts.filter, &hub.dochub_name())?;
    let mut rep = crate::progress::Reporter::start(
        cb,
        modules.len(),
        if create {
            "创建 MR"
        } else {
            "探测各模块的 MR 情况"
        },
    );
    let reviewers = resolve_reviewers(Some(ws), &opts.reviewers);

    // 评审人 username → id/显示名。指定的用户在 GitLab 上不存在时直接报错，
    // 绝不静默换人——用错误的评审人比提不出 MR 更糟。
    let mut resolved: BTreeMap<Side, (u64, User)> = BTreeMap::new();
    for side in [Side::Backend, Side::Frontend] {
        let username = reviewers.for_side(side);
        // **没配评审人就跳过这一侧。**
        //
        // 开源版不预设评审人（见 `resolve_reviewers`），三层都没配时 username 是
        // 空串 —— 拿空串去查用户会得到「GitLab 上不存在用户: 」这种一头雾水的错，
        // 而正确行为是「这次不指定评审人」。
        if username.trim().is_empty() {
            continue;
        }
        // 只有真的会用到该侧评审人才解析，避免配置里写了个用不上的坏值时误报错
        let groups = &hub.config().defaults.frontend_groups;
        let used = modules.iter().any(|m| {
            project_path(hub, m)
                .map(|(_, g)| Side::of(m, &g, groups) == side)
                .unwrap_or(false)
        });
        if !used {
            continue;
        }
        let user = client.user_by_username(username)?;
        resolved.insert(side, (user.id, user));
    }

    let title = pick_title(ws, opts.title.as_deref());
    let description = pick_description(ws, &title, opts.description.as_deref());

    let mut plan = MrPlan {
        branch: branch.clone(),
        title: title.clone(),
        description: description.clone(),
        reviewers,
        rows: Vec::new(),
        skipped: Vec::new(),
        registered: BTreeMap::new(),
        is_dry_run: !create,
    };

    for module in &modules {
        let Some((project, group)) = project_path(hub, module) else {
            rep.fail(module.clone(), "无 origin 远程");
            plan.skipped.push((module.clone(), "无 origin 远程".into()));
            continue;
        };
        let wt = ws.module_path(module);
        let git = crate::git::Git::new(&wt);
        let repo = Repo::open(hub, module)?;

        // ① 必须已推送：MR 基于远程分支，本地提交没推上去，MR 内容就不全
        if !git.remote_branch_exists(&branch) {
            rep.fail(module.clone(), "分支没推送到远程");
            plan.skipped.push((
                module.clone(),
                format!("origin/{branch} 不存在，先 gws push"),
            ));
            continue;
        }
        let ahead = git.count_range(&format!("origin/{branch}..HEAD"))?;
        if ahead != 0 {
            rep.fail(module.clone(), format!("{ahead} 个提交未推送"));
            plan.skipped.push((
                module.clone(),
                format!("有 {ahead} 个提交未推送，先 gws push"),
            ));
            continue;
        }

        let target = repo.main_branch()?;
        let _ = git.fetch();

        // ② 已并入主干则无需再提。要能和「本来就没改动」区分开——
        //    两者的下一步动作完全不同（一个收工，一个可能忘了写代码）
        if git.is_ancestor("HEAD", &format!("origin/{target}")) {
            rep.ok(format!("{module}（已合并入 {target}，跳过）"));
            plan.skipped
                .push((module.clone(), format!("已合并入 {target}，无需提 MR")));
            continue;
        }

        let stat = git.shortstat(&format!("origin/{target}...HEAD"))?;
        if stat.trim().is_empty() {
            rep.ok(format!("{module}（无改动，跳过）"));
            plan.skipped
                .push((module.clone(), format!("相对 {target} 无文件改动")));
            continue;
        }

        let side = Side::of(module, &group, &hub.config().defaults.frontend_groups);
        // 评审人**可选**：三层都没配就是「这次不指定评审人」（开源版没有内置默认人）。
        // 用 id=0 / 空串占位，创建 MR 时 `reviewer_ids` 会是空数组 →
        // 请求体里不带该字段（见 `CreateMrRequest::reviewer_ids` 的 skip 条件）。
        let (reviewer_id, reviewer_user) = resolved.get(&side).cloned().unwrap_or((
            0,
            User { id: 0, username: String::new(), name: String::new() },
        ));

        // ③ 幂等：已有打开的 MR 就复用
        let existing = client.open_mr(&project, &branch)?;

        let mut row = MrRow {
            module: module.clone(),
            project: project.clone(),
            group,
            side,
            target_branch: target,
            files: parse_file_count(&stat),
            reviewer: reviewer_user.username.clone(),
            reviewer_name: reviewer_user.name.clone(),
            reviewer_id,
            existing,
            created: None,
            error: None,
        };

        if create && row.existing.is_none() {
            let req = CreateMrRequest {
                source_branch: branch.clone(),
                target_branch: row.target_branch.clone(),
                // 标题已由界面/core 定稿，这里**原样提交**，不再二次加工。
                title: title.clone(),
                description: description.clone(),
                // 没配评审人时为空 → 请求体里不带 reviewer_ids（不瞎填一个人）
                reviewer_ids: if reviewer_id == 0 { Vec::new() } else { vec![reviewer_id] },
                squash: false,
                // 不自动删源分支：主干合并后可能还要 hotfix，且删分支是独立决策
                remove_source_branch: false,
            };
            match client.create_mr(&project, &req) {
                Ok(mr) => {
                    rep.ok(format!("{module} → {}", mr.web_url));
                    row.created = Some(mr);
                }
                Err(e) => {
                    rep.fail(module.clone(), e.to_string());
                    row.error = Some(e.to_string());
                }
            }
        } else if let Some(mr) = &row.existing {
            rep.ok(format!("{module}（已有 MR #{}, 复用）", mr.iid));
        } else {
            rep.ok(format!("{module} 待创建"));
        }

        plan.rows.push(row);
    }
    rep.finish();

    // 把成功的 MR 登记进工作区元数据（需求 #1：以后能查到、能关掉）
    if create {
        for row in &plan.rows {
            if let (Some(iid), Some(url)) = (row.iid(), row.url()) {
                let state = row
                    .created
                    .as_ref()
                    .or(row.existing.as_ref())
                    .map(|m| m.state.clone())
                    .unwrap_or_else(|| "opened".into());
                ws.meta_mut().mr.insert(
                    row.module.clone(),
                    crate::workspace::MrRecord {
                        iid,
                        url: url.to_string(),
                        state,
                    },
                );
            }
        }
        ws.save_meta()?;
    }
    plan.registered = ws
        .meta()
        .mr
        .iter()
        .map(|(k, v)| (k.clone(), v.iid))
        .collect();

    Ok(plan)
}

/// 关闭本需求的 MR（需求 #1 的「撤销」）。
///
/// **只关闭，不碰分支。** 分支清理是需求 #11 的独立功能。
pub fn close(
    hub: &Hub,
    ws: &mut Workspace,
    client: &Client,
    filter: &ModuleFilter,
) -> Result<CloseOutcome> {
    let modules = selected_modules(ws, filter, &hub.dochub_name())?;
    let mut out = CloseOutcome::default();

    for module in &modules {
        let Some(record) = ws.meta().mr.get(module).cloned() else {
            out.skipped.push((module.clone(), "未登记过 MR".into()));
            continue;
        };
        let Some((project, _)) = project_path(hub, module) else {
            out.skipped.push((module.clone(), "无 origin 远程".into()));
            continue;
        };
        match client.close_mr(&project, record.iid) {
            Ok(mr) => {
                let mut rec = record.clone();
                rec.state = mr.state.clone();
                ws.meta_mut().mr.insert(module.clone(), rec);
                out.closed.push((module.clone(), mr.web_url));
            }
            Err(e) => out.failed.push((module.clone(), e.to_string())),
        }
    }

    if !out.closed.is_empty() {
        ws.save_meta()?;
    }
    Ok(out)
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CloseOutcome {
    /// `(模块, MR 链接)`。
    pub closed: Vec<(String, String)>,
    pub skipped: Vec<(String, String)>,
    pub failed: Vec<(String, String)>,
}

// ══ 辅助 ═══════════════════════════════════════════════════════════════

/// 模块 → `(GitLab 项目路径 group/name, group)`。
fn project_path(hub: &Hub, module: &str) -> Option<(String, String)> {
    let repo = Repo::open(hub, module).ok()?;
    let url = repo.remote_url()?;
    let path = path_from_remote_url(&url)?;
    let group = path.split('/').next().unwrap_or("").to_string();
    Some((path, group))
}

/// 从 git remote URL 解析 GitLab 项目路径。
///
/// 支持：`https://host/group/name.git`、`git@host:group/name.git`、
/// 以及多级 group（`host/a/b/name.git` → `a/b/name`）。
pub fn path_from_remote_url(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let raw = if let Some(rest) = trimmed.split_once('@').map(|(_, r)| r) {
        // git@host:group/name.git
        rest.split_once(':')
            .map(|(_, p)| p.to_string())
            .unwrap_or_default()
    } else if let Some(rest) = trimmed.split_once("://").map(|(_, r)| r) {
        // https://host/group/name.git
        rest.split_once('/')
            .map(|(_, p)| p.to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    let path = raw
        .trim_matches('/')
        .trim_end_matches(".git")
        .trim_matches('/');
    if path.is_empty() || !path.contains('/') {
        None
    } else {
        Some(path.to_string())
    }
}

/// 从 `git diff --shortstat` 输出里取文件数。
fn parse_file_count(stat: &str) -> u64 {
    stat.split(',')
        .find(|part| part.contains("file"))
        .and_then(|part| part.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// 本次操作会涉及哪些模块（提 MR 与关 MR 共用）。
///
/// # 用 `op_candidates`，不是 `present_modules`
///
/// **文档库要算进来。** 它不在 `meta.modules` 里（由 `meta.doc` 单独登记），
/// 所以拿 `present_modules()` 去过滤会把它**静默丢掉**：界面里勾了 doc-hub，
/// 到了这里一个 MR 都不提，还**不报错**——因为保留名不算「未匹配的模块」
/// （见 `ModuleFilter::unmatched_only`）。
///
/// 实测反馈：提 MR 时选不了 doc-hub，于是少提了一个。
/// 模块级操作（推送 / 合并…）那边早就改用 `op_candidates` 了，**MR 这条路漏了**。
pub fn selected_modules(
    ws: &Workspace,
    filter: &ModuleFilter,
    dochub_name: &str,
) -> Result<Vec<String>> {
    let unmatched = filter.unmatched_only(&ws.meta().modules, dochub_name);
    if !unmatched.is_empty() {
        return Err(Error::other(format!(
            "指定的模块不在本工作区: {}",
            unmatched.join("、")
        )));
    }
    Ok(filter
        .resolve(&sync::op_candidates(ws, dochub_name))
        .into_iter()
        .cloned()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{WorkspaceMeta, META_FILE};

    fn workspace(title: &str) -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::TempDir::new().unwrap();
        let meta = WorkspaceMeta::new(
            "feature-20261021-mr",
            title,
            "feature",
            Vec::new(),
            Vec::new(),
        );
        meta.save(&dir.path().join(META_FILE)).unwrap();
        let ws = Workspace::from_dir(dir.path()).unwrap();
        (dir, ws)
    }

    #[test]
    fn mr_defaults_use_the_release_title() {
        let (_tmp, ws) = workspace("会员中心改版");
        let d = defaults(&ws);
        assert_eq!(d.title, "需求上线-会员中心改版");
        assert!(
            d.description.contains("需求上线-会员中心改版"),
            "{}",
            d.description
        );
        assert!(d.description.contains("feature-20261021-mr"));
    }

    #[test]
    fn custom_title_is_used_verbatim() {
        let (_tmp, ws) = workspace("会员中心改版");
        assert_eq!(pick_title(&ws, Some("fix: 紧急修一下")), "fix: 紧急修一下");
        // 空/空白回退默认，而不是提交一个空标题
        assert_eq!(pick_title(&ws, Some("   ")), default_title(&ws));
        assert_eq!(pick_title(&ws, None), "需求上线-会员中心改版");
    }

    #[test]
    fn blank_description_falls_back_to_default() {
        let (_tmp, ws) = workspace("会员中心改版");
        let title = default_title(&ws);
        assert!(pick_description(&ws, &title, Some("  ")).contains("## 需求"));
        assert_eq!(
            pick_description(&ws, &title, Some("我自己写的")),
            "我自己写的"
        );
    }

    #[test]
    fn side_follows_configured_groups_and_suffix_rule() {
        // 默认（没配 group）= 只看 `-front` 后缀
        let none: Vec<String> = Vec::new();
        assert_eq!(Side::of("some-front", "any-group", &none), Side::Frontend);
        assert_eq!(Side::of("demo-api", "demo-team", &none), Side::Backend);

        // group 由 hub 配置给：配了才认，没配的 group 一律算后端
        let groups = vec!["web".to_string(), "h5".to_string()];
        assert_eq!(Side::of("h5-app", "web", &groups), Side::Frontend);
        assert_eq!(Side::of("anything", "h5", &groups), Side::Frontend);
        assert_eq!(Side::of("demo-api", "demo-team", &groups), Side::Backend);
        assert_eq!(Side::of("demo-payment", "demo-internal", &groups), Side::Backend);
    }

    #[test]
    fn parses_project_path_from_both_url_forms() {
        assert_eq!(
            path_from_remote_url("https://gitlab.example.com/demo-api-team/demo-api.git").unwrap(),
            "demo-api-team/demo-api"
        );
        assert_eq!(
            path_from_remote_url("git@gitlab.example.com:demo-api-team/demo-api.git").unwrap(),
            "demo-api-team/demo-api"
        );
        // 多级 group
        assert_eq!(
            path_from_remote_url("https://host/a/b/c.git").unwrap(),
            "a/b/c"
        );
        // 没有 .git 后缀
        assert_eq!(path_from_remote_url("https://host/g/n").unwrap(), "g/n");
        // 尾部斜杠
        assert_eq!(
            path_from_remote_url("https://host/g/n.git/").unwrap(),
            "g/n"
        );
    }

    #[test]
    fn rejects_urls_without_a_group() {
        assert!(path_from_remote_url("https://host/only-name.git").is_none());
        assert!(path_from_remote_url("").is_none());
        assert!(path_from_remote_url("not a url").is_none());
    }

    #[test]
    fn parses_file_count_from_shortstat() {
        assert_eq!(
            parse_file_count(" 5 files changed, 120 insertions(+), 3 deletions(-)"),
            5
        );
        assert_eq!(parse_file_count(" 1 file changed, 2 insertions(+)"), 1);
        assert_eq!(parse_file_count(""), 0);
        assert_eq!(parse_file_count("垃圾内容"), 0);
    }

    #[test]
    fn reviewer_file_supports_both_shapes() {
        let flat: ReviewerFile =
            serde_json::from_str(r#"{"frontend":"a.cai","backend":"b.liu"}"#).unwrap();
        assert_eq!(flat.pick(), (Some("a.cai".into()), Some("b.liu".into())));

        let nested: ReviewerFile =
            serde_json::from_str(r#"{"reviewers":{"frontend":"c.cai","backend":"d.liu"}}"#)
                .unwrap();
        assert_eq!(nested.pick(), (Some("c.cai".into()), Some("d.liu".into())));

        // 只写一个 key 时另一个保持未指定（由上层决定是否覆盖）
        let partial: ReviewerFile = serde_json::from_str(r#"{"backend":"only.liu"}"#).unwrap();
        assert_eq!(partial.pick(), (None, Some("only.liu".into())));

        // 嵌套写法里缺的 key 可以回退到顶层
        let mixed: ReviewerFile =
            serde_json::from_str(r#"{"frontend":"top.cai","reviewers":{"backend":"in.liu"}}"#)
                .unwrap();
        assert_eq!(
            mixed.pick(),
            (Some("top.cai".into()), Some("in.liu".into()))
        );
    }

    #[test]
    fn reviewer_priority_prefers_command_line() {
        // 开源版没有内置默认评审人：什么都不配就是「未指定」
        let r = resolve_reviewers(None, &ReviewerOverride::default());
        assert_eq!(r.frontend, "");
        assert_eq!(r.backend, "");
        assert_eq!(r.source, "未指定");

        // --reviewer 统一指定：两侧都变，且不再区分前后端
        let over = ReviewerOverride {
            all: Some("someone".into()),
            ..Default::default()
        };
        let r = resolve_reviewers(None, &over);
        assert_eq!(r.frontend, "someone");
        assert_eq!(r.backend, "someone");
        assert!(r.source.contains("--reviewer"), "{}", r.source);

        // 只改一侧：另一侧仍是未指定，不会被凭空填上一个默认人
        let over = ReviewerOverride {
            backend: Some("b.liu".into()),
            ..Default::default()
        };
        let r = resolve_reviewers(None, &over);
        assert_eq!(r.frontend, "", "未指定的一侧保持未指定");
        assert_eq!(r.backend, "b.liu");

        // 空白视为未指定
        let over = ReviewerOverride {
            all: Some("   ".into()),
            ..Default::default()
        };
        let r = resolve_reviewers(None, &over);
        assert_eq!(r.backend, "", "空白不该被当成评审人");
        assert_eq!(r.source, "未指定");
    }
}