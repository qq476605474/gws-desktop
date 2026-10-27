//! 发布地址矩阵（需求 #7）。

use serde::Serialize;
use tauri::State;
use gws_core::deploy::{Deployments, EnvAddress, MatrixRow};

use super::{bg, App, CmdResult, CommandError};

/// 全部仓库 × 全部环境的地址矩阵。界面拿它画表格，人只填空格子。
#[tauri::command]
pub fn deploy_matrix(app: State<'_, App>) -> CmdResult<Vec<MatrixRow>> {
    app.with_hub(gws_core::deploy::matrix)
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AddressInput {
    pub repo: String,
    pub env: String,
    pub url: String,
    pub domain: String,
}

/// 保存一个格子的地址。**保存前校验 URL 格式**——填错的链接点开是空白页，
/// 比直接拦住更让人困惑。
#[tauri::command]
pub fn deploy_set_address(input: AddressInput, app: State<'_, App>) -> CmdResult<()> {
    validate_url(&input.url)?;
    app.with_hub(|hub| {
        let mut d = Deployments::load(hub)?;
        d.set_address(
            &input.repo,
            &input.env,
            EnvAddress { url: input.url.trim().to_string(), domain: input.domain.trim().to_string() },
        );
        d.save(hub)
    })
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RepoInfoInput {
    pub repo: String,
    #[serde(default)]
    pub notes: Option<String>,
    /// 给了就整体替换（不是追加）——界面拿的是完整列表，语义更清楚。
    #[serde(default)]
    pub links: Option<Vec<gws_core::workspace::Link>>,
}

/// 保存仓库的自定义备注 / 链接（实测反馈 #5：tapd、飞书、域名这类信息）。
#[tauri::command]
pub fn deploy_set_repo_info(input: RepoInfoInput, app: State<'_, App>) -> CmdResult<()> {
    if let Some(links) = &input.links {
        for l in links {
            if !l.url.trim().is_empty() {
                validate_url(&l.url)?;
            }
        }
    }
    app.with_hub(|hub| {
        let mut d = Deployments::load(hub)?;
        if let Some(notes) = &input.notes {
            d.set_notes(&input.repo, notes.clone());
        }
        if let Some(links) = input.links.clone() {
            d.set_links(&input.repo, links);
        }
        d.save(hub)
    })
}

/// 保存某仓库的 GitLab 页地址。
#[tauri::command]
pub fn deploy_set_gitlab(repo: String, url: String, app: State<'_, App>) -> CmdResult<()> {
    if !url.trim().is_empty() {
        validate_url(&url)?;
    }
    app.with_hub(|hub| {
        let mut d = Deployments::load(hub)?;
        d.set_gitlab(&repo, url.trim().to_string());
        d.save(hub)
    })
}

/// 一次提交多个格子（界面上整行/整列编辑时用）。
#[tauri::command]
pub fn deploy_set_many(inputs: Vec<AddressInput>, app: State<'_, App>) -> CmdResult<()> {
    for i in &inputs {
        validate_url(&i.url)?;
    }
    app.with_hub(|hub| {
        let mut d = Deployments::load(hub)?;
        for i in &inputs {
            d.set_address(
                &i.repo,
                &i.env,
                EnvAddress {
                    url: i.url.trim().to_string(),
                    domain: i.domain.trim().to_string(),
                },
            );
        }
        d.save(hub)
    })
}

fn validate_url(url: &str) -> CmdResult<()> {
    let u = url.trim();
    if u.is_empty() {
        return Ok(()); // 空 = 清空这一格，合法
    }
    if !u.starts_with("https://") && !u.starts_with("http://") {
        return Err(CommandError::plain(format!("地址要以 http:// 或 https:// 开头：{u}")));
    }
    Ok(())
}


// ══ 自更新 ════════════════════════════════════════════════════════════

#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateInfo {
    pub current: String,
    /// `up_to_date` | `available` | `ahead`
    pub state: String,
    pub latest: Option<String>,
    pub release_url: String,
    /// Releases 页地址（无论检查结果如何都有，前端可以直接跳）。
    pub releases_url: String,
    /// 当前平台对应的资产名（让它能看到自己该下哪个）。
    pub asset: String,
}

/// 检查更新。**不下载任何东西**，也不需要 token（Release 是公开的）。
///
/// 要联网，所以走 `bg`——见 `bg` 文档（这里原来是 `#[tauri::command(async)]`
/// 直接加在同步函数上，`tauri dev` 下必 panic，界面「检查更新」永远转圈）。
#[tauri::command(async)]
pub async fn check_update() -> CmdResult<UpdateInfo> {
    bg(check).await
}

fn check() -> CmdResult<UpdateInfo> {
    use gws_core::update;

    let current = env!("CARGO_PKG_VERSION");
    // 自更新走 GitHub，与 hub 绑定的 GitLab 无关，所以不需要客户端
    let check = update::check(current).map_err(|e| CommandError::plain(e.to_string()))?;
    let (state, latest, url) = match check.status {
        update::UpdateStatus::UpToDate => ("up_to_date", None, String::new()),
        // web_url 由 GitHub 直接给出，是完整地址；给空值时前端退回 releases_url
        update::UpdateStatus::Available(r) => ("available", Some(r.version), r.web_url),
        update::UpdateStatus::Ahead(r) => ("ahead", Some(r.version), r.web_url),
    };
    Ok(UpdateInfo {
        current: current.to_string(),
        state: state.to_string(),
        latest,
        release_url: url,
        releases_url: update::releases_url(),
        asset: update::asset_name(),
    })
}

/// 下载并安装桌面端更新（**一键更新**）。
///
/// 慢（下载几 MB + 装），所以带进度、走 `bg`。装完根据 `next` 告诉前端：
/// - `Restart`：已替换好，前端调 `restart_app` 退出，重启助手会把它拉起来
/// - `Manual`：Linux 的 deb 装不了（要 sudo），把包路径和命令交给用户
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInstallOutcome {
    pub version: String,
    /// `restart` | `manual`
    pub next: String,
    pub package: String,
    pub message: String,
    /// 需要人工执行时要敲的命令（Linux）。
    pub command: Option<String>,
}

#[tauri::command(async)]
pub async fn update_install(app_handle: tauri::AppHandle) -> CmdResult<UpdateInstallOutcome> {
    let cb = super::progress_cb(&app_handle);
    bg(move || {
        use gws_core::update;

        let current = env!("CARGO_PKG_VERSION");
        let check = update::check(current).map_err(|e| CommandError::plain(e.to_string()))?;
        let update::UpdateStatus::Available(rel) = check.status else {
            return Err(CommandError::plain("当前已是最新版本，无需更新"));
        };

        let out = update::install_desktop(&rel, Some(&cb))
            .map_err(|e| CommandError::plain(e.to_string()))?;
        Ok(UpdateInstallOutcome {
            version: out.version,
            next: match out.next {
                update::InstallNext::Restart => "restart".to_string(),
                update::InstallNext::Manual => "manual".to_string(),
            },
            package: out.package.to_string_lossy().to_string(),
            message: out.message,
            command: out.command,
        })
    })
    .await
}

/// 装好新版本之后**退出应用**，让重启助手把它拉起来。
///
/// 只允许在刚装完之后调 —— 所以 `update_install` 返回 `restart` 时前端才调它。
#[tauri::command]
pub fn restart_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// 打开 Releases 页（更新交给用户手工做——桌面端替换自己的 app bundle
/// 比替换单个 CLI 二进制复杂得多，Tauri 有专门的 updater 插件，
/// 但需要签名密钥，见设计方案 §11.5 未决问题）。
#[tauri::command]
pub fn open_releases() -> CmdResult<()> {
    // 地址由 core 给：**别在这里自己拼**，之前少拼了项目路径，
    // 点开是 `https://gitlab.example.com/-/releases` → 404。
    // 打开动作复用 platform::open_url（三平台分别处理过，别再自己 spawn）。
    crate::platform::open_url(gws_core::update::releases_url())
        .map_err(CommandError::plain)
}

// ══ 仓库配置的复制 / 粘贴（实测反馈：仓库也一键复制给别人）══════════════

/// 把某仓库的配置渲染成一段可粘贴的文本（发给别人）。
#[tauri::command]
pub fn repo_share(repo: String, app: State<'_, App>) -> CmdResult<String> {
    app.with_hub(|hub| Ok(gws_core::deploy::share_repo(hub, &repo)?.render()))
}

/// 解析别人发来的仓库配置（纯本地，不动任何东西）。
#[tauri::command]
pub fn repo_share_parse(
    text: String,
) -> CmdResult<gws_core::deploy::SharedRepo> {
    Ok(gws_core::deploy::parse_repo(&text)?)
}

/// 导入结果：仓库本身 + 补上的自定义信息 + 环境检出。
#[derive(Debug, Clone, Serialize)]
pub struct RepoImportResult {
    pub name: String,
    /// 是不是新克隆的。
    pub added: bool,
    pub info_applied: usize,
    /// 本地已有、**没有覆盖**的项。
    pub kept_local: Vec<String>,
    /// 本地没有的环境（地址跳过了）。
    pub skipped_envs: Vec<String>,
    pub env_created: Vec<(String, String, String)>,
    pub env_failures: Vec<(String, String)>,
}

/// 按别人发来的配置把仓库导进来（缺就克隆，自定义信息只补空、不覆盖）。
#[tauri::command(async)]
pub async fn repo_share_import(
    shared: gws_core::deploy::SharedRepo,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<RepoImportResult> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    let handle = app_handle.clone();
    bg(move || {
        // 界面递过来的也过同一套校验
        gws_core::deploy::validate_repo(&shared)?;
        let out = gws_core::deploy::import_repo(&hub, &shared, Some(&cb))?;
        // 新克隆的仓库在每个环境里都缺一份检出 —— 顺手补上
        let names = if out.added { vec![out.name.clone()] } else { Vec::new() };
        let env = super::repo::ensure_envs_for(&hub, &names, &handle);
        Ok(RepoImportResult {
            name: out.name,
            added: out.added,
            info_applied: out.info_applied,
            kept_local: out.kept_local,
            skipped_envs: out.skipped_envs,
            env_created: env.created,
            env_failures: env.failures,
        })
    })
    .await
}
