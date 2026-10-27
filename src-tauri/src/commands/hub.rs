//! hub 的打开 / 新建 / 信息。

use serde::Serialize;
use tauri::State;
use gws_core::env::EnvDef;
use gws_core::hub::{GitIdentity, GitlabBindingInput, Hub, InitOptions, QuickLink};
use gws_core::repo;

use super::{bg, App, CmdResult, CommandError};

#[derive(Debug, Clone, Serialize)]
pub struct HubInfo {
    pub root: String,
    pub name: String,
    /// 环境标识与显示名。
    pub envs: Vec<(String, String)>,
    pub repo_count: usize,
    pub workspace_count: usize,
    /// 文档库在 hub 里的**目录名**（从配置的 URL 推导，如 `x-doc-hub`）。
    /// 前端别再写死 `doc-hub` —— 用户填什么名字就是什么名字。
    pub dochub_name: String,
    /// doc-hub 是否已克隆到本地。
    pub dochub_ready: bool,
    /// doc-hub 的主干分支（未克隆时为 None）。
    pub dochub_branch: Option<String>,
    /// 新建需求时自动拼接用的前缀与日期位数（6/8）。
    /// 界面预览必须用真实配置，不能写死 `feature` + 8 位。
    pub branch_prefix: String,
    pub branch_date_digits: u8,
    /// 打开时做过的目录迁移/清理说明（空 = 什么都没动）。
    /// 界面上给一条提示，别让人以为文件是自己少掉的。
    pub migration_notes: Vec<String>,
    /// 用户自己配的快捷入口。**空列表 = 界面整块隐藏快捷入口**。
    pub quick_links: Vec<QuickLink>,
    /// 这个 hub 的提交身份（两个都为空 = 用 git 全局配置）。
    pub git_name: String,
    pub git_email: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct InitInput {
    pub path: String,
    pub name: Option<String>,
    pub envs: Option<Vec<String>>,
    #[serde(default)]
    pub force: bool,
    /// 这个 hub 绑定的 GitLab 实例地址（**建 hub 时填，之后不可改**）。
    #[serde(default)]
    pub gitlab_url: Option<String>,
    /// GitLab Personal Access Token（需要 api 权限）。
    #[serde(default)]
    pub gitlab_token: Option<String>,
    /// 文档库仓库地址（**建 hub 时填，之后不可改**）。
    #[serde(default)]
    pub dochub_url: Option<String>,
    /// 提交身份（可留空 = 用 git 全局配置）。**建 hub 时填，之后随时可改。**
    #[serde(default)]
    pub git_name: Option<String>,
    #[serde(default)]
    pub git_email: Option<String>,
}

impl InitInput {
    /// 组装 GitLab 绑定。
    ///
    /// 三样要么都给、要么都不给（都不给 = 稍后用「设置 → GitLab」补绑）。
    /// 只给一部分是最容易出错的用法，直接报错比默默建一个半成品 hub 好。
    fn binding(&self) -> CmdResult<Option<GitlabBindingInput>> {
        let get = |v: &Option<String>| v.clone().unwrap_or_default().trim().to_string();
        let (url, token, doc) = (
            get(&self.gitlab_url),
            get(&self.gitlab_token),
            get(&self.dochub_url),
        );
        if url.is_empty() && token.is_empty() && doc.is_empty() {
            return Ok(None);
        }
        if url.is_empty() {
            return Err(CommandError::plain("请填写 GitLab 地址"));
        }
        if token.is_empty() {
            return Err(CommandError::plain("请填写 GitLab token（需要 api 权限）"));
        }
        if doc.is_empty() {
            return Err(CommandError::plain("请填写文档库地址（每个需求都要写文档）"));
        }
        Ok(Some(GitlabBindingInput {
            base_url: url,
            token,
            dochub_url: doc,
        }))
    }
}

/// 从某目录向上找 hub。用于「打开已有 hub」时自动补全。
#[tauri::command]
pub fn locate_hub(start: String) -> CmdResult<Option<String>> {
    match Hub::locate(std::path::Path::new(&start)) {
        Ok(p) => Ok(Some(p.to_string_lossy().to_string())),
        // 找不到不是错误：用户可能就是在选一个还没建 hub 的目录
        Err(_) => Ok(None),
    }
}

#[tauri::command]
pub fn open_hub(path: String, app: State<'_, App>) -> CmdResult<HubInfo> {
    let hub = Hub::open(std::path::Path::new(&path))?;
    // 已有的 hub 可能还没有 CLAUDE.md（早期版本建的）——补一份，
    // 否则 AI 进来不知道目录语义、也不知道文档该放哪
    // 缺**任一份**就都刷一遍（AGENTS.md 是给别的工具认的，见 AGENT_DOC_NAMES）
    let missing = gws_core::hub::AGENT_DOC_NAMES
        .iter()
        .any(|n| !hub.root().join(n).is_file());
    if missing {
        let _ = hub.write_claude_md();
    }
    let info = hub_info(&hub)?;
    app.set_hub(hub);
    Ok(info)
}

/// 新建 hub。**不预设任何仓库**：开源版没有内置仓库列表 ——
/// 每个人的仓库都不一样。建完由用户自己用 `repo add` 加需要的。
///
/// ⚠️ **必须 async + `bg`**：`Hub::init` 在建目录之前会真的连一次 GitLab
/// （`current_user` + 校验文档库）。Tauri 的同步命令跑在主线程上，内网 DNS 卡住
/// 时整个界面会冻住几十秒（详见设计方案 §8.1 第 17 条）。
#[tauri::command(async)]
pub async fn init_hub(input: InitInput, app: State<'_, App>) -> CmdResult<HubInfo> {
    let input2 = input.clone();
    let hub = bg(move || {
        if input2.path.trim().is_empty() {
            return Err(CommandError::plain("请先选择要建 hub 的目录"));
        }
        let opts = InitOptions {
            name: input2.name.clone().filter(|s| !s.trim().is_empty()),
            envs: input2
                .envs
                .clone()
                .unwrap_or_else(|| vec!["dev".into()])
                .into_iter()
                .map(|id| {
                    let label = env_label(&id);
                    EnvDef::new(id, label)
                })
                .collect(),
            allow_non_empty: input2.force,
            // 给了就一起绑定：core 会在建目录**之前**校验连通性，
            // 地址/token 不对时磁盘上不会留下任何东西
            gitlab: input2.binding()?,
            identity: GitIdentity {
                author_name: input2.git_name.clone().unwrap_or_default().trim().to_string(),
                author_email: input2.git_email.clone().unwrap_or_default().trim().to_string(),
            },
        };
        Hub::init(std::path::Path::new(&input2.path), opts)
            .map_err(|e| CommandError::plain(e.to_string()))
    })
    .await?;
    let info = hub_info(&hub)?;
    app.set_hub(hub);
    Ok(info)
}

/// 改 hub 的**显示名**。
///
/// 名字只是标签（存在 `gws.json` 的 `name`，全仓只有一处读它、只用于显示），
/// 不参与任何路径或身份判断 —— 所以改它安全。
///
/// 实测反馈：把 hub 目录改名/搬走之后，界面上的名字还是旧的，而**界面上
/// 原本没有改名入口**，只能手改 `gws.json`。这里补上入口。
#[tauri::command]
pub fn hub_rename(name: String, app: State<'_, App>) -> CmdResult<HubInfo> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(CommandError::plain("名字不能为空"));
    }
    // update_hub 的闭包只能返回 core 的 Error，所以这里只改配置，
    // 拿最新信息放到外面（hub_info 返回的是 CommandError）。
    app.update_hub(|hub| {
        hub.config_mut().name = name.clone();
        Ok(())
    })?;
    hub_info(&app.hub()?)
}

/// 设置「快捷入口」列表。
///
/// 这是**用户自己配的**——开源版不预设任何发布系统。传空列表是合法操作
/// （也是默认值），界面会整块隐藏快捷入口。
#[tauri::command]
pub fn hub_set_quick_links(
    links: Vec<QuickLink>,
    app: State<'_, App>,
) -> CmdResult<HubInfo> {
    app.update_hub(|hub| {
        hub.config_mut().quick_links = links.clone();
        Ok(())
    })?;
    hub_info(&app.hub()?)
}

/// 设置这个 hub 的**提交身份**（两个都留空 = 清除，回到 git 全局配置）。
///
/// 会同步写进 hub 里每个仓库的 local config —— hub 内的 worktree 与主 clone
/// 共享同一份 config，所以用户在 `ws/<分支>/` 里**手工 `git commit`** 用的
/// 也是这个身份。这就是「跟 git 全局配置解耦」的意义。
#[tauri::command]
pub fn hub_set_identity(name: String, email: String, app: State<'_, App>) -> CmdResult<HubInfo> {
    app.update_hub(|hub| {
        hub.set_identity(GitIdentity {
            author_name: name.trim().to_string(),
            author_email: email.trim().to_string(),
        })?;
        Ok(())
    })?;
    hub_info(&app.hub()?)
}

/// 读 git **全局**身份，给「新建 hub」的提交身份字段做预填。
///
/// 大多数人本来就配过全局身份，让他在建 hub 时再手打一遍是白费事。
/// 读不到就返回 `[null, null]`（不是错误 —— 没配全局身份很正常）。
#[tauri::command]
pub fn global_git_identity() -> (Option<String>, Option<String>) {
    match gws_core::git::global_identity() {
        Some(id) => (Some(id.author_name), Some(id.author_email)),
        None => (None, None),
    }
}

#[tauri::command]
pub fn hub_info_of(app: State<'_, App>) -> CmdResult<HubInfo> {
    hub_info(&app.hub()?)
}

#[tauri::command]
pub fn close_hub(app: State<'_, App>) {
    app.clear_hub();
}

fn hub_info(hub: &Hub) -> CmdResult<HubInfo> {
    Ok(HubInfo {
        root: hub.root().to_string_lossy().to_string(),
        name: hub.config().name.clone(),
        envs: hub
            .config()
            .envs
            .iter()
            .map(|e| (e.id.clone(), e.label.clone()))
            .collect(),
        repo_count: repo::list(hub)?.len(),
        workspace_count: gws_core::workspace::list(hub)?.len(),
        dochub_name: hub.dochub_name(),
        dochub_ready: hub.dochub_dir().join(".git").is_dir(),
        dochub_branch: if hub.dochub_dir().join(".git").is_dir() {
            gws_core::git::detect_main_branch(&hub.dochub_dir()).ok()
        } else {
            None
        },
        branch_prefix: hub.config().defaults.branch_prefix.clone(),
        branch_date_digits: hub.config().defaults.branch_date_digits,
        migration_notes: {
            let m = hub.migration();
            let mut notes: Vec<String> = m
                .moved
                .iter()
                .map(|(from, to)| format!("目录已迁移：{from} → {to}"))
                .collect();
            notes.extend(
                m.pruned_modules
                    .iter()
                    .map(|(b, mm)| format!("已清掉残留的模块登记：{b} / {mm}（文档不再算模块）")),
            );
            notes.extend(m.failed.iter().map(|f| format!("⚠ {f}")));
            // worktree 链接的修复（搬动 hub 造成）也走这条「我动了什么」通道
            notes.extend(hub.repairs().iter().cloned());
            notes
        },
        quick_links: hub.config().quick_links.clone(),
        git_name: hub.config().git.author_name.clone(),
        git_email: hub.config().git.author_email.clone(),
    })
}

fn env_label(id: &str) -> String {
    match id {
        "dev" => "开发",
        "pre" => "预发",
        "prod" => "生产",
        "test" => "测试",
        other => other,
    }
    .to_string()
}