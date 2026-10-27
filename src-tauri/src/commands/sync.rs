//! 推送 / 拉取 / 合并 / 同步。

use tauri::State;
use gws_core::workspace::sync::{self, DoneOutcome, MergeOutcome, PullOutcome, PushOutcome, SyncMainOutcome, SyncOutcome};
use gws_core::workspace::Workspace;

use super::{bg, App, CmdResult, FilterInput};

#[tauri::command]
pub fn ws_push(branch: String, filter: FilterInput, app: State<'_, App>) -> CmdResult<PushOutcome> {
    app.with_hub(|hub| {
        let ws = Workspace::open(hub, &branch)?;
        sync::push(hub, &ws, &filter.into())
    })
}

#[tauri::command]
pub fn ws_pull(
    branch: String,
    rebase: bool,
    filter: FilterInput,
    app: State<'_, App>,
) -> CmdResult<PullOutcome> {
    app.with_hub(|hub| {
        let ws = Workspace::open(hub, &branch)?;
        sync::pull(hub, &ws, &filter.into(), rebase)
    })
}

#[tauri::command]
pub fn ws_merge(
    branch: String,
    env: String,
    push: bool,
    force: bool,
    filter: FilterInput,
    app: State<'_, App>,
) -> CmdResult<MergeOutcome> {
    app.with_hub(|hub| {
        let ws = Workspace::open(hub, &branch)?;
        sync::merge(hub, &ws, &env, &filter.into(), push, force)
    })
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct SyncMainInput {
    /// 覆盖基线链。空 = 用需求里登记的基线。
    #[serde(default)]
    pub from: Vec<String>,
    #[serde(flatten)]
    pub filter: FilterInput,
}

#[tauri::command]
pub fn ws_sync_main(
    branch: String,
    input: SyncMainInput,
    app: State<'_, App>,
) -> CmdResult<SyncMainOutcome> {
    app.with_hub(|hub| {
        let ws = Workspace::open(hub, &branch)?;
        let from = if input.from.is_empty() { None } else { Some(input.from.as_slice()) };
        sync::sync_main(hub, &ws, &input.filter.into(), from)
    })
}

// ══ 合并后检查（需求：合进 dev 之后要能看「合了什么、推没推」）══════════

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
    /// 合到环境里的提交（相对主干多出来的那些）：`(短 sha, 说明)`。
    pub commits: Vec<(String, String)>,
    /// 相对主干改了哪些文件。
    pub files: u64,
    /// 该模块在此环境实际检出的分支 —— 前端拿它拼 `origin/<分支>` 做比对基准。
    pub env_branch: String,
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
        let names: Vec<String> = ws.meta().modules.clone();
        let mut out = Vec::new();

        for m in names {
            let dir = hub.env_dir(&env).join(&m);
            if !dir.join(".git").exists() {
                continue; // 这个模块不参与该环境，不是错误
            }
            out.push(check_one(&hub, &m, &env, &dir));
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
        error: None,
    };

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

    // ② 合了什么：环境里相对主干多出来的提交
    if let Ok(text) = git.out(&["log", "--format=%h|%s", &format!("{main_ref}..HEAD")]) {
        c.commits = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let (sha, subject) = l.split_once('|').unwrap_or((l, ""));
                (sha.to_string(), subject.to_string())
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

