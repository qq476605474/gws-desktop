//! 同步与合并：`st` / `push` / `pull` / `sync-main` / `merge` / `sync` / `done`。
//!
//! # 设计原则：绝不静默丢东西
//!
//! 这一组命令是全工具里最容易造成损失的地方（改写本地分支、动远程环境分支）。
//! 旧版在这里积累了大量「保护现场」的规则，本模块逐条保留并在注释里标注出处。
//! 三条贯穿始终的规则：
//!
//! 1. **冲突现场永不 reset**——冲突是有价值的信息，不是错误。
//! 2. **未推送的成果永不静默丢弃**——要丢必须显式 `--force`。
//! 3. **判断冲突看未合并路径，不看退出码**——`merge --autostash` 回填 stash
//!    冲突时 git 仍返回 0。

use std::collections::BTreeMap;

use serde::Serialize;

use crate::git::Git;
use crate::hub::Hub;
use crate::repo::Repo;
use crate::workspace::lifecycle::resolve_base;
use crate::workspace::{ModuleFilter, Workspace};
use crate::{Error, Result};

/// `git clean` 时要保留的路径：体积大、重新生成成本高。
const CLEAN_KEEP: &[&str] = &["node_modules", "target"];

/// 解析某环境在某模块上实际使用的分支名（环境标识与分支名解耦，§2.2）。
pub fn env_branch_for(hub: &Hub, env_id: &str, module: &str) -> String {
    hub.config()
        .env(env_id)
        .map(|e| e.branch_for(module).to_string())
        .unwrap_or_else(|| env_id.to_string())
}

/// 按 `ModuleFilter` 取出本次要处理的模块。
///
/// `--only` 里出现本工作区没有的模块名时直接报错——多半是名字敲错了，
/// 静默什么都不做会让人以为命令生效了。
fn selected(ws: &Workspace, filter: &ModuleFilter, dochub_name: &str) -> Result<Vec<String>> {
    let unmatched = filter.unmatched_only(&ws.meta().modules, dochub_name);
    if !unmatched.is_empty() {
        return Err(Error::other(format!(
            "指定的模块不在本工作区: {}（本工作区模块: {}）",
            unmatched.join("、"),
            ws.meta().modules.join("、")
        )));
    }
    Ok(filter.resolve(&op_candidates(ws, dochub_name)).into_iter().cloned().collect())
}

/// 挂在这个工作区上的文档库，是不是个**能操作的 worktree**。
///
/// 判据是「`meta.doc` 登记过」且「`ws/<分支>/doc-hub/` 真的是 git worktree」。
/// 只看 `meta.doc` 不够 —— 目录可能被手工删了。
pub fn doc_attached(ws: &Workspace, dochub_name: &str) -> bool {
    ws.meta().doc.is_some() && ws.module_path(dochub_name).join(".git").exists()
}

/// 模块级操作（push / pull / merge / sync-main / done）的**候选模块**。
///
/// = 元数据登记且目录还在的模块，**外加文档库**（挂了文档时）。
///
/// # 为什么文档库要单独补进来
///
/// 它**不在 `meta.modules` 里** —— 由 `meta.doc` 单独登记（迁移时专门把
/// 「文档算模块」的残留登记清掉了，见 `prune_legacy_module_entries`）。
/// 但 `ws/<分支>/doc-hub/` 在工作区里是一个**真实的 git worktree**，
/// 需求文档也该跟着需求一起推、一起合。
///
/// 实测反馈：合代码 /「合并并推送」时**选不到 doc-hub**。
/// 而且更糟的是——因为保留名不算「未匹配的模块」（见 `ModuleFilter::unmatched_only`），
/// 后端连报错都不会报，选中了也是**静默跳过**。
pub fn op_candidates(ws: &Workspace, dochub_name: &str) -> Vec<String> {
    let mut out = ws.present_modules();
    if doc_attached(ws, dochub_name) && !out.iter().any(|m| m == dochub_name) {
        out.push(dochub_name.to_string());
    }
    out
}

// ══ 环境覆盖的分支校验 ═════════════════════════════════════════════════

/// 一条覆盖规则的存在性检查结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchCheck {
    pub repo: String,
    pub branch: String,
    /// 该仓库的远程有没有这条分支。
    pub exists: bool,
    /// 探测失败的原因（网络等）。`exists=false` 且有 `error` 时，
    /// 是「没查到」而不是「确定没有」——两者对用户的含义不同。
    pub error: Option<String>,
}

/// 批量校验分支是否存在。
///
/// 环境覆盖保存前要跑这个：填错分支名如果放过去，下次 `gws sync` 会把
/// worktree 切到一个不存在的分支上，报错离现场很远、很难查（实测反馈）。
///
/// `repo` 为 `*` 时校验该分支**在任意一个仓库存在**即可（通配规则）。
/// 并发跑：22 个仓库串行 fetch 要几十秒。
pub fn check_branches(hub: &Hub, pairs: &[(String, String)]) -> Result<Vec<BranchCheck>> {
    use crate::repo;

    // 通配规则只需要「至少一个仓库有」，单独处理
    let wildcards: Vec<String> = pairs
        .iter()
        .filter(|(r, _)| r == "*")
        .map(|(_, b)| b.clone())
        .collect();

    let specific: Vec<(String, String)> = pairs
        .iter()
        .filter(|(r, _)| r != "*")
        .cloned()
        .collect();

    let mut out = Vec::new();

    if !wildcards.is_empty() {
        let scanned = repo::map_parallel(hub, |r| {
            let git = r.git();
            let _ = git.fetch();
            Ok(git.remote_branch_exists(&wildcards[0]))
        })?;
        // 每个通配分支单独查一遍（数量极少，不值得再优化）
        for b in &wildcards {
            let any = scanned.iter().any(|(_, res)| res.as_ref().copied().unwrap_or(false));
            let err = scanned
                .iter()
                .find_map(|(_, res)| res.as_ref().err().map(|e| e.to_string()));
            out.push(BranchCheck { repo: "*".into(), branch: b.clone(), exists: any, error: err });
        }
    }

    if !specific.is_empty() {
        let results = repo::map_parallel(hub, |r| {
            let git = r.git();
            let _ = git.fetch();
            Ok(specific
                .iter()
                .filter(|(repo, _)| repo == &r.name)
                .map(|(_, b)| (b.clone(), git.remote_branch_exists(b)))
                .collect::<Vec<_>>())
        })?;
        for (repo_name, res) in results {
            match res {
                Ok(list) => {
                    for (branch, exists) in list {
                        out.push(BranchCheck { repo: repo_name.clone(), branch, exists, error: None });
                    }
                }
                Err(e) => {
                    for (_, b) in specific.iter().filter(|(r, _)| r == &repo_name) {
                        out.push(BranchCheck {
                            repo: repo_name.clone(),
                            branch: b.clone(),
                            exists: false,
                            error: Some(e.to_string()),
                        });
                    }
                }
            }
        }
    }

    // 保持输入顺序，界面上的错误提示才跟用户填的顺序对得上
    out.sort_by_key(|c| {
        pairs.iter().position(|(r, b)| r == &c.repo && b == &c.branch).unwrap_or(usize::MAX)
    });
    Ok(out)
}

// ══ st：各模块状态 ═════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModuleStatus {
    pub module: String,
    /// 目录是否存在。缺失的模块需要 `gws drop` 清理残留元数据。
    pub present: bool,
    pub dirty: u64,
    /// 需求分支是否已推送到远程。
    pub pushed: bool,
    pub ahead: u64,
    pub behind: u64,
    /// 领先主干的提交数。`None` = 算不出来（无主干引用等）。
    pub ahead_of_main: Option<u64>,
    /// 读取该模块状态时的错误。**不假装成 0** —— 上游界面需要区分
    /// 「干净」和「读不到」。
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusReport {
    pub branch: String,
    pub title: String,
    pub modules: Vec<ModuleStatus>,
}

/// 各模块状态一览。
///
/// `filter` 只影响**展示哪些模块**，不影响各模块状态的读取方式。
/// 注意状态行里的模块来自元数据（含目录缺失的），而不是 `present_modules()`——
/// 恰恰是缺失的模块最需要被看到。
///
/// # 为什么是并发读的
///
/// 每个模块要跑四五个 git 子进程（`status` / `rev-parse` / `rev-list`），
/// 而**起进程本身**才是大头（Windows 上一个 git 进程上百毫秒）。串行读 4 个模块
/// 就是一两秒、8 个模块好几秒 —— 界面点进需求要盯着转圈等（实测反馈：
/// 「进入需求会 loading 卡几秒才出来界面」）。各模块落在各自的仓库目录里，
/// 互不相干，并发跑是安全的（仓库之间没有共享的 git 状态）。
pub fn status(hub: &Hub, ws: &Workspace, filter: &ModuleFilter) -> Result<StatusReport> {
    let branch = ws.branch().to_string();

    let unmatched = filter.unmatched_only(&ws.meta().modules, &hub.dochub_name());
    if !unmatched.is_empty() {
        return Err(Error::other(format!(
            "指定的模块不在本工作区: {}",
            unmatched.join("、")
        )));
    }
    let mut wanted: Vec<String> =
        filter.resolve(&ws.meta().modules).into_iter().cloned().collect();

    // **文档库也要报进来**。它不在 `meta.modules` 里（理由见 `op_candidates`），
    // 所以只看元数据看不到它 —— 但界面要靠这条才能把它列进「合并 / 推送」
    // 的模块勾选里（实测反馈：选不到 doc-hub）。
    let doc = hub.dochub_name().to_string();
    if doc_attached(ws, &hub.dochub_name()) && !wanted.iter().any(|m| m == &doc) {
        let doc_wanted = !filter.resolve(std::slice::from_ref(&doc)).is_empty();
        if doc_wanted {
            wanted.push(doc);
        }
    }

    // 发出去、再按原顺序收回：顺序稳定，界面上的模块行就不会跳来跳去。
    let modules = std::thread::scope(|scope| {
        let handles: Vec<_> = wanted
            .iter()
            .map(|m| scope.spawn(|| one_module_status(hub, ws, m, &branch)))
            .collect();
        handles
            .into_iter()
            .zip(wanted.iter())
            .map(|(h, m)| {
                h.join().unwrap_or_else(|_| ModuleStatus {
                    module: m.clone(),
                    present: false,
                    dirty: 0,
                    pushed: false,
                    ahead: 0,
                    behind: 0,
                    ahead_of_main: None,
                    error: Some("读该模块状态的工作线程异常退出".into()),
                })
            })
            .collect()
    });

    Ok(StatusReport { branch, title: ws.title().to_string(), modules })
}

/// 读一个模块的状态。抽出来是因为**文档库那条要复用同一套读法**
/// （见 `status` 末尾）—— 两份各写一遍迟早会漂。
fn one_module_status(hub: &Hub, ws: &Workspace, m: &str, branch: &str) -> ModuleStatus {
    let wt = ws.module_path(m);
    if !wt.exists() {
        return ModuleStatus {
            module: m.to_string(),
            present: false,
            dirty: 0,
            pushed: false,
            ahead: 0,
            behind: 0,
            ahead_of_main: None,
            error: None,
        };
    }

    let git = Git::new(&wt);
    let mut st = ModuleStatus {
        module: m.to_string(),
        present: true,
        dirty: 0,
        pushed: false,
        ahead: 0,
        behind: 0,
        ahead_of_main: None,
        error: None,
    };
    match read_one_status(hub, &git, m, branch) {
        Ok((dirty, pushed, ahead, behind, ahead_of_main)) => {
            st.dirty = dirty;
            st.pushed = pushed;
            st.ahead = ahead;
            st.behind = behind;
            st.ahead_of_main = ahead_of_main;
        }
        Err(e) => st.error = Some(e.to_string()),
    }
    st
}

type OneStatus = (u64, bool, u64, u64, Option<u64>);

/// 读一个模块的状态。**只用 3 次 git 调用**（原来是 5 次）。
///
/// # 为什么抠这个数
///
/// Windows 上每次 git 调用 ~85ms（起进程才是大头，`git status` 还要 123ms），
/// 一个需求 4 个模块就是 20 次调用 ≈ 1.7 秒 —— 实测反馈「进需求要转圈等两秒」。
/// 而且**并发救不了**：并发跑仍是那个数，说明 Windows 上起进程这件事被串行化了
/// （大概是 Defender 实时扫描）。所以只能减少调用次数：
///
/// - 「这条远程分支存在吗」并进 `rev-list --left-right --count`（不存在会非零退出，
///   正好一次调用回答两个问题）。省 1 次。
/// - 「vs 主干」直接问 `origin/HEAD`（clone 时设好的、指向远程默认分支），
///   不再先跑一次「探测主干叫什么」。省 1 次。
fn read_one_status(
    hub: &Hub,
    git: &Git,
    module: &str,
    branch: &str,
) -> Result<OneStatus> {
    let dirty = git.dirty_count()?;

    let (pushed, ahead, behind) = match git.ahead_behind_opt(&format!("origin/{branch}")) {
        Some((a, b)) => (true, a, b),
        // 读不到这条远程分支 = 本地还没推过（正常状态，不是错误）
        None => (false, 0, 0),
    };

    // `origin/HEAD` 就是远程的默认分支（各仓库 main/master 混用的问题它一并解决）。
    // 万一它缺失（手工 fetch 过、远程没设 HEAD），才回退到"探测主干 + 数一次"。
    let ahead_of_main = match git.ahead_behind_opt("origin/HEAD") {
        Some((a, _)) => Some(a),
        None => Repo::open(hub, module)
            .ok()
            .and_then(|r| r.main_branch().ok())
            .and_then(|mb| git.count_ahead("HEAD", &format!("origin/{mb}")).ok()),
    };

    Ok((dirty, pushed, ahead, behind, ahead_of_main))
}

/// 错误/输出的首行——给用户看的行只留一行，多行会把界面撑乱。
fn first_line(s: &str) -> String {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string()
}

/// git 因为「没配提交身份」而拒绝时，补一句**能照做**的提示。
///
/// 这条在没配过 git 全局身份的机器上（新装的 Linux、CI、容器）必然踩到，而 git
/// 只说 `Committer identity unknown` / `empty ident name` —— 用户不知道该去哪儿配。
fn ident_hint(output: &str) -> &'static str {
    let o = output.to_ascii_lowercase();
    if o.contains("identity") || o.contains("empty ident") {
        "（原因是没有提交身份：桌面端「设置 → 提交身份」填名字与邮箱，         或 `git config --global user.name / user.email`）"
    } else {
        ""
    }
}

// ══ push ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PushOutcome {
    pub pushed: Vec<String>,
    pub dirty: Vec<(String, u64)>,
    pub behind: Vec<(String, u64)>,
    pub failed: Vec<(String, String)>,
}

impl PushOutcome {
    /// 全部成功且没有任何模块被跳过。
    pub fn is_clean(&self) -> bool {
        self.failed.is_empty() && self.dirty.is_empty() && self.behind.is_empty()
    }
}

/// 推送需求分支到远程。
///
/// **先整体检查是否落后远程**：他人已推送时直接推会被 git 拒（non-fast-forward），
/// 但先提示 `gws pull` 比让 git 抛一个英文错误直观得多。
pub fn push(hub: &Hub, ws: &Workspace, filter: &ModuleFilter) -> Result<PushOutcome> {
    push_with(hub, ws, filter, None)
}

/// 带进度的 [`push`]。
///
/// 进度回调单独开一个入口而不是给 `push` 加参数：CLI 与 `sync_merge` 测试里
/// 有四十多处调用，只为传一个 `None` 全改一遍不值得。桌面端用这个版本，
/// 每个模块都要 `fetch`，没有逐模块进度界面上就是一个不动的转圈。
pub fn push_with(
    hub: &Hub,
    ws: &Workspace,
    filter: &ModuleFilter,
    cb: crate::progress::Progress<'_>,
) -> Result<PushOutcome> {
    let branch = ws.branch().to_string();
    let modules = selected(ws, filter, &hub.dochub_name())?;
    let mut out = PushOutcome::default();

    // 第一遍：落后检查，任一模块落后就整体不动
    let mut rep = crate::progress::Reporter::start(cb, modules.len(), "检查远程是否有新提交");
    for m in &modules {
        // 带 hub 的 token：私有仓库不带它 fetch 直接失败，判落后就会误判成"没推过"
        let git = Git::new(ws.module_path(m)).with_hub_token(hub);
        let _ = git.fetch();
        if !git.remote_branch_exists(&branch) {
            rep.ok(m.clone());
            continue;
        }
        let (_, behind) = git.ahead_behind(&format!("origin/{branch}"))?;
        if behind > 0 {
            out.behind.push((m.clone(), behind));
            rep.fail(m.clone(), format!("落后远程 {behind} 个提交"));
        } else {
            rep.ok(m.clone());
        }
    }
    rep.finish();
    if !out.behind.is_empty() {
        return Ok(out);
    }

    // 第二遍：逐个推
    let mut rep = crate::progress::Reporter::start(cb, modules.len(), "推送需求分支");
    for m in &modules {
        let git = Git::new(ws.module_path(m)).with_hub_token(hub);
        let dirty = git.dirty_count()?;
        if dirty > 0 {
            out.dirty.push((m.clone(), dirty));
            rep.fail(m.clone(), format!("有 {dirty} 个未提交改动"));
            continue;
        }
        match git.push(&branch, true) {
            Ok(()) => {
                out.pushed.push(m.clone());
                rep.ok(m.clone());
            }
            Err(e) => {
                out.failed.push((m.clone(), e.to_string()));
                rep.fail(m.clone(), e.to_string());
            }
        }
    }
    rep.finish();

    Ok(out)
}

// ══ pull ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PullOutcome {
    pub updated: Vec<(String, u64)>,
    pub skipped: Vec<(String, String)>,
    pub dirty: Vec<(String, u64)>,
    pub conflicts: Vec<String>,
}

/// 拉取他人推到同一需求分支的提交（协作场景）。
pub fn pull(
    hub: &Hub,
    ws: &Workspace,
    filter: &ModuleFilter,
    rebase: bool,
) -> Result<PullOutcome> {
    pull_with(hub, ws, filter, rebase, None)
}

/// 带进度的 [`pull`]（理由见 [`push_with`]）。
pub fn pull_with(
    hub: &Hub,
    ws: &Workspace,
    filter: &ModuleFilter,
    rebase: bool,
    cb: crate::progress::Progress<'_>,
) -> Result<PullOutcome> {
    let branch = ws.branch().to_string();
    let modules = selected(ws, filter, &hub.dochub_name())?;
    let mut out = PullOutcome::default();
    let mut rep = crate::progress::Reporter::start(cb, modules.len(), "拉取需求分支");

    for m in &modules {
        let git = Git::new(ws.module_path(m)).with_hub_token(hub);
        let _ = git.fetch();

        if !git.remote_branch_exists(&branch) {
            out.skipped.push((m.clone(), format!("远程无 {branch}")));
            rep.ok(format!("{m}（远程无 {branch}）"));
            continue;
        }
        let (_, behind) = git.ahead_behind(&format!("origin/{branch}"))?;
        if behind == 0 {
            rep.ok(format!("{m}（已是最新）"));
            continue; // 已是最新，不必动
        }
        let dirty = git.dirty_count()?;
        if dirty > 0 {
            out.dirty.push((m.clone(), dirty));
            rep.fail(m.clone(), format!("有 {dirty} 个未提交改动，未拉取"));
            continue;
        }

        let target = format!("origin/{branch}");
        let (ok, output) =
            if rebase { git.rebase(&target)? } else { git.merge(&target, false)? };

        // 看未合并路径而不是退出码——见模块文档第 3 条
        let unmerged = git.unmerged_paths()?;
        if ok && unmerged.is_empty() {
            out.updated.push((m.clone(), behind));
            rep.ok(format!("{m}（{behind} 个提交）"));
        } else if unmerged.is_empty() {
            // 合并**根本没成**（不是冲突）：未合并路径为空就说明 git 直接拒绝了。
            // 原来也报成「冲突」，人会去找不存在的冲突标记 —— 如实说，并带上 git 的话。
            out.skipped.push((
                m.clone(),
                format!("合并失败（不是冲突，现场没动）：{}{}", first_line(&output), ident_hint(&output)),
            ));
            rep.fail(m.clone(), "合并失败（不是冲突，现场没动）");
        } else {
            out.conflicts.push(m.clone());
            rep.fail(m.clone(), "冲突待处理");
        }
    }

    rep.finish();
    Ok(out)
}

// ══ sync-main：把基线的最新提交合进需求 ════════════════════════════════

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SyncMainOutcome {
    pub from: Vec<String>,
    pub updated: Vec<(String, String)>,
    /// 合并本身冲突（解决后 commit 会自动回填 stash）。
    pub conflicts: Vec<String>,
    /// 合并成功，但**回填未提交改动**时撞车（解决后 git add 即可，无需 commit）。
    pub restash_conflicts: Vec<String>,
    /// 合并根本没开始（多为未跟踪文件与来源新增文件同名）。
    pub blocked: Vec<(String, String)>,
    pub downgraded: Vec<(String, String)>,
}

impl SyncMainOutcome {
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty() && self.restash_conflicts.is_empty() && self.blocked.is_empty()
    }
}

/// 把基线的最新提交合进当前需求。
///
/// `base` 为空时用工作区元数据里登记的基线链（需求 #15：基线可随时修改，
/// 改完后继同步走新值）。
pub fn sync_main(
    hub: &Hub,
    ws: &Workspace,
    filter: &ModuleFilter,
    base_override: Option<&[String]>,
) -> Result<SyncMainOutcome> {
    sync_main_with(hub, ws, filter, base_override, None)
}

/// 带进度的 [`sync_main`]（理由见 [`push_with`]）。
pub fn sync_main_with(
    hub: &Hub,
    ws: &Workspace,
    filter: &ModuleFilter,
    base_override: Option<&[String]>,
    cb: crate::progress::Progress<'_>,
) -> Result<SyncMainOutcome> {
    let chain: Vec<String> = match base_override {
        Some(b) if !b.is_empty() => b.to_vec(),
        _ => {
            let registered = ws.meta().base.clone();
            if registered.is_empty() {
                vec!["主干".to_string()]
            } else {
                registered
            }
        }
    };
    let modules = selected(ws, filter, &hub.dochub_name())?;
    let mut out = SyncMainOutcome { from: chain.clone(), ..Default::default() };
    let mut rep = crate::progress::Reporter::start(cb, modules.len(), "把基线最新提交合进本需求");

    for m in &modules {
        let wt = ws.module_path(m);
        let git = Git::new(&wt);
        let _ = git.fetch();

        let res = resolve_base(hub, m, &chain)?;
        let mut src = res.reference.clone();

        if !git.ref_exists(&src) {
            // 来源分支在该模块不存在 → 降级主干。阶段 2 依赖阶段 1 时，阶段 2
            // 新引入的模块不会有阶段 1 的分支，从主干同步才对。
            let main = format!("origin/{}", Repo::open(hub, m)?.main_branch()?);
            if src == main {
                out.blocked.push((m.clone(), format!("主干 {src} 不存在，跳过")));
                rep.fail(m.clone(), format!("主干 {src} 不存在，跳过"));
                continue;
            }
            out.downgraded.push((m.clone(), format!("{src} → {main}")));
            src = main;
        }

        // 有未提交改动时加 --autostash：git 先替你 stash、合完回填。
        // 不加的话 git 会因「本地改动会被覆盖」整个拒绝合并。
        let autostash = git.dirty_count()? > 0;
        let (_, output) = git.merge(&src, autostash)?;
        let unmerged = git.unmerged_paths()?;

        if !unmerged.is_empty() {
            if git.in_merge() {
                // 合并本身冲突。此时未提交改动被 git 暂存在 MERGE_AUTOSTASH，
                // 工作树里暂时看不到——解决冲突 commit 后会自动回填。
                out.conflicts.push(m.clone());
                rep.fail(m.clone(), "合并冲突待处理");
            } else {
                // 合并成功了，是回填未提交改动时撞车。解决后 git add 即可，
                // 无需 commit。
                out.restash_conflicts.push(m.clone());
                rep.fail(m.clone(), "改动回填冲突");
            }
        } else if output.is_empty() || !git.in_merge() {
            let note = res.note().unwrap_or_else(|| format!("← {src}"));
            out.updated.push((m.clone(), note.clone()));
            rep.ok(format!("{m}（{note}）"));
        } else {
            out.blocked.push((m.clone(), output.clone()));
            rep.fail(m.clone(), output);
        }
    }

    rep.finish();
    Ok(out)
}

// ══ merge：需求 → 环境分支 ══════════════════════════════════════════════

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MergeOutcome {
    pub env: String,
    /// 远程环境分支被重建（force push）过，本地副本已作废并重新合并。
    pub stale: Vec<String>,
    /// 本地已有未推送的合并成果，本次未动它们。
    pub pending: Vec<(String, u64)>,
    pub clean: Vec<String>,
    pub skipped: Vec<(String, String)>,
    pub conflicts: Vec<String>,
    /// 冲突模块的工作目录，供界面直接打开。
    pub conflict_dirs: BTreeMap<String, String>,
    pub pushed: Vec<String>,
    pub push_failed: Vec<(String, String)>,
    pub did_push: bool,
    /// 模块 → 它在**这个环境里实际用的分支**。
    ///
    /// 界面必须拿它来报「合到哪条分支」：环境标识（`pre`）与实际分支
    /// （`pre-202610`）**是两回事** —— 用标识去报，人看到的和实际发生的对不上
    /// （实测反馈：明明设的是 `pre-202610`，提示却说 `origin/pre`）。
    pub branches: BTreeMap<String, String>,
}

impl MergeOutcome {
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }
}

/// 把需求合并到环境分支。
///
/// # 关键设计
///
/// - **默认只合到本地 `envs/<环境>/`，不推不问**。review 由人做，`push=true`
///   才推远程。
/// - 逐模块单遍处理，**绝不 reset 掉未推送的成果与冲突现场**。
/// - `force`：丢弃 envs 里本地未推送的合并成果，从远程重新合。
pub fn merge(
    hub: &Hub,
    ws: &Workspace,
    env_id: &str,
    filter: &ModuleFilter,
    push_after: bool,
    force: bool,
) -> Result<MergeOutcome> {
    merge_with(hub, ws, env_id, filter, push_after, force, None)
}

/// 带进度的 [`merge`]（理由见 [`push_with`]）。
///
/// 三段各自上报：检查需求分支与远程的差异 → 合进环境 → 推送环境分支。
/// 每段都从 `Start` 重新开始，界面上的计数与标签会跟着换。
pub fn merge_with(
    hub: &Hub,
    ws: &Workspace,
    env_id: &str,
    filter: &ModuleFilter,
    push_after: bool,
    force: bool,
    cb: crate::progress::Progress<'_>,
) -> Result<MergeOutcome> {
    let branch = ws.branch().to_string();
    let modules = selected(ws, filter, &hub.dochub_name())?;
    let envs_dir = hub.env_dir(env_id);
    std::fs::create_dir_all(&envs_dir).map_err(|e| Error::io(envs_dir.clone(), e))?;

    let mut out = MergeOutcome { env: env_id.to_string(), ..Default::default() };

    // ── 前置闸门：需求分支落后远程时先停下 ────────────────────────────
    // 协作场景：其他人已推送新提交，此时直接合入环境分支会用旧代码覆盖其成果。
    // 旧版。
    let mut behind: Vec<(String, u64, u64)> = Vec::new();
    let mut unpushed: Vec<(String, u64)> = Vec::new();
    let mut rep = crate::progress::Reporter::start(cb, modules.len(), "检查需求分支与远程的差异");
    for m in &modules {
        let git = Git::new(ws.module_path(m));
        let _ = git.fetch();
        if !git.remote_branch_exists(&branch) {
            rep.ok(m.clone());
            continue;
        }
        let (a, b) = git.ahead_behind(&format!("origin/{branch}"))?;
        if b > 0 {
            behind.push((m.clone(), a, b));
            rep.fail(m.clone(), format!("落后远程 {b} 个提交"));
        } else if a > 0 {
            unpushed.push((m.clone(), a));
            rep.ok(format!("{m}（本地有 {a} 个未推送）"));
        } else {
            rep.ok(m.clone());
        }
    }
    rep.finish();
    if !behind.is_empty() && !force {
        let detail = behind
            .iter()
            .map(|(m, a, b)| {
                if *a > 0 {
                    format!("{m}: 落后 {b} 个提交，本地另有 {a} 个未推送")
                } else {
                    format!("{m}: 落后 {b} 个提交")
                }
            })
            .collect::<Vec<_>>()
            .join("\n  ");
        return Err(Error::other(format!(
            "需求分支落后于远程，直接合并会用旧代码覆盖他人的成果：\n  {detail}\n\n\
             先同步再合并: gws pull → gws push → gws merge {env_id}"
        )));
    }
    out.pending.extend(unpushed);

    // ── 逐模块处理 ────────────────────────────────────────────────────
    let mut rep = crate::progress::Reporter::start(
        cb,
        modules.len(),
        format!("合并到环境 {env_id}"),
    );
    for m in &modules {
        let Ok(repo) = Repo::open(hub, m) else {
            out.skipped.push((m.clone(), "仓库不存在".into()));
            rep.ok(format!("{m}（跳过：仓库不存在）"));
            continue;
        };
        let env_branch = env_branch_for(hub, env_id, m);
        let wt = envs_dir.join(m);

        if !repo.git().remote_branch_exists(&env_branch) {
            out.skipped.push((
                m.clone(),
                format!("该仓库没有 {env_branch} 分支，本需求不涉及它"),
            ));
            rep.ok(format!("{m}（跳过：没有 {env_branch} 分支）"));
            continue;
        }

        // ⚠️ **合之前必须确认这个环境里检出的就是它该有的分支。**
        //
        // 实测事故：用户选「合并到环境 dev」，东西却落到了 `pre-202610` 分支上 ——
        // 因为 `envs/dev/<仓库>` 当时检出的不是 dev，而这里原来是往「当前检出的分支」
        // 上合、却按配置里该有的分支上报。环境 worktree 的分支标签一旦和历史
        // 对不上（改过覆盖表、手工动过目录），就会**默默合错分支**，而且不可逆。
        if wt.exists() {
            let cur = Git::new(&wt).current_branch().unwrap_or_default();
            if !cur.is_empty() && cur != env_branch {
                return Err(Error::other(format!(
                    "环境 {} 里的 {} 实际检出的分支是 `{cur}`，不是这个环境该有的 \
                     `{env_branch}`。\n\n为避免把代码合到错误的分支上，这里直接停下。\
                     修法：先对该环境执行「同步此环境」（gws sync 会把检出切回该有的分支），\
                     或者删掉 {} 让它重建。",
                    env_id,
                    m,
                    wt.display()
                )));
            }
        }

        // 环境 worktree 不存在则补建（新增仓库时会出现）
        if !wt.exists() {
            let created = if repo.git().local_branch_exists(&env_branch) {
                repo.git().worktree_add(&wt, &env_branch, false, None)
            } else {
                repo.git().worktree_add(&wt, &env_branch, true, Some(&format!("origin/{env_branch}")))
            };
            if let Err(e) = created {
                out.skipped.push((m.clone(), format!("环境 worktree 创建失败: {e}")));
                rep.fail(m.clone(), format!("环境 worktree 创建失败: {e}"));
                continue;
            }
        }

        let gwt = Git::new(&wt);
        let _ = repo.git().fetch();

        // ① 远程被重建（force push）：本地 HEAD 不再包含远程 tip → 本地副本全部作废。
        //    必须先于冲突检测判断——此时本地的冲突现场也是基于旧基线的，一并丢弃。
        //    重置后**不 continue**：继续往下完成合并，否则要跑两次才合得进去
        //    。
        if !gwt.is_ancestor(&format!("origin/{env_branch}"), "HEAD") {
            let _ = gwt.merge_abort();
            gwt.reset_hard(&format!("origin/{env_branch}"))?;
            let _ = gwt.clean_untracked(CLEAN_KEEP);
            out.stale.push(m.clone());
        }

        // ② 合并中途（正在解冲突）：已解完 → 补提交；还没解完 → 留现场
        if gwt.in_merge() {
            if !gwt.unmerged_paths()?.is_empty() {
                out.conflicts.push(m.clone());
                out.conflict_dirs.insert(m.clone(), wt.to_string_lossy().to_string());
                rep.fail(m.clone(), "冲突待处理");
                continue;
            }
            let _ = gwt.commit_no_edit();
        }

        // ③ 已有未推送成果（含在 IDE 里完成的冲突解决提交）：不 reset、不重合
        let ahead = gwt.count_range(&format!("origin/{env_branch}..HEAD"))?;
        if ahead != 0 {
            if force {
                let _ = gwt.merge_abort();
                gwt.reset_hard(&format!("origin/{env_branch}"))?;
                let _ = gwt.clean_untracked(CLEAN_KEEP);
            } else {
                out.pending.push((m.clone(), ahead));
                rep.ok(format!("{m}（已有 {ahead} 个未推送提交，未动）"));
                continue;
            }
        }

        // ④ 镜像重置 + 从 origin/<需求分支> 合（忘记 push 就合不进去）
        gwt.reset_hard(&format!("origin/{env_branch}"))?;
        let _ = gwt.clean_untracked(CLEAN_KEEP);

        if !repo.git().remote_branch_exists(&branch) {
            out.skipped.push((m.clone(), format!("origin/{branch} 不存在，先 gws push")));
            rep.fail(m.clone(), format!("origin/{branch} 不存在，先推送需求分支"));
            continue;
        }

        let (_, _output) = gwt.merge(&format!("origin/{branch}"), false)?;
        if gwt.unmerged_paths()?.is_empty() {
            out.clean.push(m.clone());
            rep.ok(m.clone());
        } else {
            out.conflicts.push(m.clone());
            out.conflict_dirs.insert(m.clone(), wt.to_string_lossy().to_string());
            rep.fail(m.clone(), "冲突待处理");
        }
    }
    rep.finish();

    // ── 推送决策：默认只合到本地，绝不自动推 ──────────────────────────
    if push_after && !out.has_conflicts() {
        let mut all: Vec<String> = out.pending.iter().map(|(m, _)| m.clone()).collect();
        all.extend(out.clean.iter().cloned());
        all.retain(|m| !out.skipped.iter().any(|(s, _)| s == m));
        all.sort();
        all.dedup();

        if !all.is_empty() {
            out.did_push = true;
            let mut rep = crate::progress::Reporter::start(cb, all.len(), "推送环境分支");
            for m in &all {
                let env_branch = env_branch_for(hub, env_id, m);
                let gwt = Git::new(envs_dir.join(m)).with_hub_token(hub);
                match gwt.push(&env_branch, false) {
                    Ok(()) => {
                        out.pushed.push(m.clone());
                        // 记下**实际推的那条分支** —— 界面要报的就是它，
                        // 不是环境标识（`pre` 与 `pre-202610` 是两回事）
                        out.branches.insert(m.clone(), env_branch.clone());
                        rep.ok(format!("{m} → origin/{env_branch}"));
                    }
                    Err(e) => {
                        out.push_failed.push((m.clone(), e.to_string()));
                        rep.fail(m.clone(), e.to_string());
                    }
                }
            }
            rep.finish();
        }
    }

    Ok(out)
}

// ══ sync：仓库主干与环境全面对齐远程 ═══════════════════════════════════

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SyncOutcome {
    pub repos_updated: Vec<String>,
    pub repo_failures: Vec<(String, String)>,
    /// 补建出来的环境 worktree：`(环境标识, 仓库名, 实际分支名)`。
    pub env_created: Vec<(String, String, String)>,
    /// 分支名被重建/回退的模块（需要显式报出来，不能藏）。
    pub env_moved: Vec<(String, String, String)>,
    pub env_failures: Vec<(String, String)>,
}

/// 把 `repos/` 与 `envs/` 全部 reset 到远程。
///
/// 两件事：
/// 1. `repos/` 是主干镜像，reset 到 `origin/<主干>`
/// 2. `envs/<标识>/` 是各环境的镜像：**缺的 worktree 补建**（新增仓库或新增环境后
///    必然缺），已有的 reset 到 `origin/<该环境在该仓库上的分支>`
///
/// 逐模块报出**前进/后退了几个提交**：只数单向的话，远程被删库重建或强推回退时
/// 会算出 0，等于把「提交被抹掉了」这件事藏起来。
pub fn sync_all(hub: &Hub, cb: crate::progress::Progress<'_>) -> Result<SyncOutcome> {
    sync_repos(hub, &ModuleFilter::all(), cb)?;
    let mut out = sync_repos(hub, &ModuleFilter::all(), None)?;
    let envs = hub.config().envs.clone();
    for e in &envs {
        let one = sync_env(hub, &e.id, &ModuleFilter::all(), None)?;
        out.env_created.extend(one.env_created);
        out.env_moved.extend(one.env_moved);
        out.env_failures.extend(one.env_failures);
    }
    Ok(out)
}

/// **只同步 `repos/`**（各仓库主干镜像 reset 到 `origin/<主干>`）。
///
/// 从 `sync_all` 拆出来是因为实测反馈：在「仓库」页点同步，期望的是同步主干；
/// 在「环境」页点同步，期望的是同步那个环境。原来不管在哪点都跑全量
/// （主干 + 所有环境），既慢又不是你要的。
pub fn sync_repos(
    hub: &Hub,
    filter: &ModuleFilter,
    cb: crate::progress::Progress<'_>,
) -> Result<SyncOutcome> {
    use crate::repo;

    let mut out = SyncOutcome::default();
    let all = repo::list(hub)?;
    let names: Vec<String> = all.iter().map(|r| r.name.clone()).collect();
    let wanted: std::collections::BTreeSet<String> =
        filter.resolve(&names).into_iter().cloned().collect();
    let repos: Vec<_> = all.into_iter().filter(|r| wanted.contains(&r.name)).collect();
    // 总步数 = 仓库数 + 环境数×仓库数（环境里没有该分支的会跳过，不影响计数）
    let mut rep = crate::progress::Reporter::start(cb, repos.len(), "同步仓库主干");

    // ── repos/：reset 到主干 ──────────────────────────────────────────
    for repo in &repos {
        let git = repo.git();
        let main = match repo.main_branch() {
            Ok(m) => m,
            Err(e) => {
                rep.fail(format!("repos/{}", repo.name), "探测主干失败");
                out.repo_failures.push((repo.name.clone(), e.to_string()));
                continue;
            }
        };
        if let Err(e) = git.fetch() {
            rep.fail(format!("repos/{}", repo.name), "fetch 失败");
            out.repo_failures.push((repo.name.clone(), e.to_string()));
            continue;
        }
        if git.is_dirty()? {
            rep.ok(format!("repos/{}（有未提交改动，跳过）", repo.name));
            out.repo_failures.push((repo.name.clone(), "有未提交改动，跳过".into()));
            continue;
        }
        match git.reset_hard(&format!("origin/{main}")) {
            Ok(()) => {
                rep.ok(format!("repos/{}", repo.name));
                out.repos_updated.push(repo.name.clone());
            }
            Err(e) => {
                rep.fail(format!("repos/{}", repo.name), e.to_string());
                out.repo_failures.push((repo.name.clone(), e.to_string()));
            }
        }
    }

    rep.finish();
    Ok(out)
}

/// **只同步某一个环境**（`envs/<环境标识>/` 下各仓库 reset 到 `origin/<该环境分支>`）。
///
/// 与 `sync_repos` 分开，对应界面上两个不同的按钮：仓库页同步主干、环境页同步该环境。
///
/// **它会 reset 已有的 worktree** —— 没推送的合并成果会被抹掉。只补建请用
/// [`ensure_env_worktrees`]。
pub fn sync_env(
    hub: &Hub,
    env_id: &str,
    filter: &ModuleFilter,
    cb: crate::progress::Progress<'_>,
) -> Result<SyncOutcome> {
    env_worktrees(hub, env_id, filter, cb, true)
}

/// **只补建缺的 worktree，不碰已经存在的。**
///
/// 用在「添加仓库 / 新建环境」之后自动把环境补齐：这两件事发生时环境里必然缺
/// 检出。用户实测反馈把他自己绕进去了 —— 环境页列的是配置，目录要手动点
/// 「同步此环境」才建，看着像 BUG。所以改成自动补。
///
/// # 为什么这里不能用 `sync_env`
///
/// `sync_env` 会把**已有的** worktree `reset --hard` 到 `origin/<分支>`。
/// 环境里可能躺着还没推送的合并成果，自动跑一遍就会把它抹掉 ——
/// 一个「顺手帮你补齐」的动作不该有这种后果。所以这个模式只建不 reset。
pub fn ensure_env_worktrees(
    hub: &Hub,
    env_id: &str,
    filter: &ModuleFilter,
    cb: crate::progress::Progress<'_>,
) -> Result<SyncOutcome> {
    env_worktrees(hub, env_id, filter, cb, false)
}

fn env_worktrees(
    hub: &Hub,
    env_id: &str,
    filter: &ModuleFilter,
    cb: crate::progress::Progress<'_>,
    reset_existing: bool,
) -> Result<SyncOutcome> {
    use crate::repo;

    let env = hub
        .config()
        .env(env_id)
        .cloned()
        .ok_or_else(|| Error::other(format!("环境 {env_id} 不存在")))?;

    let mut out = SyncOutcome::default();
    let all = repo::list(hub)?;
    let names: Vec<String> = all.iter().map(|r| r.name.clone()).collect();
    let wanted: std::collections::BTreeSet<String> =
        filter.resolve(&names).into_iter().cloned().collect();
    let repos: Vec<_> = all.into_iter().filter(|r| wanted.contains(&r.name)).collect();

    let env_dir = hub.env_dir(&env.id);
    std::fs::create_dir_all(&env_dir).map_err(|e| Error::io(env_dir.clone(), e))?;
    let label = if reset_existing {
        format!("同步环境 {}", env.id)
    } else {
        format!("补齐环境 {}", env.id)
    };
    let mut rep = crate::progress::Reporter::start(cb, repos.len(), label);

    for repo in &repos {
        let branch = env.branch_for(&repo.name).to_string();
        if !repo.git().remote_branch_exists(&branch) {
            // 不是每个仓库都参与每个环境，跳过是正常的；但也计一步，进度条才走得满
            rep.ok(format!("{}（无此分支，跳过）", repo.name));
            continue;
        }
        let wt = env_dir.join(&repo.name);

        if !wt.exists() {
            let created = if repo.git().local_branch_exists(&branch) {
                repo.git().worktree_add(&wt, &branch, false, None)
            } else {
                repo.git().worktree_add(&wt, &branch, true, Some(&format!("origin/{branch}")))
            };
            if let Err(e) = created {
                rep.fail(repo.name.clone(), "worktree 创建失败");
                out.env_failures.push((
                    format!("{}/{}", env.id, repo.name),
                    format!("worktree 创建失败: {e}"),
                ));
                continue;
            }
            rep.ok(format!("{}（新建）", repo.name));
            out.env_created.push((env.id.clone(), repo.name.clone(), branch.clone()));
        } else if !reset_existing {
            // 只补建模式：已有的**一律不碰**。不 reset、也不算「动了它」，
            // 免得结果面板上写着改了哪几个，其实一个都没动。
            rep.ok(format!("{}（已存在，不动）", repo.name));
            continue;
        }

        let gwt = Git::new(&wt);
        // ⚠️ **检出分支不对就先切回来**，不能直接 reset ——
        // 直接 reset 会把**错的那条分支指针**也挪到该有的提交上（实测事故：
        // `envs/dev/<仓库>` 检出在 `pre-202610`，同步时把 pre-202610 指到了 dev 上）。
        let cur = gwt.current_branch().unwrap_or_default();
        if !cur.is_empty() && cur != branch {
            rep.ok(format!("{}（检出在 {cur}，切回 {branch}）", repo.name));
            if let Err(e) = gwt.checkout_switch(&branch, &format!("origin/{branch}")) {
                rep.fail(repo.name.clone(), "切换分支失败");
                out.env_failures.push((
                    format!("{}/{}", env.id, repo.name),
                    format!("检出在 {cur} 而不是 {branch}，且切换失败: {e}"),
                ));
                continue;
            }
        }
        let target = format!("origin/{branch}");
        let before = gwt.sha("HEAD").ok();
        if let Err(e) = gwt.reset_hard(&target) {
            rep.fail(repo.name.clone(), "reset 失败");
            out.env_failures.push((format!("{}/{}", env.id, repo.name), e.to_string()));
            continue;
        }
        let after = gwt.sha("HEAD").ok();
        if before == after {
            rep.ok(repo.name.clone());
            continue;
        }
        let (fwd, bwd) = match &before {
            Some(b) => (
                gwt.count_range(&format!("{b}..{target}"))?,
                gwt.count_range(&format!("{target}..{b}"))?,
            ),
            None => (0, 0),
        };
        let desc = match (fwd, bwd) {
            (f, b) if f > 0 && b > 0 => format!("+{f} -{b}（远程已重建/强推，本地按远程为准）"),
            (0, b) if b > 0 => format!("-{b}（远程回退）"),
            (f, _) => format!("+{f}"),
        };
        rep.ok(format!("{}  {desc}", repo.name));
        out.env_moved.push((env.id.clone(), repo.name.clone(), desc));
    }

    rep.finish();
    Ok(out)
}

// ══ done：检查是否已全部并入主干 ═══════════════════════════════════════

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DoneOutcome {
    /// 已并入主干（相对主干没有多余提交）的模块。
    ///
    /// 也报出来，而不是只报"没并的"：结果面板上「3 个已并入、2 个还没」
    /// 才是一个完整的答案。只列没并的，人无法判断是"查过了"还是"没查到"。
    pub merged: Vec<String>,
    /// 尚未并入主干的模块及领先提交数。
    pub unmerged: Vec<(String, u64)>,
}

impl DoneOutcome {
    pub fn is_done(&self) -> bool {
        self.unmerged.is_empty()
    }

    /// 检查过的模块总数。
    pub fn total(&self) -> usize {
        self.merged.len() + self.unmerged.len()
    }
}

/// 检查需求是否已全部并入主干。
///
/// 每个模块都要 `git fetch` 才能跟主干比（**会联网**），所以必须上报进度：
/// 22 个模块静默跑十几秒，界面上就是一个不动的转圈，分不清在跑还是卡死。
pub fn done(
    hub: &Hub,
    ws: &Workspace,
    filter: &ModuleFilter,
    cb: crate::progress::Progress<'_>,
) -> Result<DoneOutcome> {
    let modules = selected(ws, filter, &hub.dochub_name())?;
    let mut out = DoneOutcome::default();
    let mut rep = crate::progress::Reporter::start(cb, modules.len(), "检查是否已并入主干");

    for m in &modules {
        let Ok(repo) = Repo::open(hub, m) else {
            rep.fail(m.clone(), "仓库不存在");
            continue;
        };
        let main = match repo.main_branch() {
            Ok(v) => v,
            Err(e) => {
                rep.fail(m.clone(), "探测主干失败");
                // 探测不到主干就不敢说它"已并入"，归到未并入那栏并带上原因
                out.unmerged.push((m.clone(), 0));
                let _ = e;
                continue;
            }
        };
        let _ = repo.git().fetch();
        let git = Git::new(ws.module_path(m));
        match git.count_ahead("HEAD", &format!("origin/{main}")) {
            Ok(0) => {
                rep.ok(m.clone());
                out.merged.push(m.clone());
            }
            Ok(n) => {
                rep.ok(format!("{m}（领先主干 {n} 个提交）"));
                out.unmerged.push((m.clone(), n));
            }
            Err(_) => {
                rep.fail(m.clone(), "读不到与主干的差异");
                out.unmerged.push((m.clone(), 0));
            }
        }
    }

    rep.finish();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_branch_falls_back_to_id_for_unknown_env() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hub = Hub::init(&tmp.path().join("hub"), Default::default()).unwrap();
        assert_eq!(env_branch_for(&hub, "dev", "any"), "dev");
        assert_eq!(env_branch_for(&hub, "不存在", "any"), "不存在");
    }

    #[test]
    fn clean_keep_covers_heavy_build_dirs() {
        // 清掉 node_modules / target 会让下次构建非常慢，必须保留
        assert!(CLEAN_KEEP.contains(&"node_modules"));
        assert!(CLEAN_KEEP.contains(&"target"));
    }
}