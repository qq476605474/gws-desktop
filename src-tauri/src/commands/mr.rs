//! Merge Request。

use serde::Serialize;
use tauri::State;
use gws_core::gitlab::Client;
use gws_core::workspace::mr::{
    self, CloseOutcome, MrOptions, MrPlan, ReviewerOverride, Reviewers,
};
use gws_core::workspace::{ModuleFilter, Workspace};

use super::{bg, App, CmdResult, CommandError, FilterInput};

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct MrInput {
    pub branch: String,
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub reviewer: Option<String>,
    #[serde(default)]
    pub frontend_reviewer: Option<String>,
    #[serde(default)]
    pub backend_reviewer: Option<String>,
    #[serde(flatten)]
    pub filter: FilterInput,
}

impl MrInput {
    fn to_options(&self) -> MrOptions {
        MrOptions {
            title: self.title.clone(),
            description: self.description.clone(),
            filter: self.filter.clone().into(),
            reviewers: ReviewerOverride {
                all: self.reviewer.clone(),
                frontend: self.frontend_reviewer.clone(),
                backend: self.backend_reviewer.clone(),
            },
        }
    }
}

/// 弹窗预填用的默认标题/描述（纯本地，不联网）。
#[tauri::command]
pub fn mr_defaults(branch: String, app: State<'_, App>) -> CmdResult<mr::MrDefaults> {
    app.with_hub(|hub| {
        let ws = Workspace::open(hub, &branch)?;
        Ok(mr::defaults(&ws))
    })
}

/// MR 预览（dry-run，不写任何东西）。
///
/// dry-run 也**要联网**：解析评审人得查用户、探测目标分支得查项目。
/// 所以走 `bg`——同步命令跑在主线程上，这一下能把界面冻住几十秒。
#[tauri::command(async)]
pub async fn mr_plan(input: MrInput, app: State<'_, App>) -> CmdResult<MrPlan> {
    let hub = app.hub()?;
    bg(move || {
        let client = Client::for_hub(&hub).map_err(|e| CommandError::plain(e.to_string()))?;
        let mut ws = Workspace::open(&hub, &input.branch)?;
        Ok(mr::run(
            &hub,
            &mut ws,
            &client,
            &input.to_options(),
            false,
            None,
        )?)
    })
    .await
}

/// 真正创建 MR。**会联网**（创建 MR / 建分支），必须走 `bg`，理由见 `bg` 文档。
#[tauri::command(async)]
pub async fn mr_create(
    input: MrInput,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<MrPlan> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    bg(move || {
        let client = Client::for_hub(&hub).map_err(|e| CommandError::plain(e.to_string()))?;
        let mut ws = Workspace::open(&hub, &input.branch)?;
        Ok(mr::run(
            &hub,
            &mut ws,
            &client,
            &input.to_options(),
            true,
            Some(&cb),
        )?)
    })
    .await
}

/// 关闭本需求的 MR。**只关闭，不碰分支**（需求 #1）。
#[tauri::command(async)]
pub async fn mr_close(
    branch: String,
    filter: FilterInput,
    app: State<'_, App>,
) -> CmdResult<CloseOutcome> {
    let hub = app.hub()?;
    bg(move || {
        let client = Client::for_hub(&hub).map_err(|e| CommandError::plain(e.to_string()))?;
        let mut ws = Workspace::open(&hub, &branch)?;
        let f: ModuleFilter = filter.into();
        Ok(mr::close(&hub, &mut ws, &client, &f)?)
    })
    .await
}

/// 当前生效的评审人（含来源链），供界面在弹窗里预填。
///
/// 开源版**没有内置默认评审人**，因此这里不再带「显示名」字段 —— 编译期无从
/// 知道你的评审人叫什么。界面直接显示 username；要中文名得自己联网搜。
#[derive(Debug, Clone, Serialize)]
pub struct ReviewerInfo {
    pub frontend: String,
    pub backend: String,
    pub source: String,
}

#[tauri::command]
pub fn mr_reviewers(branch: Option<String>, app: State<'_, App>) -> CmdResult<ReviewerInfo> {
    app.with_hub(|hub| {
        let ws = branch
            .as_deref()
            .filter(|b| !b.is_empty())
            .and_then(|b| Workspace::open(hub, b).ok());
        let Reviewers {
            frontend,
            backend,
            source,
        } = mr::resolve_reviewers(ws.as_ref(), &ReviewerOverride::default());
        Ok(ReviewerInfo { frontend, backend, source })
    })
}

/// 把评审人配置写到需求级 `.gws-mr.json`（下次打开弹窗就是这个值）。
#[tauri::command]
pub fn mr_save_reviewers(
    branch: String,
    front: Option<String>,
    back: Option<String>,
    app: State<'_, App>,
) -> CmdResult<()> {
    app.with_hub(|hub| {
        let ws = Workspace::open(hub, &branch)?;
        let path = ws.dir().join(".gws-mr.json");
        let mut v = serde_json::Map::new();
        if let Some(f) = front.filter(|s| !s.trim().is_empty()) {
            v.insert("frontend".into(), serde_json::Value::String(f));
        }
        if let Some(b) = back.filter(|s| !s.trim().is_empty()) {
            v.insert("backend".into(), serde_json::Value::String(b));
        }
        let text = serde_json::to_string_pretty(&serde_json::Value::Object(v))
            .map_err(|e| gws_core::Error::other(format!("序列化失败: {e}")))?;
        std::fs::write(&path, format!("{text}\n")).map_err(|e| gws_core::Error::io(path, e))
    })
}