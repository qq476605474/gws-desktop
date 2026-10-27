//! 撤销本地改动（需求 #3）与强制回退到远程（需求 #10）。
//!
//! # 为什么这两个放在一起
//!
//! 它们是同一件事的两个尺度：
//!
//! - **撤销改动**（`revert`）：丢掉工作区里**未提交**的东西，分支不动
//! - **强制回退**（`reset`）：把整个需求拉回远程分支的状态，**连本地提交一起丢**
//!
//! 共同点是**会丢东西**，所以都遵循同一条纪律：先给出精确的影响面清单
//! （哪些文件、几个提交），让人确认之后才动手。
//!
//! # 需求 #10 的真实场景
//!
//! 「有人重建了分支回退了一些代码」——测试工程师不写代码只读代码，他们的
//! 工作区会停留在旧的分支状态；或者别人的 force push 让本地分叉了。
//! 这时候需要一键把本地拉回远程的样子。
//!
//! [`SyncOutcome`]: crate::workspace::sync::SyncOutcome

use std::collections::BTreeMap;

use serde::Serialize;

use crate::git::Git;
use crate::hub::Hub;
use crate::workspace::{ModuleFilter, Workspace};
use crate::{Error, Result};

/// `git clean` 时要保留的路径。体积大、重新生成成本高，
/// 清掉会让下一次构建变得很慢，而这跟「撤销改动」的意图无关。
pub const CLEAN_KEEP: &[&str] = &["node_modules", "target", ".idea", ".vscode"];

/// 某个模块将被撤销的内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RevertModule {
    pub module: String,
    /// 已跟踪文件的改动（`checkout` 会还原它们）。
    pub modified: Vec<String>,
    /// 未跟踪文件（`clean` 会删掉它们）。
    ///
    /// **未跟踪文件是最危险的**：新写的代码如果还没 `git add`，撤销就是
    /// 永久删除，git 里没有任何记录。所以界面上必须把这类单独列出来。
    pub untracked: Vec<String>,
}

impl RevertModule {
    pub fn file_count(&self) -> usize {
        self.modified.len() + self.untracked.len()
    }
}

/// 撤销前的预览。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RevertPlan {
    pub branch: String,
    pub modules: Vec<RevertModule>,
}

impl RevertPlan {
    pub fn total_files(&self) -> usize {
        self.modules.iter().map(RevertModule::file_count).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }
}

/// 撤销的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RevertOutcome {
    pub reverted: Vec<(String, usize)>,
    pub failed: Vec<(String, String)>,
}

/// 预览「撤销改动」会丢掉什么。**不改任何东西。**
pub fn revert_plan(hub: &Hub, ws: &Workspace, filter: &ModuleFilter) -> Result<RevertPlan> {
    let modules = selected(ws, filter, &hub.dochub_name())?;
    let mut out = Vec::new();

    for m in modules {
        let git = Git::new(ws.module_path(&m));
        let porcelain = git.status_porcelain()?;
        let mut modified = Vec::new();
        let mut untracked = Vec::new();
        for line in porcelain.lines() {
            if line.len() < 4 {
                continue;
            }
            let (code, path) = line.split_at(2);
            let path = path.trim();
            if code.starts_with("??") {
                untracked.push(path.to_string());
            } else {
                modified.push(path.to_string());
            }
        }
        if !modified.is_empty() || !untracked.is_empty() {
            out.push(RevertModule { module: m, modified, untracked });
        }
    }

    let _ = hub;
    Ok(RevertPlan { branch: ws.branch().to_string(), modules: out })
}

/// 执行撤销。调用方**必须先展示 [`revert_plan`] 让人确认**。
pub fn revert_apply(hub: &Hub, ws: &Workspace, filter: &ModuleFilter) -> Result<RevertOutcome> {
    let plan = revert_plan(hub, ws, filter)?;
    let mut out = RevertOutcome::default();

    for m in &plan.modules {
        let git = Git::new(ws.module_path(&m.module));
        // 先还原已跟踪文件，再清未跟踪文件。顺序反过来的话，
        // clean 可能删掉 checkout 正要还原的目标。
        if let Err(e) = git.status(&["checkout", "--", "."]) {
            out.failed.push((m.module.clone(), e.to_string()));
            continue;
        }
        let keep: Vec<&str> = CLEAN_KEEP.to_vec();
        if let Err(e) = git.clean_untracked(&keep) {
            out.failed.push((m.module.clone(), e.to_string()));
            continue;
        }
        // 撤销后校验真的干净了——不然「报成功但没撤销」是老问题
        let left = git.dirty_count()?;
        if left > 0 {
            out.failed.push((m.module.clone(), format!("撤销后仍有 {left} 处改动")));
            continue;
        }
        out.reverted.push((m.module.clone(), m.file_count()));
    }

    Ok(out)
}

// ══ 强制回退到远程（需求 #10）══════════════════════════════════════════

/// 某个模块回退后会丢掉什么。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResetModule {
    pub module: String,
    /// 目标 ref，如 `origin/feature-20261010-x`。
    pub target: String,
    /// 会被丢弃的本地提交数（`origin/<分支>..HEAD`）。
    pub local_commits: u64,
    /// 会被还原的文件数（未提交改动）。
    pub dirty_files: u64,
    /// 本地分支是否领先于远程（有未推送的提交）。
    pub ahead: bool,
    /// 本地分支是否落后远程。
    pub behind: bool,
}

/// 回退前的预览。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResetPlan {
    pub branch: String,
    pub modules: Vec<ResetModule>,
    /// 远程没有该分支的模块（跳过，不动它们）。
    pub skipped: Vec<(String, String)>,
}

impl ResetPlan {
    /// 会丢掉的本地提交总数。
    pub fn total_local_commits(&self) -> u64 {
        self.modules.iter().map(|m| m.local_commits).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ResetOutcome {
    pub reset: Vec<(String, String)>,
    pub skipped: Vec<(String, String)>,
    pub failed: Vec<(String, String)>,
}

/// 预览「强制回退到远程」会丢掉什么。**不改任何东西。**
pub fn reset_plan(hub: &Hub, ws: &Workspace, filter: &ModuleFilter) -> Result<ResetPlan> {
    let _ = hub;
    let branch = ws.branch().to_string();
    let mut modules = Vec::new();
    let mut skipped = Vec::new();

    for m in selected(ws, filter, &hub.dochub_name())? {
        let git = Git::new(ws.module_path(&m));
        let _ = git.fetch();
        let target = format!("origin/{branch}");
        if !git.ref_exists(&target) {
            skipped.push((m, format!("远程没有 {branch}")));
            continue;
        }
        let (ahead, behind) = git.ahead_behind(&target)?;
        modules.push(ResetModule {
            module: m,
            target,
            local_commits: ahead,
            dirty_files: git.dirty_count()?,
            ahead: ahead > 0,
            behind: behind > 0,
        });
    }

    Ok(ResetPlan { branch, modules, skipped })
}

/// 执行强制回退。调用方**必须先展示 [`reset_plan`] 让人确认**。
pub fn reset_apply(hub: &Hub, ws: &Workspace, filter: &ModuleFilter) -> Result<ResetOutcome> {
    let plan = reset_plan(hub, ws, filter)?;
    let mut out = ResetOutcome { skipped: plan.skipped.clone(), ..Default::default() };

    for m in &plan.modules {
        let git = Git::new(ws.module_path(&m.module));
        // 合并中途要先中止，否则 reset 会留下 MERGE_HEAD 让人困惑
        if git.in_merge() {
            let _ = git.merge_abort();
        }
        if let Err(e) = git.reset_hard(&m.target) {
            out.failed.push((m.module.clone(), e.to_string()));
            continue;
        }
        let keep: Vec<&str> = CLEAN_KEEP.to_vec();
        let _ = git.clean_untracked(&keep);

        // 回退后校验 HEAD 真的等于目标——不然「报成功但没回退」
        match git.sha("HEAD") {
            Ok(head) => match git.sha(&m.target) {
                Ok(t) if head == t => out.reset.push((m.module.clone(), m.target.clone())),
                Ok(t) => out.failed.push((
                    m.module.clone(),
                    format!("回退后 HEAD 与目标不一致（{} vs {}）", &head[..7.min(head.len())], &t[..7.min(t.len())]),
                )),
                Err(e) => out.failed.push((m.module.clone(), e.to_string())),
            },
            Err(e) => out.failed.push((m.module.clone(), e.to_string())),
        }
    }

    Ok(out)
}

/// 按 `ModuleFilter` 取出要处理的模块。
fn selected(ws: &Workspace, filter: &ModuleFilter, dochub_name: &str) -> Result<Vec<String>> {
    let unmatched = filter.unmatched_only(&ws.meta().modules, dochub_name);
    if !unmatched.is_empty() {
        return Err(Error::other(format!(
            "指定的模块不在本工作区: {}",
            unmatched.join("、")
        )));
    }
    // 同上：文档库也是挂在需求分支上的 worktree，撤销改动要能选到它
    Ok(filter.resolve(&crate::workspace::sync::op_candidates(ws, dochub_name)).into_iter().cloned().collect())
}

/// 把预览整理成给人看的行（CLI 与界面共用）。
pub fn revert_lines(plan: &RevertPlan) -> Vec<String> {
    if plan.is_empty() {
        return vec!["没有未提交的改动".to_string()];
    }
    let mut out = vec![format!("将撤销 {} 个模块、{} 处改动：", plan.modules.len(), plan.total_files())];
    let mut by_module: BTreeMap<&str, &RevertModule> = BTreeMap::new();
    for m in &plan.modules {
        by_module.insert(&m.module, m);
    }
    for m in by_module.values() {
        out.push(format!("  {}  （{} 处改动，{} 个未跟踪文件）", m.module, m.modified.len(), m.untracked.len()));
        if !m.untracked.is_empty() {
            out.push("    ⚠ 未跟踪文件会被永久删除（git 里没有记录）:".to_string());
            for f in m.untracked.iter().take(20) {
                out.push(format!("       {f}"));
            }
            if m.untracked.len() > 20 {
                out.push(format!("       …另有 {} 个", m.untracked.len() - 20));
            }
        }
    }
    out
}

/// 把回退预览整理成给人看的行。
pub fn reset_lines(plan: &ResetPlan) -> Vec<String> {
    let mut out = Vec::new();
    if plan.is_empty() && plan.skipped.is_empty() {
        return vec!["没有可回退的模块".to_string()];
    }
    if !plan.modules.is_empty() {
        out.push(format!(
            "将把 {} 个模块回退到远程分支，**丢弃 {} 个本地提交**：",
            plan.modules.len(),
            plan.total_local_commits()
        ));
        for m in &plan.modules {
            let mut bits = vec![format!("→ {}", m.target)];
            if m.local_commits > 0 {
                bits.push(format!("丢弃 {} 个本地提交", m.local_commits));
            }
            if m.dirty_files > 0 {
                bits.push(format!("还原 {} 处未提交改动", m.dirty_files));
            }
            if m.local_commits == 0 && m.dirty_files == 0 {
                bits.push("本来就和远程一致".to_string());
            }
            out.push(format!("  {}  {}", m.module, bits.join("，")));
        }
    }
    for (m, why) in &plan.skipped {
        out.push(format!("  - {m}  {why}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revert_lines_warn_about_untracked_files() {
        let plan = RevertPlan {
            branch: "feature-x".into(),
            modules: vec![RevertModule {
                module: "repo-a".into(),
                modified: vec!["a.txt".into()],
                untracked: vec!["new.txt".into()],
            }],
        };
        let lines = revert_lines(&plan);
        let text = lines.join("\n");
        // 未跟踪文件是永久删除，必须显眼
        assert!(text.contains("永久删除"), "{text}");
        assert!(text.contains("new.txt"), "{text}");
    }

    #[test]
    fn empty_plans_say_so_instead_of_printing_nothing() {
        let plan = RevertPlan { branch: "b".into(), modules: vec![] };
        assert_eq!(revert_lines(&plan), vec!["没有未提交的改动".to_string()]);

        let plan = ResetPlan { branch: "b".into(), modules: vec![], skipped: vec![] };
        assert_eq!(reset_lines(&plan), vec!["没有可回退的模块".to_string()]);
    }

    #[test]
    fn reset_lines_mention_discarded_commits() {
        let plan = ResetPlan {
            branch: "feature-x".into(),
            modules: vec![ResetModule {
                module: "repo-a".into(),
                target: "origin/feature-x".into(),
                local_commits: 3,
                dirty_files: 2,
                ahead: true,
                behind: false,
            }],
            skipped: vec![],
        };
        let text = reset_lines(&plan).join("\n");
        assert!(text.contains("丢弃 3 个本地提交"), "{text}");
        assert!(text.contains("还原 2 处未提交改动"), "{text}");
    }

    #[test]
    fn clean_keep_covers_heavy_build_dirs() {
        // 清掉这些会让下次构建非常慢，而它们跟「撤销改动」的意图无关
        assert!(CLEAN_KEEP.contains(&"node_modules"));
        assert!(CLEAN_KEEP.contains(&"target"));
        assert!(CLEAN_KEEP.contains(&".idea"));
    }
}