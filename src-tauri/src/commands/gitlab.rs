//! GitLab 凭据与检索。
//!
//! # 凭据跟着 hub 走
//!
//! 地址来自当前 hub 的 `gws.json`（建 hub 时固定、不可改），token 存在该 hub
//! 目录下的 `gitlab-token`。所以这些命令都先取 hub 再构造客户端 ——
//! **没有「全局 GitLab 配置」这回事**。

use serde::Serialize;
use tauri::State;
use gws_core::gitlab::{Client, Project, User};
use gws_core::hub::GitlabBindingInput;

use super::{bg, App, CmdResult, CommandError};

#[derive(Debug, Clone, Serialize)]
pub struct GitlabStatus {
    /// 当前 hub 绑定的地址（没打开 hub、或还没绑定时为 `""`）。
    pub base_url: String,
    /// 是否已经绑定过 GitLab（老 hub / 离线创建的可能没绑）。
    pub bound: bool,
    /// **有没有打开 hub**。没打开时界面不该显示一份「全局 GitLab 配置」，
    /// 而要提示先打开 hub。
    pub has_hub: bool,
    /// 凭据来源（环境变量 / hub 内配置文件）。
    pub source: String,
    pub has_stored_token: bool,
    /// 凭据文件位置（界面告诉用户存哪儿了）。
    pub store_path: String,
    /// 连通时的当前用户。
    pub user: Option<(String, String)>,
    /// 失败原因，**逐层保留**——「没绑地址」「没配 token」「连不上」要给不同提示。
    pub error: Option<String>,
}

/// 状态查询要打网络（拿当前用户），所以走 `bg`。
///
/// 这里原来直接写成 `#[tauri::command(async)]` 的同步函数，结果在 `tauri dev`
/// 下必 panic、界面永远转圈——原因见 `bg` 的文档。**所有会联网的命令都必须这样写。**
#[tauri::command(async)]
pub async fn gitlab_status(app: State<'_, App>) -> CmdResult<GitlabStatus> {
    // 没打开 hub 时不该报错 —— 启动页 / 设置页都可能问一句状态
    status_impl(app.hub().ok()).await
}

/// 命令体。抽出来是为了能在测试里不依赖 Tauri 的 `State` 直接跑一遍
/// —— `State` 没法在单测里构造，但「联网命令必须能在 async 运行时里跑通」
/// 这条必须继续钉住（见 `commands/tests.rs` 的模块文档）。
pub(crate) async fn status_impl(hub: Option<gws_core::hub::Hub>) -> CmdResult<GitlabStatus> {
    bg(move || Ok(status_of(hub.as_ref()))).await
}

fn status_of(hub: Option<&gws_core::hub::Hub>) -> GitlabStatus {
    let Some(hub) = hub else {
        return GitlabStatus {
            base_url: String::new(),
            bound: false,
            has_hub: false,
            source: "未打开 hub".to_string(),
            has_stored_token: false,
            store_path: String::new(),
            user: None,
            error: None,
        };
    };

    let base_url = hub.config().gitlab.base_url.clone();
    let bound = hub.config().gitlab.is_bound();
    let (user, error) = match Client::for_hub(hub) {
        Ok(c) => match c.current_user() {
            Ok(u) => (Some((u.name, u.username)), None),
            Err(e) => (None, Some(e.to_string())),
        },
        Err(e) => (None, Some(e.to_string())),
    };

    GitlabStatus {
        base_url,
        bound,
        has_hub: true,
        source: gws_core::gitlab::token::source(hub.root()).to_string(),
        has_stored_token: gws_core::gitlab::token::has_stored(hub.root()),
        store_path: gws_core::gitlab::token::store_path(hub.root()),
        user,
        error,
    }
}

/// 用给定的地址 + token + 文档库地址试连一次（建 hub 之前先验证）。
///
/// **三项一起验**：地址与 token 决定能不能连上，文档库决定它到底存不存在。
/// 只验前两项的话，用户会看到「连接成功」，然后在创建时才发现文档库填错了 ——
/// 而文档库地址落盘后不可改，那个 hub 就废了。
///
/// 成功返回 `(显示名, username)`。
#[tauri::command(async)]
pub async fn gitlab_probe(
    base_url: String,
    token: String,
    dochub_url: Option<String>,
) -> CmdResult<(String, String)> {
    bg(move || {
        let host = gws_core::gitlab::remote_host(&base_url).to_string();
        let c = Client::new(base_url, token).map_err(|e| CommandError::plain(e.to_string()))?;
        let u = c.current_user().map_err(|e| CommandError::plain(e.to_string()))?;
        // 文档库给了就一并校验（同域名 + 真的存在 + 有权限）
        if let Some(doc) = dochub_url.filter(|d| !d.trim().is_empty()) {
            gws_core::hub::validate_dochub(&c, &host, &doc)
                .map_err(|e| CommandError::plain(e.to_string()))?;
        }
        Ok((u.name, u.username))
    })
    .await
}

/// 给**已存在但还没绑定**的 hub 补绑 GitLab（老 hub 迁移用）。
///
/// 已经绑定过的会被 core 拒绝 —— 地址不可改。
#[tauri::command(async)]
pub async fn hub_bind_gitlab(
    url: String,
    doc: String,
    token: String,
    app: State<'_, App>,
) -> CmdResult<()> {
    let mut hub = app.hub()?;
    // 在后台线程里绑定，**回到主线程再写回共享状态** —— `State` 的借用
    // 出不了这个函数体，不能直接在闭包里用它。
    let bound = bg(move || {
        hub.bind_gitlab(&GitlabBindingInput {
            base_url: url,
            token,
            dochub_url: doc,
        })
        .map_err(|e| CommandError::plain(e.to_string()))?;
        Ok(hub)
    })
    .await?;
    app.set_hub(bound);
    Ok(())
}

/// 只把 token 存进**当前 hub** 的凭据文件，**不验证**。
///
/// 实测反馈：原来保存时会顺带验证，而验证要打网络——网络一慢就卡在
/// 「正在验证并保存」好几分钟，用户以为程序死了。
///
/// 存就存，用户填什么就是什么；要验证走「重新检测」。真正提 MR 时如果 token
/// 不对，那一步本来就会报错，那时候再来查也来得及。
#[tauri::command]
pub fn gitlab_save_token(value: String, app: State<'_, App>) -> CmdResult<()> {
    let hub = app.hub()?;
    let t = value.trim();
    if t.is_empty() {
        return Err(CommandError::plain("token 不能为空"));
    }
    gws_core::gitlab::token::store(hub.root(), t).map_err(|e| CommandError::plain(e.to_string()))
}

/// 存 token 并验证。
#[tauri::command(async)]
pub async fn gitlab_login(value: String, app: State<'_, App>) -> CmdResult<GitlabStatus> {
    let hub = app.hub()?;
    bg(move || {
        gws_core::gitlab::token::store(hub.root(), value.trim())
            .map_err(|e| CommandError::plain(e.to_string()))?;
        Ok(status_of(Some(&hub)))
    })
    .await
}

#[tauri::command(async)]
pub async fn gitlab_logout(app: State<'_, App>) -> CmdResult<GitlabStatus> {
    let hub = app.hub()?;
    bg(move || {
        gws_core::gitlab::token::clear(hub.root())
            .map_err(|e| CommandError::plain(e.to_string()))?;
        Ok(status_of(Some(&hub)))
    })
    .await
}

/// 检索**当前用户有权限**的仓库（不用逐个粘贴地址）。
#[tauri::command(async)]
pub async fn gitlab_search_projects(
    query: String,
    limit: Option<usize>,
    app: State<'_, App>,
) -> CmdResult<Vec<Project>> {
    let hub = app.hub()?;
    bg(move || {
        let c = Client::for_hub(&hub).map_err(|e| CommandError::plain(e.to_string()))?;
        c.search_projects(&query, limit.unwrap_or(50))
            .map_err(|e| CommandError::plain(e.to_string()))
    })
    .await
}

/// 搜用户（评审人下拉的候选）。
#[tauri::command(async)]
pub async fn gitlab_search_users(
    query: String,
    limit: Option<usize>,
    app: State<'_, App>,
) -> CmdResult<Vec<User>> {
    let hub = app.hub()?;
    bg(move || {
        let c = Client::for_hub(&hub).map_err(|e| CommandError::plain(e.to_string()))?;
        c.search_users(&query, limit.unwrap_or(20))
            .map_err(|e| CommandError::plain(e.to_string()))
    })
    .await
}

/// 把选中的 GitLab 项目 clone 进 hub。
#[tauri::command(async)]
pub fn gitlab_clone_projects(
    urls: Vec<String>,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<super::repo::RepoAddOutcome> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    let repos = gws_core::repo::add(&hub, &urls, Some(&cb))?;
    // 同 `repo_add`：新加进来的仓库在每个环境里都缺一份检出，顺手补上，
    // 否则环境页又是一片「点了没反应」
    let added: Vec<String> =
        repos.iter().filter(|r| r.outcome.is_ok()).map(|r| r.name.clone()).collect();
    let env = super::repo::ensure_envs_for(&hub, &added, &app_handle);
    Ok(super::repo::RepoAddOutcome { repos, env_created: env.created, env_failures: env.failures })
}