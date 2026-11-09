//! 推送 / 拉取 / 合并 / 同步。
//!
//! 这一组原来都是**同步命令**：命令体在主线程上跑，几十个仓库的 `fetch`/`merge`
//! 期间界面完全不响应，鼠标变成旋转的等待图标（实测反馈）。现在统一走
//! `#[tauri::command(async)]` + `bg`，并上报逐模块进度。

use tauri::State;
use gws_core::workspace::sync::{self, CommitOutcome, DoneOutcome, MergeOutcome, PullOutcome, PushOutcome, SyncMainOutcome, SyncOutcome};
use gws_core::workspace::Workspace;

use super::{bg, App, CmdResult, FilterInput};

#[tauri::command(async)]
pub async fn ws_push(
    branch: String,
    filter: FilterInput,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<PushOutcome> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        Ok(sync::push_with(&hub, &ws, &filter.into(), Some(&cb))?)
    })
    .await
}

#[tauri::command(async)]
pub async fn ws_pull(
    branch: String,
    rebase: bool,
    filter: FilterInput,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<PullOutcome> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        Ok(sync::pull_with(&hub, &ws, &filter.into(), rebase, Some(&cb))?)
    })
    .await
}

/// 把各模块的未提交改动 `git add -A` 后提交（推送之前的那一步）。
#[tauri::command(async)]
pub async fn ws_commit(
    branch: String,
    message: String,
    filter: FilterInput,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<CommitOutcome> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        Ok(sync::commit_with(&hub, &ws, &filter.into(), &message, Some(&cb))?)
    })
    .await
}

#[tauri::command(async)]
pub async fn ws_merge(
    branch: String,
    env: String,
    push: bool,
    force: bool,
    filter: FilterInput,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<MergeOutcome> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        Ok(sync::merge_with(&hub, &ws, &env, &filter.into(), push, force, Some(&cb))?)
    })
    .await
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct SyncMainInput {
    /// 覆盖基线链。空 = 用需求里登记的基线。
    #[serde(default)]
    pub from: Vec<String>,
    #[serde(flatten)]
    pub filter: FilterInput,
}

#[tauri::command(async)]
pub async fn ws_sync_main(
    branch: String,
    input: SyncMainInput,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<SyncMainOutcome> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        let from = if input.from.is_empty() { None } else { Some(input.from.as_slice()) };
        Ok(sync::sync_main_with(&hub, &ws, &input.filter.into(), from, Some(&cb))?)
    })
    .await
}

// ══ 合并后检查（需求：合进 dev 之后要能看「合了什么、推没推」）══════════

/// 环境里的一个提交（历史列表展示用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct EnvCommit {
    /// 短 sha。
    pub sha: String,
    /// 作者名。
    pub author: String,
    /// 提交时间（`YYYY-MM-DD HH:MM`）。
    pub date: String,
    /// 提交说明。
    pub subject: String,
}

/// 某模块在这次合并里的状态。
#[derive(Debug, Clone, serde::Serialize)]
pub struct EnvModuleCheck {
    pub module: String,
    /// 有未解决的冲突（`git diff --diff-filter=U` 里还有东西）。
    pub conflict: bool,
    /// 冲突文件。
    pub conflict_files: Vec<String>,
    /// 远程分支上还没有的本地提交数（>0 = 没推上去）。
    pub unpushed: u64,
    /// 本地落后远程多少个提交。
    pub behind: u64,
    /// 合到环境里的提交（相对主干多出来的那些）。
    pub commits: Vec<EnvCommit>,
    /// 相对主干改了哪些文件。
    pub files: u64,
    /// 该模块在此环境实际检出的分支 —— 前端拿它拼 `origin/<分支>` 做比对基准。
    pub env_branch: String,
    /// 需求分支的提交**是否都已经进了这个环境**（`git cherry` 按补丁比对，
    /// squash 合并也算已合入）。
    pub merged: bool,
    /// 需求分支里还有多少提交没进环境（>0 = 合漏了）。
    pub missing: u64,
    /// 环境 worktree 里还有多少未提交改动。
    pub dirty: u64,
    /// 环境 worktree 当前检出的分支（空串 = detached）。
    pub head_branch: String,
    /// 检出分支与配置的环境分支是否一致（不一致时上面所有比较都不可信）。
    pub branch_ok: bool,
    pub error: Option<String>,
}

/// **合并后检查**：这个需求在各模块上到底合进环境了什么、推没推、有没有冲突。
///
/// 目的很具体：原来要看「dev 上现在是什么样」得用 IDE 打开 `envs/dev/`，
/// 而那里涉及十几个仓库，内存直接吃满。这里只读 git 元数据，不开 IDE
/// 就能回答三个问题：
///   1. 有没有冲突（有 → 去解决）
///   2. 合了什么上去（提交清单 + 改动文件数）
///   3. 推没推（没推 → 可补一次推送）
#[tauri::command(async)]
pub async fn env_merge_check(
    branch: String,
    env: String,
    app: State<'_, App>,
) -> CmdResult<Vec<EnvModuleCheck>> {
    let hub = app.hub()?;
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        // **含文档库**：它不在 `meta.modules` 里，但同样会进环境。
        // 只遍历 meta.modules 的话，文档被漏合时这里根本不会报（实测反馈：
        // 「明明有代码没合进去，却说都合进去了」）。
        let names = sync::op_candidates(&ws, &hub.dochub_name());
        let mut out = Vec::new();

        for m in names {
            let dir = hub.env_dir(&env).join(&m);
            if !dir.join(".git").exists() {
                continue; // 这个模块不参与该环境，不是错误
            }
            out.push(check_one(&hub, &m, &env, &dir, &branch));
        }
        Ok(out)
    })
    .await
}

fn check_one(
    hub: &gws_core::Hub,
    module: &str,
    env: &str,
    dir: &std::path::Path,
    req_branch: &str,
) -> EnvModuleCheck {
    let git = gws_core::git::Git::new(dir);
    let mut c = EnvModuleCheck {
        module: module.to_string(),
        conflict: false,
        conflict_files: Vec::new(),
        unpushed: 0,
        behind: 0,
        commits: Vec::new(),
        files: 0,
        env_branch: String::new(),
        merged: false,
        missing: 0,
        dirty: 0,
        head_branch: String::new(),
        branch_ok: true,
        error: None,
    };

    // 刷新 origin 引用：不算这一步的话，刚推上去的需求分支在这里还是「不存在」。
    // 失败不致命（断网时按本地已有引用判断）。
    let _ = git.fetch();

    // ① 冲突：**看未合并路径**，不看退出码
    match git.unmerged_paths() {
        Ok(paths) => {
            c.conflict = !paths.is_empty();
            c.conflict_files = paths;
        }
        Err(e) => c.error = Some(format!("读冲突状态失败：{e}")),
    }

    // 主干名（各仓库 main/master 混用，逐个探测）
    let main_ref = match gws_core::repo::Repo::open(hub, module) {
        Ok(r) => match r.main_branch() {
            Ok(m) => format!("origin/{m}"),
            Err(_) => "origin/main".to_string(),
        },
        Err(_) => "origin/main".to_string(),
    };

    // ② 合了什么：环境里相对主干多出来的提交（带作者与时间，方便判断是谁什么时候合的）
    //    `%ad` 配 `--date=format:` 给的是**本地时区**的墙上时间，不是 UTC。
    if let Ok(text) = git.out(&[
        "log",
        "--format=%h|%an|%ad|%s",
        "--date=format:%Y-%m-%d %H:%M",
        &format!("{main_ref}..HEAD"),
    ]) {
        c.commits = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| {
                // 说明里可能有 `|`，所以只切前 3 个
                let mut it = l.splitn(4, '|');
                let sha = it.next()?.trim();
                if sha.is_empty() {
                    return None;
                }
                Some(EnvCommit {
                    sha: sha.to_string(),
                    author: it.next().unwrap_or("").trim().to_string(),
                    date: it.next().unwrap_or("").trim().to_string(),
                    subject: it.next().unwrap_or("").trim().to_string(),
                })
            })
            .collect();
    }
    if let Ok(stat) = git.out(&["diff", "--shortstat", &format!("{main_ref}...HEAD")]) {
        c.files = stat
            .split_whitespace()
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
    }

    // ③ 推没推：环境分支是 `envs/<环境>/` 检出时用的那条远程分支
    let env_branch = gws_core::workspace::sync::env_branch_for(hub, env, module);
    c.env_branch = env_branch.clone();
    if git.remote_branch_exists(&env_branch) {
        match git.ahead_behind(&format!("origin/{env_branch}")) {
            Ok((ahead, behind)) => {
                c.unpushed = ahead;
                c.behind = behind;
            }
            Err(e) => c.error = Some(format!("比对远程失败：{e}")),
        }
    } else {
        c.error = Some(format!("远程还没有 {env_branch} 分支"));
    }

    // ④ **需求分支的提交到底进没进这个环境**。
    //    只看「有没有冲突 / 推没推」是不够的：合并被跳过时两个都是 0，
    //    于是界面显示「合并干净、也已全部推送」——而代码其实没合进去（实测反馈）。
    if git.remote_branch_exists(req_branch) {
        match git.count_cherry("HEAD", &format!("origin/{req_branch}")) {
            Ok(0) => c.merged = true,
            Ok(n) => {
                c.merged = false;
                c.missing = n;
            }
            Err(e) => {
                c.merged = false;
                c.error = Some(format!("比对需求分支失败：{e}"));
            }
        }
    } else {
        c.merged = false;
        if c.error.is_none() {
            c.error = Some(format!("需求分支还没推到远程（origin/{req_branch}）"));
        }
    }

    // ⑤ 环境里的现场状态：半合并留下的未提交改动、检出分支跑偏
    c.dirty = git.dirty_count().unwrap_or(0);
    c.head_branch = git.current_branch().unwrap_or_default();
    c.branch_ok = c.head_branch == env_branch;

    c
}

/// 把环境里**已经合好但还没推**的分支推上远程。
///
/// 与 `merge(push=true)` 分开：那时机是「合完顺手推」，这里是「合的时候没推，
/// 事后补一次」——实测反馈里明确要一个「补推」按钮。**只推，不合并、不改内容**。
#[tauri::command(async)]
pub async fn env_push(
    branch: String,
    env: String,
    modules: Vec<String>,
    app: State<'_, App>,
) -> CmdResult<Vec<(String, String)>> {
    use gws_core::git::Git;
    let hub = app.hub()?;
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        let mut out = Vec::new();
        for m in modules {
            let dir = hub.env_dir(&env).join(&m);
            if !dir.join(".git").exists() {
                out.push((m.clone(), "该环境没有这个模块".to_string()));
                continue;
            }
            // 分支名要按环境的覆盖表算，不是目录名（标识与分支名是解耦的）
            let env_branch = sync::env_branch_for(&hub, &env, &m);
            let git = Git::new(&dir).with_token(gws_core::gitlab::token::resolve(hub.root())?);
            match git.push(&env_branch, true) {
                Ok(()) => out.push((m.clone(), String::new())),
                Err(e) => out.push((m.clone(), e.to_string())),
            }
        }
        let _ = ws;
        Ok(out)
    })
    .await
}

/// 检查需求是否已全部并入主干。/// 检查需求是否已全部并入主干。
///
/// **慢操作**：每个模块都要 fetch 才能跟主干比，22 个模块要十几秒。
/// 所以走 `bg` + 进度上报——否则界面上就是一个不动的转圈（实测反馈 #5）。
#[tauri::command(async)]
pub async fn ws_done(
    branch: String,
    filter: FilterInput,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<DoneOutcome> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        Ok(sync::done(&hub, &ws, &filter.into(), Some(&cb))?)
    })
    .await
}

/// 同步所有仓库主干与环境。**慢操作**（22 个仓库 fetch + reset）。
///
/// 界面上已拆成两个按钮（仓库页同步主干、环境页同步该环境），
/// 这个全量版本留给 CLI 的 `gws sync`。
#[tauri::command(async)]
pub fn sync_all(app: State<'_, App>, app_handle: tauri::AppHandle) -> CmdResult<SyncOutcome> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    Ok(sync::sync_all(&hub, Some(&cb))?)
}

/// **只同步仓库主干**（仓库页的「同步最新代码」）。
#[tauri::command(async)]
pub fn sync_repos(
    filter: FilterInput,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<SyncOutcome> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    Ok(sync::sync_repos(&hub, &filter.into(), Some(&cb))?)
}

/// **只同步某个环境**（环境页的「同步此环境」）。
#[tauri::command(async)]
pub fn sync_env(
    env: String,
    filter: FilterInput,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<SyncOutcome> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    Ok(sync::sync_env(&hub, &env, &filter.into(), Some(&cb))?)
}

/// 某环境在某仓库上实际用的分支名（供界面展示环境解耦效果，需求 #13）。
#[tauri::command]
pub fn env_branch_of(env: String, module: String, app: State<'_, App>) -> CmdResult<String> {
    app.with_hub(|hub| Ok(sync::env_branch_for(hub, &env, &module)))
}

/// 打开工作区时**异步**探测远程是否有新提交（需求 #8）。
///
/// 与 `ws_detail` 分开是有意的：`ws_detail` 只读本地状态、秒回；远程探测要
/// 逐个 `fetch`，可能慢。界面先渲染本地状态，再等这个命令回来补标记。
#[tauri::command(async)]
pub fn ws_probe_remote(branch: String, app: State<'_, App>) -> CmdResult<Vec<RemoteProbe>> {
    let hub = app.hub()?;
    let ws = Workspace::open(&hub, &branch)?;
    // **含文档库**：实测反馈——「检查远程」从来标不出 doc-hub 落后，
    // 于是「拉取」也不会自动勾上它（`present_modules()` 里没有它）
    let modules = gws_core::workspace::sync::op_candidates(&ws, &hub.dochub_name());

    // 并发 fetch：22 个仓库串行要几十秒，界面就卡在那里了
    let results = gws_core::repo::map_parallel(&hub, |r| {
        if !modules.contains(&r.name) {
            return Ok(None);
        }
        let wt = ws.module_path(&r.name);
        let git = gws_core::git::Git::new(&wt);
        let _ = git.fetch();
        if !git.remote_branch_exists(&branch) {
            return Ok(None);
        }
        let (ahead, behind) = git.ahead_behind(&format!("origin/{branch}"))?;
        Ok(Some((ahead, behind)))
    })?;

    let mut out = Vec::new();
    for (module, r) in results {
        match r {
            Ok(Some((ahead, behind))) if behind > 0 => out.push(RemoteProbe {
                module,
                ahead,
                behind,
                error: None,
            }),
            Ok(_) => {}
            Err(e) => out.push(RemoteProbe {
                module,
                ahead: 0,
                behind: 0,
                error: Some(e.to_string()),
            }),
        }
    }
    out.sort_by(|a, b| a.module.cmp(&b.module));
    Ok(out)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RemoteProbe {
    pub module: String,
    /// 本地领先远程的提交数。
    pub ahead: u64,
    /// **远程领先本地的提交数**（>0 表示别人推了代码）。
    pub behind: u64,
    pub error: Option<String>,
}

