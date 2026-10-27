//! 仓库与环境。

use serde::Serialize;
use tauri::State;
use gws_core::env::EnvDef;
use gws_core::repo::{self, RepoOutcome};
use gws_core::workspace::sync::{self, env_branch_for, SyncOutcome};
use gws_core::workspace::ModuleFilter;

use super::{App, CmdResult, CommandError};

#[derive(Debug, Clone, Serialize)]
pub struct RepoRow {
    pub name: String,
    /// 主干分支（各仓库 main/master 混用，逐个探测）。
    pub main_branch: String,
    /// 被哪些地方使用（工作区/环境）。删除前要能看见，否则不知道删了会影响谁。
    /// 被哪些**需求**引用（`ws/<分支>`）。**只有这一类会拦住「移除仓库」**。
    pub usages: Vec<String>,
    /// 在哪些**环境**里有检出（`envs/<标识>`）。删仓库时会被一并清掉，不拦。
    pub env_usages: Vec<String>,
    pub remote_url: String,
    /// 是不是那个特殊仓库（文档库）。
    ///
    /// 它现在就在 `repos/` 下、跟普通仓库一样被列出来和同步，但**不能当模块加、
    /// 不能移除**。给前端一个标志，省得它在好几处各自判断名字。
    pub is_doc: bool,
}

#[tauri::command]
pub fn repo_list(app: State<'_, App>) -> CmdResult<Vec<RepoRow>> {
    app.with_hub(|hub| {
        Ok(repo::list(hub)?
            .into_iter()
            .map(|r| RepoRow {
                main_branch: r.main_branch().unwrap_or_else(|_| "?".into()),
                usages: repo::workspace_usages(hub, &r.name),
                env_usages: repo::env_usages(hub, &r.name),
                remote_url: r.remote_url().unwrap_or_default(),
                is_doc: hub.is_dochub(&r.name),
                name: r.name,
            })
            .collect())
    })
}

/// 添加仓库的结果。
///
/// 加完仓库要**顺手把环境补齐**（见 `repo_add`），所以返回值不能只有仓库那一栏 ——
/// 环境那一步失败了也得能报到界面上，否则又是「悄悄没做」。
#[derive(Debug, Clone, Serialize)]
pub struct RepoAddOutcome {
    pub repos: Vec<RepoOutcome>,
    /// 自动补建出来的环境 worktree：`(环境标识, 仓库名, 分支名)`。
    pub env_created: Vec<(String, String, String)>,
    /// 环境补齐失败：`(环境标识/仓库，原因)`。**不影响仓库添加本身**。
    pub env_failures: Vec<(String, String)>,
}

/// `ensure_envs_for` 的结果。用结构体而不是元组：元组在两个调用点各拆一次，
/// 拆错顺序（created/failures 都是 `Vec<(String, String, ...)>` 一类）不会报错，
/// 只会把失败当成功显示。
#[derive(Debug, Default, Serialize)]
pub struct EnvEnsure {
    /// 补建出来的检出：`(环境标识, 仓库名, 分支名)`。
    pub created: Vec<(String, String, String)>,
    /// 失败项：`(环境标识/仓库, 原因)`。
    pub failures: Vec<(String, String)>,
}

/// 加完仓库后，**把新仓库在各环境里的检出补齐**。
///
/// 三个入口都会往 hub 里加仓库（「添加仓库」「启动页的常用仓库」「推荐仓库」），
/// 每一个加完都该补 —— 所以抽在这里，免得只修了一处、另外两处照旧缺。
///
/// 用 [`sync::ensure_env_worktrees`]（**只补建、不 reset**）：环境里可能躺着
/// 还没推送的合并成果，一个「顺手补齐」的动作不该把它抹掉。
///
/// 失败**不抛错** —— 仓库确实加进来了，环境没补上只是没补上，
/// 得单独报出来而不是让整件事看起来失败。
pub(crate) fn ensure_envs_for(
    hub: &gws_core::Hub,
    names: &[String],
    app_handle: &tauri::AppHandle,
) -> EnvEnsure {
    let mut out = EnvEnsure::default();
    if names.is_empty() {
        return out;
    }
    let cb = super::progress_cb(app_handle);
    let filter = ModuleFilter::only(names.to_vec());
    for e in hub.config().envs.clone() {
        match sync::ensure_env_worktrees(hub, &e.id, &filter, Some(&cb)) {
            Ok(o) => {
                out.created.extend(o.env_created);
                out.failures.extend(o.env_failures);
            }
            Err(err) => out.failures.push((e.id.clone(), err.to_string())),
        }
    }
    out
}

/// 添加仓库（clone 到 repos/）。慢操作，前端应显示进度。
///
/// # 为什么加完还要补环境
///
/// 实测反馈：加完仓库去环境页点「文件管理」没反应 —— 环境页列的是配置，
/// 目录要手动点「同步此环境」才建。用户以为那是 BUG（其实是少了一步），
/// 所以这里自动补：新增的仓库在每个环境里都缺一份检出，顺手建掉。
#[tauri::command(async)]
pub fn repo_add(
    urls: Vec<String>,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<RepoAddOutcome> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    let repos = repo::add(&hub, &urls, Some(&cb))?;

    let added: Vec<String> =
        repos.iter().filter(|r| r.outcome.is_ok()).map(|r| r.name.clone()).collect();
    let env = ensure_envs_for(&hub, &added, &app_handle);

    Ok(RepoAddOutcome { repos, env_created: env.created, env_failures: env.failures })
}

#[tauri::command]
pub fn repo_remove(names: Vec<String>, app: State<'_, App>) -> CmdResult<Vec<RepoOutcome>> {
    app.with_hub(|hub| repo::remove(hub, &names))
}

// ══ 环境 ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize)]
pub struct EnvRow {
    pub id: String,
    pub label: String,
    /// 仓库 → 分支名 的覆盖表。空表示全部同名。
    pub overrides: Vec<(String, String)>,
    /// 该环境下**实际检出**的分支名（按仓库），用于验证解耦是否生效（需求 #13）。
    pub effective: Vec<(String, String)>,
}

#[tauri::command]
pub fn env_list(app: State<'_, App>) -> CmdResult<Vec<EnvRow>> {
    app.with_hub(|hub| {
        let repos = repo::list(hub)?;
        Ok(hub
            .config()
            .envs
            .iter()
            .map(|e| EnvRow {
                id: e.id.clone(),
                label: e.label.clone(),
                overrides: e.branches.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                effective: repos
                    .iter()
                    .map(|r| (r.name.clone(), env_branch_for(hub, &e.id, &r.name)))
                    .collect(),
            })
            .collect())
    })
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct EnvAddInput {
    pub id: String,
    pub label: Option<String>,
    /// 分支名覆盖：`[仓库, 分支]` 或 `["*", 分支]`。
    #[serde(default)]
    pub overrides: Vec<(String, String)>,
}

/// 新建环境。建完**立刻把各仓库的 worktree 补齐**。
///
/// 实测反馈：环境页列的是配置，目录得手动点「同步此环境」才建 ——
/// 不点就一个子目录都没有，看着像 BUG。新建环境时环境里**必然是空的**，
/// 所以补的这一步没有任何风险（不会覆盖谁的东西），直接做掉。
///
/// 返回补齐的结果，让界面能说清「建了哪些、哪些仓库在这个环境没有对应分支」。
#[tauri::command(async)]
pub fn env_add(
    input: EnvAddInput,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<SyncOutcome> {
    let id = input.id.trim().to_string();
    if id.is_empty() {
        return Err(CommandError::plain("环境标识不能为空"));
    }
    // ① 先落配置 + 建目录（短锁，不在持有锁的时候跑 git）
    app.update_hub(|hub| {
        if hub.config().env(&id).is_some() {
            return Err(gws_core::Error::other(format!("环境 {id} 已存在")));
        }
        let mut e = EnvDef::new(&id, input.label.clone().unwrap_or_else(|| id.clone()));
        for (repo, branch) in &input.overrides {
            e.branches.insert(repo.clone(), branch.clone());
        }
        e.validate()?;
        hub.config_mut().envs.push(e);
        // 目录也要建出来，否则界面上的「打开目录」指向不存在的路径
        let dir = hub.env_dir(&id);
        std::fs::create_dir_all(&dir).map_err(|err| gws_core::Error::io(dir, err))?;
        Ok(())
    })?;
    // ② 再补 worktree。用 hub 的副本，不再占着那把锁（要跑几十秒）
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    Ok(sync::ensure_env_worktrees(&hub, &id, &ModuleFilter::all(), Some(&cb))?)
}

/// 改环境：标识（id）与显示名（label）。
///
/// 改标识会连带三件事，缺一样都会留下不一致：
/// 1. `envs/<旧标识>/` 目录改名（目录名就是标识）
/// 2. `deployments.json` 里按**环境标识**存的地址键迁到新标识（不迁就丢地址）
/// 3. 各需求 `.workspace.json` 的 `merged` 日期键（只是记录，但也别留孤儿键）
///
/// 目录用普通 `mv`：环境目录里放的是**各仓库的 worktree**，而 worktree 认的是
/// 主 clone 里的注册表（不是父目录名），所以改父目录名不影响它们（与文档库那种
/// 「worktree 本身就是被移动对象」不同）。
#[tauri::command]
pub fn env_update(old_id: String, new_id: String, label: String, app: State<'_, App>) -> CmdResult<()> {
    let new_id = new_id.trim().to_string();
    if new_id.is_empty() {
        return Err(CommandError::plain("环境标识不能为空"));
    }
    gws_core::name::validate_component(&new_id, "环境标识")?;
    let label = label.trim().to_string();
    let base = app.hub()?;

    if new_id != old_id {
        if base.config().envs.iter().any(|e| e.id == new_id) {
            return Err(CommandError::plain(format!("环境 {new_id} 已存在")));
        }
        let from = base.env_dir(&old_id);
        let to = base.env_dir(&new_id);
        if from.exists() {
            if to.exists() {
                return Err(CommandError::plain(format!("目录 {} 已存在", to.display())));
            }
            std::fs::rename(&from, &to).map_err(|e| {
                CommandError::plain(format!("环境目录改名失败：{e}"))
            })?;
            // 校验：改完必须真的在（改名失败却报成功是旧版最典型的毛病，§8 第 11 条）
            if !to.exists() || from.exists() {
                return Err(CommandError::plain("环境目录改名后校验未通过，已中止"));
            }
        }
        // 地址矩阵按环境标识存 —— 键要跟着迁
        let mut d = gws_core::deploy::Deployments::load(&base)?;
        d.rename_env(&old_id, &new_id);
        d.save(&base)?;
        // 各需求的 merged 记录
        for mut ws in gws_core::workspace::list(&base).unwrap_or_default() {
            let meta = ws.meta_mut();
            if let Some(v) = meta.merged.remove(&old_id) {
                meta.merged.insert(new_id.clone(), v);
                let _ = ws.save_meta();
            }
        }
    }

    app.update_hub(|hub| {
        if let Some(e) = hub.config_mut().envs.iter_mut().find(|e| e.id == old_id) {
            e.id = new_id.clone();
            e.label = label.clone();
        }
        Ok(())
    })
}

#[tauri::command]
pub fn env_remove(ids: Vec<String>, app: State<'_, App>) -> CmdResult<()> {
    for id in &ids {
        let dir = app.with_hub(|hub| Ok(hub.env_dir(id)))?;
        // 目录里有 worktree，先摘掉再删，否则 repos/ 里会留悬空引用
        app.with_hub(|hub| {
            for r in repo::list(hub)? {
                let wt = dir.join(&r.name);
                if wt.exists() {
                    let _ = r.git().worktree_remove(&wt, true);
                }
                let _ = r.git().worktree_prune();
            }
            Ok(())
        })?;
        if dir.exists() {
            gws_core::workspace::lifecycle::remove_dir_verified(&dir)?;
        }
        app.update_hub(|hub| {
            hub.config_mut().envs.retain(|e| &e.id != id);
            Ok(())
        })?;
    }
    Ok(())
}

/// 保存**前**校验一组覆盖规则的分支是否存在。
///
/// 填错分支名如果放过去，下次 `gws sync` 会把 worktree 切到不存在的分支上，
/// 那时候报错离现场很远、很难查（实测反馈）。
#[tauri::command(async)]
pub fn env_check_branches(
    overrides: Vec<(String, String)>,
    app: State<'_, App>,
) -> CmdResult<Vec<gws_core::workspace::sync::BranchCheck>> {
    let hub = app.hub()?;
    Ok(gws_core::workspace::sync::check_branches(&hub, &overrides)?)
}

/// 改环境的分支名覆盖表（需求 #13：换月份只改这里，目录名不动）。
#[tauri::command]
pub fn env_set_overrides(
    id: String,
    overrides: Vec<(String, String)>,
    app: State<'_, App>,
) -> CmdResult<()> {
    app.update_hub(|hub| {
        let e = hub
            .config_mut()
            .env_mut(&id)
            .ok_or_else(|| gws_core::Error::other(format!("环境 {id} 不存在")))?;
        e.branches = overrides.iter().cloned().collect();
        e.validate()
    })
}