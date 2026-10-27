//! 远程分支维护：批量清理（需求 #11）与批量改名（需求 #12）。
//!
//! # 为什么这两个必须走 GitLab API
//!
//! 删/建远程分支是**服务端操作**，本地 git 做不到（`git push origin --delete`
//! 虽然能删，但「先查这个分支该不该删」要靠 API 才拿得准）。
//!
//! # 改分支名为什么最麻烦
//!
//! 一个需求横跨多个仓库，每个仓库都有一份同名分支。改名要同时处理：
//!
//! 1. 本地分支改名（每个模块）
//! 2. 工作区**目录改名**——目录名就是分支名（需求 #5）
//! 3. 推新分支、删旧分支（远程）
//! 4. 更新 `.workspace.json` 的 `branch` / `docs`
//! 5. **其他人需要重拉**：本地还是旧目录名，得删掉重拉
//!
//! 第 5 条是设计上的取舍：不做自动同步，而是明确告知——因为「悄悄改掉别人
//! 的工作区」比「让人重拉一次」危险得多。

use serde::Serialize;

use crate::dochub::{meta::render_claude_md, PeriodMeta};
use crate::git::Git;
use crate::gitlab::Client;
use crate::hub::Hub;
use crate::repo::Repo;
use crate::workspace::mr::path_from_remote_url;
use crate::workspace::{DocRef, ModuleFilter, Workspace};
use crate::{Error, Result};

/// 一个模块的远程分支状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchState {
    pub module: String,
    /// GitLab 项目路径（`group/name`）。
    pub project: String,
    pub remote_branch: String,
    /// 该分支是否**已并入主干**——已并入的才建议删。
    pub merged_to_main: bool,
    /// 本地还有几个提交未推送（>0 说明删了会丢东西）。
    pub local_unpushed: u64,
    /// 远程分支是否存在。
    pub exists: bool,
}

// ══ #11 批量清理远程分支 ═══════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrunePlan {
    pub branch: String,
    /// 可以安全删除的（已并入主干 且 本地没有未推送提交）。
    pub safe: Vec<BranchState>,
    /// 存在但**不建议删**的，附原因。
    pub risky: Vec<(BranchState, String)>,
    /// 没有远程分支的模块。
    pub skipped: Vec<(String, String)>,
}

impl PrunePlan {
    pub fn is_empty(&self) -> bool {
        self.safe.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PruneOutcome {
    pub deleted: Vec<(String, String)>,
    pub failed: Vec<(String, String)>,
    pub skipped: Vec<(String, String)>,
}

/// 预览「清理远程分支」。**不删任何东西。**
///
/// 只有**已并入主干**且**本地没有未推送提交**的才算安全。
/// 这两条任一不满足就归到 `risky` 并说明原因——批量删分支最容易
/// 顺手删掉别人还在用的。
pub fn prune_plan(
    hub: &Hub,
    ws: &Workspace,
    filter: &ModuleFilter,
    client: &Client,
) -> Result<PrunePlan> {
    let branch = ws.branch().to_string();
    let mut plan = PrunePlan {
        branch: branch.clone(),
        safe: Vec::new(),
        risky: Vec::new(),
        skipped: Vec::new(),
    };

    for m in selected_with_docs(hub, ws, filter)? {
        let Some(state) = inspect(hub, ws, &m, &branch, Some(client))? else {
            plan.skipped.push((m, "无 origin 远程".into()));
            continue;
        };
        if !state.exists {
            plan.skipped.push((m.clone(), "远程没有该分支".into()));
            continue;
        }
        // 先记下要用的字段再移动 state —— 移动后再读就借不到了
        let unpushed = state.local_unpushed;
        let merged = state.merged_to_main;
        if unpushed > 0 {
            plan.risky
                .push((state, format!("本地还有 {unpushed} 个提交未推送，删了会丢")));
        } else if !merged {
            plan.risky
                .push((state, "尚未并入主干，删了这段工作就找不回来了".into()));
        } else {
            plan.safe.push(state);
        }
    }

    Ok(plan)
}

/// 执行清理。调用方**必须先展示 [`prune_plan`] 让人确认**。
pub fn prune_apply(
    hub: &Hub,
    ws: &Workspace,
    filter: &ModuleFilter,
    client: &Client,
) -> Result<PruneOutcome> {
    let plan = prune_plan(hub, ws, filter, client)?;
    let mut out = PruneOutcome {
        skipped: plan.skipped.clone(),
        ..Default::default()
    };

    for st in &plan.safe {
        match client.delete_branch(&st.project, &st.remote_branch) {
            Ok(()) => out
                .deleted
                .push((st.module.clone(), st.remote_branch.clone())),
            Err(e) => out.failed.push((st.module.clone(), e.to_string())),
        }
    }
    // risky 的也如实报出来，别让人以为「没提到就是删了」
    for (st, why) in &plan.risky {
        out.skipped.push((st.module.clone(), why.clone()));
    }

    Ok(out)
}

// ══ #12 批量改分支名 ═══════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenamePlan {
    pub old_branch: String,
    pub new_branch: String,
    /// 会改名的模块。
    pub modules: Vec<BranchState>,
    /// 不能改的原因（分支名非法、目标已存在…）。
    pub problems: Vec<String>,
    /// 其他人需要做什么（给人看的提示）。
    pub notice: Vec<String>,
}

impl RenamePlan {
    pub fn can_apply(&self) -> bool {
        self.problems.is_empty() && !self.modules.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RenameOutcome {
    pub renamed: Vec<String>,
    pub pushed: Vec<String>,
    pub deleted_remote: Vec<String>,
    pub failed: Vec<(String, String)>,
    /// 其他人需要做的事。
    pub notice: Vec<String>,
}

/// 预览「改分支名」。会查目标分支是否已被占用。**不改任何东西。**
///
/// `client` 可以没有：**所有模块都还没推送到远程**时纯本地改名，不需要令牌。
/// 只要有一个模块已推送，就必须有 client —— 否则无法推新分支/删旧分支，
/// 半改比不改更危险。
pub fn rename_plan(
    hub: &Hub,
    ws: &Workspace,
    filter: &ModuleFilter,
    new_branch: &str,
    client: Option<&Client>,
) -> Result<RenamePlan> {
    let old_branch = ws.branch().to_string();
    let mut plan = RenamePlan {
        old_branch: old_branch.clone(),
        new_branch: new_branch.to_string(),
        modules: Vec::new(),
        problems: Vec::new(),
        notice: Vec::new(),
    };

    if let Err(e) = crate::name::validate_component(new_branch, "新分支名") {
        plan.problems.push(e.to_string());
        return Ok(plan);
    }
    if new_branch == old_branch {
        plan.problems.push("新旧分支名相同".into());
        return Ok(plan);
    }
    if !hub.config().envs.iter().all(|e| e.id != new_branch) {
        plan.problems
            .push(format!("新分支名与已有环境标识冲突: {new_branch}"));
    }

    // 工作区目录名就是分支名：目标目录已存在时不能改，否则会在
    // 本地分支已经改完之后才发现，留下半截状态。
    let new_dir = hub.ws_dir().join(new_branch);
    if new_dir.exists() {
        plan.problems
            .push(format!("目标工作区目录已存在: {}", new_dir.display()));
    }
    // 文档期次目录也随分支名走，同样先查冲突
    if let Some(doc) = ws.meta().doc.as_ref() {
        let doc_root = ws.dir().join(hub.dochub_name()).join("workspaces");
        let to = doc_root.join(&doc.family).join(new_branch);
        if to.exists() {
            plan.problems
                .push(format!("文档期次目录已存在: {}", to.display()));
        }
    }

    let mut pushed_without_client = Vec::new();
    for m in selected_with_docs(hub, ws, filter)? {
        let Some(state) = inspect(hub, ws, &m, &old_branch, client)? else {
            plan.problems.push(format!("{m}: 无 origin 远程"));
            continue;
        };
        // 目标分支已被占用的话不能改——否则会跟别人的工作撞名。
        // 没有 client 时只能看本地已取回的远程引用（可能略旧，但足以挡住撞名）。
        if state.exists {
            if let Some(client) = client {
            match client.branch_exists(&state.project, new_branch) {
                Ok(true) => {
                        plan.problems
                            .push(format!("{m}: 远程已存在分支 {new_branch}"));
                    continue;
                }
                Ok(false) => {}
                Err(e) => {
                    plan.problems.push(format!("{m}: 查询远程分支失败: {e}"));
                    continue;
                }
            }
            } else {
                pushed_without_client.push(m.clone());
                if Git::new(ws.module_path(&m)).remote_branch_exists(new_branch) {
                    plan.problems
                        .push(format!("{m}: 远程已存在分支 {new_branch}"));
                    continue;
                }
            }
        }
        plan.modules.push(state);
    }

    if !pushed_without_client.is_empty() {
        plan.problems.push(format!(
            "{} 已推送到远程，必须配置 GitLab 令牌才能安全改名（设置 → GitLab）；\
             未推送的模块可以直接改，但这里不会半改",
            pushed_without_client.join("、")
        ));
    }

    if plan.can_apply() {
        plan.notice = notices(&old_branch, new_branch);
    }
    Ok(plan)
}

/// 执行改名。调用方**必须先展示 [`rename_plan`] 让人确认**。
///
/// 顺序很重要：先推新分支（成功才继续），再删旧分支。
/// 反过来先删旧的话，推新失败就等于把远程分支弄丢了。
pub fn rename_apply(
    hub: &Hub,
    ws: &mut Workspace,
    filter: &ModuleFilter,
    new_branch: &str,
    client: Option<&Client>,
) -> Result<RenameOutcome> {
    let plan = rename_plan(hub, ws, filter, new_branch, client)?;
    if !plan.can_apply() {
        return Err(Error::other(format!(
            "不能改分支名：\n  {}",
            plan.problems.join("\n  ")
        )));
    }

    let old_branch = plan.old_branch.clone();
    let mut out = RenameOutcome {
        notice: plan.notice.clone(),
        ..Default::default()
    };

    // ① 每个模块：本地分支改名 +（已推送时）推新分支 + 删旧远程分支
    for st in &plan.modules {
        let wt = ws.module_path(&st.module);
        // 推新分支是**写远程**：带上 hub 的 token，别退回全局凭据
        let git = Git::new(&wt).with_hub_token(hub);

        if let Err(e) = git.status(&["branch", "-m", &old_branch, new_branch]) {
            out.failed
                .push((st.module.clone(), format!("本地分支改名失败: {e}")));
            continue;
        }
        out.renamed.push(st.module.clone());

        if st.exists {
            let Some(client) = client else {
                // plan 已经拦住这种情况；这里兜底，避免带着半截状态继续往下走
                out.failed.push((
                    st.module.clone(),
                    "该模块已推送到远程，但没有 GitLab 令牌".into(),
                ));
                continue;
            };
            // 先推新的
            match git.push(new_branch, true) {
                Ok(()) => out.pushed.push(st.module.clone()),
                Err(e) => {
                    out.failed
                        .push((st.module.clone(), format!("推送新分支失败: {e}")));
                    // 推失败就别删旧的了——保住远程那份
                    continue;
                }
            }
            // 确认新分支真的上去了，再删旧的
            match client.branch_exists(&st.project, new_branch) {
                Ok(true) => {}
                other => {
                    out.failed.push((
                        st.module.clone(),
                        format!(
                            "新分支未在远程确认存在（{:?}），保留旧分支",
                            other.err().map(|e| e.to_string())
                        ),
                    ));
                    continue;
                }
            }
            match client.delete_branch(&st.project, &old_branch) {
                Ok(()) => out.deleted_remote.push(st.module.clone()),
                Err(e) => out.failed.push((
                    st.module.clone(),
                    format!("旧远程分支删除失败（需手工删）: {e}"),
                )),
            }
        }
    }

    // ② 工作区目录改名：目录名就是分支名（需求 #5）
    let old_dir = ws.dir().to_path_buf();
    let new_dir = hub.ws_dir().join(new_branch);
    if new_dir.exists() {
        return Err(Error::other(format!(
            "目标目录已存在: {}（先处理掉它）",
            new_dir.display()
        )));
    }
    if let Err(e) = std::fs::rename(&old_dir, &new_dir) {
        // ⚠️ **这一步失败不能只丢一个 IO 错误出去。**
        //
        // 走到这里，**远程已经改完了**（新分支推上去了、旧远程分支删了），而本地
        // 目录还没改名。只返一个 `Err` 的话，调用方拿不到 `out`，用户看到的就是
        // 一句「文件名太长/访问被拒绝」，完全不知道远程已经变了 —— 而这在 Windows
        // 上很常见（目录被终端/IDE/资源管理器占着，rename 必失败）。
        //
        // 所以：把已经发生的事**如实回报**，并说清怎么收尾。
        out.failed.push((
            "本地工作区目录".to_string(),
            format!("改名失败: {e}"),
        ));
        out.notice.push(format!(
            "⚠️ 各模块的**远程分支与本地分支名已经改完**，只有工作区目录没动\
             （`ws/{old_branch}` 仍是旧名）。\n  \
             多半是目录被占用（终端 / IDE / 资源管理器停在里面）—— 关掉它们，\
             然后把 `ws/{old_branch}` 手工改名为 `ws/{new_branch}` 即可。",
        ));
        return Ok(out);
    }

    // worktree 元数据指向旧路径，必须修复，否则 git 认为目录已丢
    for st in &plan.modules {
        if let Ok(repo) = Repo::open(hub, &st.module) {
            let _ = repo.git().worktree_repair(&new_dir.join(&st.module));
        }
    }

    // ③ 更新元数据
    let mut reopened = Workspace::from_dir(&new_dir)?;
    reopened.meta_mut().branch = new_branch.to_string();
    reopened.meta_mut().prefix = new_branch
        .split('-')
        .next()
        .unwrap_or("feature")
        .to_string();
    let doc = reopened.meta().doc.clone();
    if let Some(doc) = doc.as_ref() {
        reopened.meta_mut().doc = Some(DocRef::new(doc.family.clone(), new_branch.to_string()));
    }
    reopened.save_meta()?;

    // ④ 文档期次目录跟着改名并刷新期次里的说明文件
    if let Some(doc) = doc {
        rename_doc_period(&new_dir, &hub.dochub_name(), &doc, new_branch, &mut out)?;
    }

    Ok(out)
}

/// 把 `workspaces/<族>/<旧分支>/` 改成新分支名，并更新期次的 `meta.json`。
///
/// 文档 worktree 已经跟着工作区目录一起挪过来了，这里只动它内部的期次目录。
/// 失败回填到 `out.failed`，不让整个改名回滚——代码分支已经改好了，
/// 半途报错反而更难收拾。
fn rename_doc_period(
    new_ws_dir: &std::path::Path,
    doc_name: &str,
    doc: &DocRef,
    new_branch: &str,
    out: &mut RenameOutcome,
) -> Result<()> {
    let root = new_ws_dir
        .join(doc_name)
        .join("workspaces")
        .join(&doc.family);
    let from = root.join(&doc.period);
    let to = root.join(new_branch);
    if !from.is_dir() || from == to {
        return Ok(());
    }
    if to.exists() {
        out.failed.push((
            doc_name.to_string(),
            format!("文档期次目录已存在，未改名: {}", to.display()),
        ));
        return Ok(());
    }
    if let Err(e) = std::fs::rename(&from, &to) {
        out.failed.push((
            doc_name.to_string(),
            format!("文档期次目录改名失败（需手工改）: {e}"),
        ));
        return Ok(());
    }

    let meta_path = to.join(crate::dochub::META_FILE);
    if meta_path.is_file() {
        match PeriodMeta::load(&meta_path) {
            Ok(mut m) => {
                m.branch = new_branch.to_string();
                if let Err(e) = m.save(&meta_path) {
                    out.failed.push((
                        doc_name.to_string(),
                        format!("文档期次 meta.json 更新失败: {e}"),
                    ));
                }
                if let Err(e) = crate::hub::write_agent_doc(&to, &render_claude_md(&m)) {
                    out.failed.push((
                        doc_name.to_string(),
                        format!("文档期次说明文件刷新失败: {e}"),
                    ));
                }
            }
            Err(e) => out.failed.push((
                doc_name.to_string(),
                format!("文档期次 meta.json 读取失败（内容未改）: {e}"),
            )),
        }
    }
    Ok(())
}

fn notices(old_branch: &str, new_branch: &str) -> Vec<String> {
    vec![
        format!("分支名已从 {old_branch} 改为 {new_branch}"),
        "**其他人需要重拉**：本地的工作区目录名还是旧的，需要：".to_string(),
        format!("  1. 删掉旧工作区目录（ws/{old_branch}/）"),
        format!("  2. 用「导入需求」按新分支名 {new_branch} 重新拉取"),
        "本地分支/目录与服务端不一致时，gws 的操作会失败，务必先重拉。".to_string(),
        format!(
            "以下游 `ws:{old_branch}` 为基线的需求需要手动改成 `ws:{new_branch}`；\
             已经创建的 MR 的源分支不会自动跟着改。"
        ),
    ]
}

/// 查一个模块的分支状态。
fn inspect(
    hub: &Hub,
    ws: &Workspace,
    module: &str,
    branch: &str,
    client: Option<&Client>,
) -> Result<Option<BranchState>> {
    let Ok(repo) = Repo::open(hub, module) else {
        return Ok(None);
    };
    // 纯本地改名不需要 GitLab 项目路径（本地裸仓库/测试仓库都拿不到它）
    let project = match repo.remote_url().and_then(|u| path_from_remote_url(&u)) {
        Some(p) => p,
        None if client.is_none() => String::new(),
        None => return Ok(None),
    };
    let git = Git::new(ws.module_path(module));
    let _ = git.fetch();

    // 没有令牌时只看本地已取回的远程引用：网络/API 不该挡住「纯本地改名」
    let exists = match client {
        Some(client) => client
            .branch_exists(&project, branch)
            .unwrap_or_else(|_| repo.git().remote_branch_exists(branch)),
        None => repo.git().remote_branch_exists(branch),
    };

    let main = repo.main_branch()?;
    let merged_to_main =
        exists && git.is_ancestor(&format!("origin/{branch}"), &format!("origin/{main}"));
    let local_unpushed = if exists {
        git.count_range(&format!("origin/{branch}..HEAD"))
            .unwrap_or(0)
    } else {
        0
    };

    Ok(Some(BranchState {
        module: module.to_string(),
        project,
        remote_branch: branch.to_string(),
        merged_to_main,
        local_unpushed,
        exists,
    }))
}

/// 参与分支清理 / 改名的「仓库」列表。
///
/// **文档库也要算进来**：它在远程同样有一条同名需求分支（`doc-hub` 这个仓库上），
/// 需求删掉后它一样需要清理。原来只用 `meta.modules`（代码模块），于是文档分支
/// 永远清不掉（实测反馈）。
///
/// 排在最后：它是特殊仓库，界面上也不该混在代码模块里。
fn selected_with_docs(hub: &Hub, ws: &Workspace, filter: &ModuleFilter) -> Result<Vec<String>> {
    let mut out = selected(ws, filter, &hub.dochub_name())?;
    if ws.meta().doc.is_some() && !out.iter().any(|m| hub.is_dochub(m)) {
        out.push(hub.dochub_name());
    }
    Ok(out)
}

fn selected(ws: &Workspace, filter: &ModuleFilter, dochub_name: &str) -> Result<Vec<String>> {
    let unmatched = filter.unmatched_only(&ws.meta().modules, dochub_name);
    if !unmatched.is_empty() {
        return Err(Error::other(format!(
            "指定的模块不在本工作区: {}",
            unmatched.join("、")
        )));
    }
    // 用**含文档库**那份：改分支名 / 清远程分支时，文档库在这条分支上的 worktree
    // 也要跟着一起（它不在 `meta.modules` 里，`present_modules()` 会把它漏掉）
    Ok(filter
        .resolve(&crate::workspace::sync::op_candidates(ws, dochub_name))
        .into_iter()
        .cloned()
        .collect())
}

/// 预览整理成给人看的行。
pub fn prune_lines(plan: &PrunePlan) -> Vec<String> {
    let mut out = Vec::new();
    if !plan.safe.is_empty() {
        out.push(format!("可安全删除的远程分支（{} 个）:", plan.safe.len()));
        for s in &plan.safe {
            out.push(format!("  {}  {}/{}", s.module, s.project, s.remote_branch));
        }
    }
    if !plan.risky.is_empty() {
        out.push(format!("\n⚠ 不建议删（{} 个）:", plan.risky.len()));
        for (s, why) in &plan.risky {
            out.push(format!("  {}  {}", s.module, why));
        }
    }
    for (m, why) in &plan.skipped {
        out.push(format!("  - {m}  {why}"));
    }
    if out.is_empty() {
        out.push("没有需要清理的远程分支".to_string());
    }
    out
}

/// 改名预览整理成给人看的行。
pub fn rename_lines(plan: &RenamePlan) -> Vec<String> {
    let mut out = vec![format!("分支名: {} → {}", plan.old_branch, plan.new_branch)];
    if !plan.modules.is_empty() {
        out.push(format!("\n将处理 {} 个模块:", plan.modules.len()));
        for m in &plan.modules {
            out.push(format!("  {}  {}/{}", m.module, m.project, m.remote_branch));
        }
    }
    for p in &plan.problems {
        out.push(format!("\n✗ {p}"));
    }
    for n in &plan.notice {
        out.push(n.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_lines_separate_safe_from_risky() {
        let st = |m: &str| BranchState {
            module: m.into(),
            project: format!("g/{m}"),
            remote_branch: "feature-x".into(),
            merged_to_main: true,
            local_unpushed: 0,
            exists: true,
        };
        let plan = PrunePlan {
            branch: "feature-x".into(),
            safe: vec![st("a")],
            risky: vec![(st("b"), "尚未并入主干".into())],
            skipped: vec![("c".into(), "远程没有该分支".into())],
        };
        let text = prune_lines(&plan).join("\n");
        assert!(text.contains("可安全删除"), "{text}");
        assert!(text.contains("不建议删"), "{text}");
        assert!(text.contains("尚未并入主干"), "{text}");
    }

    #[test]
    fn empty_prune_says_so() {
        let plan = PrunePlan {
            branch: "b".into(),
            safe: vec![],
            risky: vec![],
            skipped: vec![],
        };
        assert_eq!(
            prune_lines(&plan),
            vec!["没有需要清理的远程分支".to_string()]
        );
    }

    #[test]
    fn rename_notice_tells_others_what_to_do() {
        // 不做自动同步，而是明确告知——悄悄改掉别人的工作区危险得多
        let n = notices("feature-old", "feature-new");
        let text = n.join("\n");
        assert!(text.contains("其他人需要重拉"), "{text}");
        assert!(text.contains("feature-new"), "{text}");
        assert!(text.contains("重新拉取"), "{text}");
    }

    #[test]
    fn rename_plan_rejects_same_name() {
        let plan = RenamePlan {
            old_branch: "a".into(),
            new_branch: "a".into(),
            modules: vec![],
            problems: vec!["新旧分支名相同".into()],
            notice: vec![],
        };
        assert!(!plan.can_apply());
    }

    #[test]
    fn doc_period_directory_follows_the_branch_rename() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws_dir = tmp.path().join("feature-20261020-new");
        let old_period = ws_dir.join("doc-hub/workspaces/收银台/feature-20261020-old");
        std::fs::create_dir_all(&old_period).unwrap();
        let meta = PeriodMeta::new("feature-20261020-old", "收银台", "收银台改版");
        meta.save(&old_period.join(crate::dochub::META_FILE))
            .unwrap();

        let doc = DocRef::new("收银台", "feature-20261020-old");
        let mut out = RenameOutcome::default();
        rename_doc_period(&ws_dir, "doc-hub", &doc, "feature-20261020-new", &mut out).unwrap();

        assert!(out.failed.is_empty(), "{:?}", out.failed);
        assert!(!old_period.exists(), "旧期次目录应消失");
        let new_period = ws_dir.join("doc-hub/workspaces/收银台/feature-20261020-new");
        assert!(new_period.is_dir());

        let back = PeriodMeta::load(&new_period.join(crate::dochub::META_FILE)).unwrap();
        assert_eq!(back.branch, "feature-20261020-new");
        assert_eq!(back.family, "收银台");
        assert!(new_period.join("AGENTS.md").is_file());
        assert!(new_period.join("CLAUDE.md").is_file());
    }
}