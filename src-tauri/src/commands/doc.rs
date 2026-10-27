//! 需求文档（doc-hub）。

use serde::Serialize;
use tauri::State;
use gws_core::dochub::{DocHub, Family, PeriodMeta, DOC_TYPES};
use gws_core::workspace::Workspace;

use super::{bg, App, CmdResult, CommandError};

#[derive(Debug, Clone, Serialize)]
pub struct DocFile {
    pub name: String,
    /// 相对期次目录的路径（用作显示与去重键）。
    pub rel: String,
    pub abs: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocView {
    pub meta: PeriodMeta,
    /// 期次目录的绝对路径。
    pub dir: String,
    pub files: Vec<DocFile>,
    /// 待提交的文档变更。
    pub pending: Vec<String>,
    /// **空目录**（相对期次目录，如 `sql`）。
    ///
    /// 只靠 `files` 画不出它们 —— `sql/` 这种约定好的目录，刚建出来时是空的，
    /// 界面上却看不见（实测反馈）。而它恰恰是「把 SQL 放这儿」的提示。
    pub empty_dirs: Vec<String>,
    /// 期次里有没有真正的文档（只有脚手架时为 `false`）。
    pub has_content: bool,
}

#[tauri::command]
pub fn doc_view(branch: String, app: State<'_, App>) -> CmdResult<DocView> {
    app.with_hub(|hub| {
        let dh = DocHub::ensure_local(hub)?;
        let ws = Workspace::open(hub, &branch)?;
        let dir = dh.period_dir(&ws);
        let meta = PeriodMeta::load(&dir.join("meta.json"))?;
        let pending = dh.status(&ws).unwrap_or_default();

        let mut files = Vec::new();
        collect(&dir, &dir, &mut files)?;
        files.sort_by(|a, b| a.rel.cmp(&b.rel));
        let mut empty_dirs = Vec::new();
        collect_empty_dirs(&dir, &dir, &mut empty_dirs);
        empty_dirs.sort();
        let has_content = gws_core::dochub::period_has_content(&dir);

        Ok(DocView {
            meta,
            dir: dir.to_string_lossy().to_string(),
            files,
            pending,
            empty_dirs,
            has_content,
        })
    })
}

/// 这个目录（含子孙）里有没有**真文件**（系统垃圾不算）。
fn has_file_under(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".DS_Store" || name.starts_with("._") {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            if has_file_under(&p) {
                return true;
            }
        } else {
            return true;
        }
    }
    false
}

/// 相对路径**一律用 `/` 分隔**（把 Windows 的 `\` 换掉）。
///
/// # 为什么必须统一
///
/// 前端拿 `rel` 拼目录树（`nodesFromFiles` 按 `/` 切分），也拿它显示。
/// Windows 上 `Path::to_string_lossy()` 给的是 `原型\index.html` —— 切不开，
/// 于是「需求相关文档」里**整份文档平铺成一行**、还带着反斜杠（实测反馈：
/// 「Windows 包里点需求相关文档，没有树形结构」）。mac/Linux 上是 `/`，所以
/// 这个 bug 只在 Windows 上现形。
///
/// 做成**吃 `&str`** 而不是吃 `&Path`：这样在 mac 上也能单测（喂一对 Windows
/// 形态的字符串即可），不必等人在 Windows 上撞见。
pub(crate) fn to_slash(s: &str) -> String {
    s.replace('\\', "/")
}

/// 收集**空的**目录（相对 root 的路径）。只收最外层的空目录，不往它里面再走。
fn collect_empty_dirs(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        if has_file_under(&p) {
            collect_empty_dirs(root, &p, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(to_slash(&rel.to_string_lossy()));
        }
    }
}

fn collect(
    root: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<DocFile>,
) -> gws_core::Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(DocFile {
                name: entry.file_name().to_string_lossy().to_string(),
                rel: to_slash(&rel.to_string_lossy()),
                abs: path.to_string_lossy().to_string(),
            });
        }
    }
    Ok(())
}

/// 挂文档目录（新建需求时已自动挂，这里是手动补挂入口）。
#[tauri::command]
pub fn doc_attach(
    branch: String,
    family: Option<String>,
    app: State<'_, App>,
) -> CmdResult<gws_core::dochub::AttachOutcome> {
    let hub = app.hub()?;
    let dh = DocHub::ensure_local(&hub)?;
    let mut ws = Workspace::open(&hub, &branch)?;
    Ok(dh.attach(&mut ws, family.as_deref())?)
}

/// 把文档从需求上摘掉（简单需求不需要文档时）。**不影响 doc-hub 上已有的内容**。
#[tauri::command]
pub fn doc_detach(branch: String, app: State<'_, App>) -> CmdResult<bool> {
    let hub = app.hub()?;
    let dh = DocHub::ensure_local(&hub)?;
    let mut ws = Workspace::open(&hub, &branch)?;
    Ok(dh.detach(&mut ws)?)
}

/// 可选的文档类型（界面用来画「新建文档」的按钮/下拉）。
#[tauri::command]
pub fn doc_types() -> Vec<DocTypeView> {
    DOC_TYPES
        .iter()
        .map(|t| DocTypeView {
            key: t.key.to_string(),
            file: t.file.to_string(),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct DocTypeView {
    pub key: String,
    pub file: String,
}

#[tauri::command]
pub fn doc_new(branch: String, kind: String, app: State<'_, App>) -> CmdResult<String> {
    let hub = app.hub()?;
    let dh = DocHub::ensure_local(&hub)?;
    let ws = Workspace::open(&hub, &branch)?;
    Ok(dh.new_doc(&ws, &kind)?.to_string_lossy().to_string())
}

/// 文档分支相对 doc-hub 主干的领先提交数（界面「vs主干」那一列）。
#[tauri::command(async)]
pub async fn doc_ahead_of_main(branch: String, app: State<'_, App>) -> CmdResult<u64> {
    let hub = app.hub()?;
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        let dh = DocHub::ensure_local(&hub)?;
        Ok(dh.ahead_of_main(&ws)?)
    })
    .await
}

#[tauri::command]
pub fn doc_status(branch: String, app: State<'_, App>) -> CmdResult<Vec<String>> {
    app.with_hub(|hub| {
        let dh = DocHub::ensure_local(hub)?;
        let ws = Workspace::open(hub, &branch)?;
        dh.status(&ws)
    })
}

/// 提交文档；`push=true` 推到远程文档分支。
///
/// 与「AI 只 add 不 commit」的约定不冲突：这是**人显式点按钮**触发的工具行为。
#[tauri::command(async)]
pub fn doc_sync(
    branch: String,
    message: Option<String>,
    push: bool,
    app: State<'_, App>,
) -> CmdResult<gws_core::dochub::DocSyncOutcome> {
    let hub = app.hub()?;
    let dh = DocHub::ensure_local(&hub)?;
    let ws = Workspace::open(&hub, &branch)?;
    Ok(dh.sync(&ws, message.as_deref().unwrap_or(""), push)?)
}

/// 改期次标签（V1 / V2 / BUGFIX …）。自动编号猜不出人的意图，所以允许改。
#[tauri::command]
pub fn doc_set_label(branch: String, label: String, app: State<'_, App>) -> CmdResult<String> {
    app.with_hub(|hub| {
        let ws = Workspace::open(hub, &branch)?;
        let dh = DocHub::ensure_local(hub)?;
        dh.set_label(&ws, &label)
    })
}

/// 所有需求分类与期次（读历史文档用，需求 #17）。
#[tauri::command(async)]
pub fn doc_families(app: State<'_, App>) -> CmdResult<Vec<Family>> {
    let hub = app.hub()?;
    Ok(DocHub::ensure_local(&hub)?.families()?)
}

/// 按需克隆 doc-hub（首次会比较慢）。`families`/`doc_view` 内部也会触发，
/// 单独暴露出来是为了让界面上有个明确的「准备文档库」按钮。
///
/// 文档库拉下来之后**它在 `repos/` 下就是一个普通仓库**，所以各环境里也一样
/// 缺一份检出 —— 顺手补上，否则环境页那行「文档」会一直显示「未检出」。
#[tauri::command(async)]
pub fn dochub_ensure(
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<super::repo::RepoAddOutcome> {
    let hub = app.hub()?;
    DocHub::ensure(&hub).map_err(|e| CommandError::plain(e.to_string()))?;
    let env = super::repo::ensure_envs_for(
        &hub,
        &[hub.dochub_name()],
        &app_handle,
    );
    Ok(super::repo::RepoAddOutcome {
        repos: Vec::new(),
        env_created: env.created,
        env_failures: env.failures,
    })
}