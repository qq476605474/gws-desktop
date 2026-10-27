//! 需求工作区。

use serde::Serialize;
use tauri::State;
use gws_core::repo::{Outcome, RepoOutcome};
use gws_core::workspace::lifecycle::{
    self, CleanOutcome, DropMissingOutcome, GetOptions, NewOptions, RemoveOutcome,
};
use gws_core::workspace::{sync, ModuleFilter, Workspace, WorkspaceMeta};

use gws_core::dochub::AttachMode;

use super::{bg, App, CmdResult, CommandError, FilterInput};

#[derive(Debug, Clone, Serialize)]
pub struct WsRow {
    pub branch: String,
    pub title: String,
    pub modules: usize,
    /// 元数据登记但目录已不在的模块数（>0 说明需要 `gws drop` 清理）。
    pub missing_modules: usize,
    pub created: String,
    /// 基线来源链，**顺序即优先级**；空 = 未设置（即主干）。
    /// 界面上按这个顺序找上游：第一个有该模块分支的用它，都不命中兜底主干。
    pub base: Vec<String>,
    pub doc_family: Option<String>,
}

/// 解析「与基线」实际该用的 ref：**配置链里最后配置的**、该模块确实存在的那个。
///
/// 只读；会 fetch 一次，避免本地 ref 过期。全部不存在时返回主干，
/// `fell_back_to_main=true` 让界面能说清为什么比的是主干。
#[tauri::command(async)]
pub async fn ws_baseline(
    branch: String,
    module: String,
    app: State<'_, App>,
) -> CmdResult<lifecycle::BaseResolution> {
    let hub = app.hub()?;
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        Ok(lifecycle::resolve_latest_base(
            &hub,
            &module,
            &ws.meta().base,
        )?)
    })
    .await
}

#[tauri::command]
pub fn ws_list(app: State<'_, App>) -> CmdResult<Vec<WsRow>> {
    app.with_hub(|hub| {
        Ok(gws_core::workspace::list(hub)?
            .into_iter()
            .map(|ws| {
                let present = ws.present_modules().len();
                WsRow {
                    missing_modules: ws.meta().modules.len() - present,
                    modules: present,
                    branch: ws.branch().to_string(),
                    title: ws.title().to_string(),
                    created: ws.meta().created.clone(),
                    base: ws.meta().base.clone(),
                    doc_family: ws.meta().doc.as_ref().map(|d| d.family.clone()),
                }
            })
            .collect())
    })
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewInput {
    /// 需求名称。与 `branch` 二选一。
    pub name: Option<String>,
    /// 完整分支名（指定了就完全按它来，不拼接）。
    pub branch: Option<String>,
    pub title: Option<String>,
    pub modules: Vec<String>,
    #[serde(default)]
    pub base: Vec<String>,
    pub prefix: Option<String>,
    pub doc_family: Option<String>,
    /// 建的时候就能填的备注与自定义信息。
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub links: Option<Vec<gws_core::workspace::Link>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NewResult {
    pub branch: String,
    pub created: Vec<String>,
    pub base_resolved: std::collections::BTreeMap<String, String>,
    pub base_notes: std::collections::BTreeMap<String, String>,
    /// 文档目录挂载结果。挂失败不影响工作区本身，所以单独回报而不是让整个调用失败。
    pub doc: Option<gws_core::dochub::AttachOutcome>,
    pub doc_error: Option<String>,
}

#[tauri::command(async)]
pub fn ws_new(input: NewInput, app: State<'_, App>) -> CmdResult<NewResult> {
    let hub = app.hub()?;

    let branch = match (&input.branch, &input.name) {
        (Some(b), _) if !b.trim().is_empty() => b.trim().to_string(),
        (_, Some(n)) if !n.trim().is_empty() => {
            let prefix = input
                .prefix
                .clone()
                .filter(|p| !p.trim().is_empty())
                .unwrap_or_else(|| hub.config().defaults.branch_prefix.clone());
            let digits = hub.config().defaults.branch_date_digits;
            gws_core::name::compose_branch(
                &prefix,
                &gws_core::date::today_with_digits(digits),
                n,
            )
        }
        _ => return Err(CommandError::plain("请填写需求名称，或直接给出完整分支名")),
    };

    // 与界面默认一致：没传模块时**只挂文档库**，不拉任何代码仓库。
    // 旧版回落到「全部仓库」，一次建 22 个 worktree；界面虽然总会传值，
    // 但 skill / 外部调用同样走这里，默认必须是安全的。
    let modules = if input.modules.is_empty() {
        vec![hub.dochub_name()]
    } else {
        input.modules.clone()
    };

    let opts = NewOptions {
        branch: branch.clone(),
        title: input.title.clone().unwrap_or_else(|| branch.clone()),
        prefix: input
            .prefix
            .clone()
            .unwrap_or_else(|| hub.config().defaults.branch_prefix.clone()),
        // 空数组 = 主干，**不要写「主干」这个字面量**（写进去界面就会把它当成
        // 一个真填过的值显示出来，实测反馈 #4）。core 的 new 还会再归一化一遍。
        base: input.base.clone(),
        modules,
        doc_family: input.doc_family.clone(),
        notes: input.notes.clone().unwrap_or_default(),
        links: input.links.clone().unwrap_or_default(),
    };

    let out = lifecycle::new(&hub, &opts)?;

    // 自动挂文档（需求 #2）。失败只回报，不让整个创建失败——代码工作区已经
    // 建好了，文档挂不上不该让它白建。
    // 勾了 doc-hub 才挂文档（一般需求都会勾，但可以不勾）
    let (doc, doc_error) = if out.wants_docs {
        attach_docs(
            &hub,
            &branch,
            input.doc_family.as_deref(),
            AttachMode::Create,
        )
    } else {
        (None, None)
    };

    Ok(NewResult {
        branch: out.branch,
        created: out.created,
        base_resolved: out.base_resolved,
        base_notes: out.base_notes,
        doc,
        doc_error,
    })
}

/// 挂文档目录。
///
/// `mode` 决定"本地还没有这一期时怎么办"：
/// - [`AttachMode::Create`]（新建需求、手动「挂上文档」）：生成期次目录与说明文件
/// - [`AttachMode::ReuseOnly`]（导入需求）：**什么都不写** —— 那一期在远程就有，
///   没有就说明这个需求还没文档（见 `AttachMode` 的注释）
fn attach_docs(
    hub: &gws_core::Hub,
    branch: &str,
    family: Option<&str>,
    mode: gws_core::dochub::AttachMode,
) -> (Option<gws_core::dochub::AttachOutcome>, Option<String>) {
    if !hub.config().dochub.enabled {
        return (None, None);
    }
    let r = gws_core::dochub::DocHub::ensure(hub).and_then(|dh| {
        let mut ws = Workspace::open(hub, branch)?;
        dh.attach_with(&mut ws, family, mode)
    });
    match r {
        Ok(a) => (Some(a), None),
        Err(e) => (None, Some(e.to_string())),
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GetInput {
    pub branch: String,
    pub title: Option<String>,
    pub doc_family: Option<String>,
    /// 建的时候就能填的备注与自定义信息。
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub links: Option<Vec<gws_core::workspace::Link>>,
}

#[tauri::command(async)]
pub async fn ws_get(
    input: GetInput,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<lifecycle::GetOutcome> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    bg(move || {
        let opts = GetOptions {
            branch: input.branch.clone(),
            title: input.title.clone().unwrap_or_else(|| input.branch.clone()),
            doc_family: input.doc_family.clone(),
            notes: input.notes.clone().unwrap_or_default(),
            links: input.links.clone().unwrap_or_default(),
            base: Vec::new(),
            // 界面上单独填的导入：老行为，扫全部仓库反推谁参与了这个需求
            modules: None,
        };
        // **带上进度**：导入要扫所有仓库的远程分支，跑十几秒。
        // 原来传 None，界面上只有一个转圈、看不到在扫哪个仓库（实测反馈）。
        let out = lifecycle::get(&hub, &opts, Some(&cb))?;
        // 导入的需求也挂上文档（对方推上来的文档分支就在 doc-hub 上）；
        // 失败不影响导入本身。
        //
        // **ReuseOnly**：远程没有这一期就**什么都不建** —— 导入时凭空生成
        // meta.json / CLAUDE.md / AGENTS.md 会挡住之后的 `git pull`（实测反馈）。
        let _ = attach_docs(
            &hub,
            &out.branch,
            input.doc_family.as_deref(),
            AttachMode::ReuseOnly,
        );
        Ok(out)
    })
    .await
}

// ══ 需求配置的复制 / 粘贴（实测反馈：把需求发给别人，对方粘贴即建）══════════
//
// 两个场景：
//   - **拉别人一起做**：我建好需求，把配置发给他，他粘贴一下就有跟我一样的环境
//   - **提测**：以前得把分支名念给测试听，现在直接发这段，他粘贴即建
//
// 所以导出的是**自包含**的一段文本：模块连着远程地址一起带上，
// 对方没有那个仓库时能自己拉下来（见 core 的 `workspace::share`）。

/// 把某个需求的配置渲染成一段可粘贴的文本（发给别人）。
#[tauri::command]
pub fn ws_share(branch: String, app: State<'_, App>) -> CmdResult<String> {
    app.with_hub(|hub| {
        let ws = Workspace::open(hub, &branch)?;
        Ok(gws_core::workspace::share::export(hub, &ws)?.render())
    })
}

/// 解析别人发来的那段配置（**纯本地，不建任何东西**）。
///
/// 界面拿到它就先把表单填好、并把「你本地缺哪几个仓库」列出来让人确认 ——
/// 直接闷头建的话，缺仓库要联网克隆几十秒，人看不到会发生什么。
#[tauri::command]
pub fn ws_share_parse(
    text: String,
) -> CmdResult<gws_core::workspace::share::SharedRequirement> {
    Ok(gws_core::workspace::share::parse(&text)?)
}

/// 导入结果：需求本身 + 文档挂载。
///
/// **没有「顺手补的环境检出」** —— 导入不再替你在各环境建检出（理由见
/// `ws_share_import`）。那一对字段一并去掉，免得留着永远为空的字段让人
/// 以为「环境已经补齐了」。
#[derive(Debug, Clone, Serialize)]
pub struct ShareImportResult {
    pub branch: String,
    pub title: String,
    /// 这次补上的仓库（本地原本没有）。
    pub added_repos: Vec<String>,
    pub modules: Vec<String>,
    pub reused: Vec<String>,
    /// 配置里写了、但这个仓库的远程没有该分支（正常）。
    pub not_involved: Vec<String>,
    /// 文档挂载（失败不影响需求本身，单独回报）。
    pub doc: Option<gws_core::dochub::AttachOutcome>,
    pub doc_error: Option<String>,
}

/// 按别人发来的配置把需求建出来。
///
/// **慢**：缺的仓库要克隆、各仓库要 fetch 确认真有这条分支，都要进度。
#[tauri::command(async)]
pub async fn ws_share_import(
    shared: gws_core::workspace::share::SharedRequirement,
    app: State<'_, App>,
    app_handle: tauri::AppHandle,
) -> CmdResult<ShareImportResult> {
    let hub = app.hub()?;
    let cb = super::progress_cb(&app_handle);
    bg(move || {
        // 界面递过来的（人在预览里可能改过中文名/备注）**也要过同一套校验** ——
        // 校验只有 `validate` 这一处，两条路都走它。
        gws_core::workspace::share::validate(&shared)?;
        let out = gws_core::workspace::share::import(&hub, &shared, Some(&cb))?;

        // ⚠️ **这里不补环境检出**（原来会为每个新补的仓库在 dev/pre… 各建一份）。
        //
        // 实测反馈：① 导入会顺带拉好几个环境的检出，又慢又不在这次操作的重点上；
        // ② 更要紧的是**环境分支是有覆盖的**（`envs[].overrides`：某个仓库在 pre
        // 用的可能是 `pre-202610` 而不是 `pre`）—— 在那个上下文之外替人决定
        // 「这个仓库该检出哪条分支」，很可能绑错。
        //
        // 所以：环境检出交给「环境」页的「同步」，那时才按覆盖表算分支。
        // 补进来的仓库仍然登记在案（`repo::add` 干的），环境页会照常列出来并
        // 标出「还没检出」——不会变成没人知道的状态。
        // 其他人推上来的文档分支就在 doc-hub 上，一并挂上（失败只回报，不影响需求）。
        // 同样是 **ReuseOnly**：只复用他推上来的那一期，不凭空生成。
        let (doc, doc_error) = attach_docs(
            &hub,
            &out.branch,
            shared.doc_family.as_deref(),
            AttachMode::ReuseOnly,
        );

        Ok(ShareImportResult {
            branch: out.branch,
            title: out.title,
            added_repos: out.added_repos,
            modules: out.modules,
            reused: out.reused,
            not_involved: out.not_involved,
            doc,
            doc_error,
        })
    })
    .await
}

/// 某工作区的**第一屏**：只要元数据，**一个 git 调用都不打**。
///
/// # 为什么把它从 `ws_detail` 里拆出来
///
/// 各模块那三列数字要起十几个 git 进程 —— Windows 上每个 ~85ms，一个 4 模块的需求
/// 就是 1~2 秒（实测反馈：「每次进去需求还是会转一会儿圈，卡住一会儿」）。
/// 而**元数据与模块名只是读几个文件**（毫秒级）。
///
/// 所以界面上分两段：先拿这个把页面（表头、模块行、操作按钮）**立刻画出来**，
/// 数字再等 `ws_detail` 回来填。等待期间那几格显示「…」，不是空白也不是 0
/// —— 0 会被读成"真的没有改动"。
#[derive(Debug, Clone, Serialize)]
pub struct ShellModule {
    pub name: String,
    /// 目录还在不在（查文件系统，不跑 git）。
    pub present: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WsShell {
    pub meta: WorkspaceMeta,
    /// **元数据里登记的**模块 + 目录是否还在（只查文件系统）。
    ///
    /// 刻意用 `meta.modules` 而不是 `present_modules()`：后者把"目录缺失"的模块
    /// 滤掉了，于是第二屏（`ws_detail`）会**多出几行**来 —— 表格当场长高、
    /// 下面的东西被顶下去（跟备注块是同一类问题）。行集合必须两屏一致。
    pub modules: Vec<ShellModule>,
    /// 挂了文档库没有（决定"置顶那一行"要不要显示数字）。
    pub doc: bool,
}

#[tauri::command(async)]
pub async fn ws_shell(branch: String, app: State<'_, App>) -> CmdResult<WsShell> {
    let hub = app.hub()?;
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        Ok(WsShell {
            meta: ws.meta().clone(),
            modules: ws
                .meta()
                .modules
                .iter()
                .map(|m| ShellModule {
                    name: m.clone(),
                    present: ws.module_path(m).join(".git").exists(),
                })
                .collect(),
            doc: ws.meta().doc.is_some(),
        })
    })
    .await
}

/// 某工作区的元数据 + 各模块状态（**慢的那一半**：每个模块要起几个 git 进程）。
#[derive(Debug, Clone, Serialize)]
pub struct WsDetail {
    pub meta: WorkspaceMeta,
    pub status: sync::StatusReport,
    /// 文档（doc-hub）那一行的数字。
    ///
    /// **跟模块状态一起给**，而不是让前端再发两个请求去拼 —— 那样任一个失败
    /// 整行就是空的（实测反馈：需求里的 doc-hub 三个列都没显示）。
    /// 它本身也是「一块挂在需求分支上的 worktree」，意义完全一样。
    pub doc: Option<DocRowStatus>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DocRowStatus {
    /// 待提交的文档改动数。
    pub dirty: usize,
    /// 相对 doc-hub 主干领先 / 落后多少提交。
    pub ahead_of_main: u64,
    /// 相对它自己的远程分支。
    pub pushed: bool,
    pub ahead: u64,
    pub behind: u64,
}

/// 某工作区的元数据 + 各模块状态。
///
/// **走 `bg`，不能写成同步命令**：Tauri 的同步命令在**主线程**上跑，而这里要
/// 对每个模块打一串 git（起进程是大头）。同步跑意味着点开需求的这一两秒里，
/// 整个 IPC 都堵在它后面（实测反馈：「进入需求会 loading 卡几秒才出来界面」）。
#[tauri::command(async)]
pub async fn ws_detail(
    branch: String,
    filter: FilterInput,
    app: State<'_, App>,
) -> CmdResult<WsDetail> {
    let hub = app.hub()?;
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        let f: ModuleFilter = filter.into();
        // 代码模块那一堆 git 和文档那一行的 git 落在**不同目录**里，互不相干 ——
        // 并起来跑，省掉文档那 4 个 git 进程的等待（它原来是接在后面串着跑的）。
        let (status, doc) = std::thread::scope(|scope| {
            let st = scope.spawn(|| sync::status(&hub, &ws, &f));
            let doc = scope.spawn(|| doc_row_status(&hub, &ws));
            let status = st.join().unwrap_or_else(|_| {
                Err(gws_core::Error::other("读模块状态的工作线程异常退出"))
            });
            let doc = doc.join().unwrap_or(None);
            (status, doc)
        });
        Ok(WsDetail {
            status: status?,
            meta: ws.meta().clone(),
            doc,
        })
    })
    .await
}

/// 文档那一行的数字。**任何一步失败就返回 `None`**，界面据此把那几列留空
/// 而不是显示错数字。
fn doc_row_status(hub: &gws_core::Hub, ws: &Workspace) -> Option<DocRowStatus> {
    // 没挂文档就没有这一行
    ws.meta().doc.as_ref()?;
    let dh = gws_core::dochub::DocHub::ensure_local(hub).ok()?;
    let dirty = dh.status(ws).map(|v| v.len()).unwrap_or(0);
    let ahead_of_main = dh.ahead_of_main(ws).ok()?;
    let dir = dh.docs_dir(ws);
    let git = gws_core::git::Git::new(&dir);
    // 远程分支可能还没建（刚 attach）→ 用 (0,0) 表示"未推送"
    let pushed = git.remote_branch_exists(ws.branch());
    let (ahead, behind) = if pushed {
        git.ahead_behind(&format!("origin/{}", ws.branch()))
            .unwrap_or((0, 0))
    } else {
        (0, 0)
    };
    Some(DocRowStatus {
        dirty,
        ahead_of_main,
        pushed,
        ahead,
        behind,
    })
}

#[tauri::command]
pub fn ws_add_modules(
    branch: String,
    modules: Vec<String>,
    app: State<'_, App>,
) -> CmdResult<Vec<RepoOutcome>> {
    app.with_hub(|hub| {
        let mut ws = Workspace::open(hub, &branch)?;
        lifecycle::add_modules(hub, &mut ws, &modules)
    })
}

#[tauri::command]
pub fn ws_drop_module(
    branch: String,
    module: String,
    force: bool,
    app: State<'_, App>,
) -> CmdResult<Outcome> {
    app.with_hub(|hub| {
        let mut ws = Workspace::open(hub, &branch)?;
        lifecycle::drop_module(hub, &mut ws, &module, force)
    })
}

/// 清理「元数据登记了但目录已不在」的模块。
#[tauri::command]
pub fn ws_drop_missing(branch: String, app: State<'_, App>) -> CmdResult<DropMissingOutcome> {
    app.with_hub(|hub| {
        let mut ws = Workspace::open(hub, &branch)?;
        lifecycle::drop_missing_modules(hub, &mut ws)
    })
}

/// 删除工作区。`force=false` 时若存在未保存的工作，返回带风险项的 `RemoveOutcome`
/// （不抛错），由界面决定是否再带 `force=true` 重来——比抛异常更好办。
#[tauri::command]
pub fn ws_remove(branch: String, force: bool, app: State<'_, App>) -> CmdResult<RemoveOutcome> {
    app.with_hub(|hub| lifecycle::remove(hub, &branch, force))
}

/// 扫描 ws/ 下的残留目录（需求 #6 的另一半）。
#[tauri::command]
pub fn ws_clean(apply: bool, app: State<'_, App>) -> CmdResult<CleanOutcome> {
    app.with_hub(|hub| lifecycle::clean(hub, apply))
}

/// 改需求元数据：中文备注 / 基线 / 自定义信息 / 备注（需求 #15 / #20）。
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct MetaPatch {
    pub title: Option<String>,
    /// `None` = 不改，`Some(vec![])` = 清空回主干。
    pub base: Option<Vec<String>>,
    /// `None` = 不改。
    pub links: Option<Vec<gws_core::workspace::Link>>,
    pub notes: Option<String>,
    pub doc_family: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetaPatchOutcome {
    pub meta: WorkspaceMeta,
    /// **顺带动了什么**（改分类挪了文档目录、改中文名同步了期次元数据…）。
    ///
    /// 这些是磁盘上的实际改动，必须让人看见 —— 界面上弹一条。空 = 只改了元数据。
    pub notes: Vec<String>,
}

#[tauri::command]
pub fn ws_patch_meta(
    branch: String,
    patch: MetaPatch,
    app: State<'_, App>,
) -> CmdResult<MetaPatchOutcome> {
    app.with_hub(|hub| {
        let mut ws = Workspace::open(hub, &branch)?;
        // 这些要在借出 meta_mut 之前读掉，否则会与可变借用冲突
        let period = ws.branch().to_string();
        let old_family = ws.meta().doc.as_ref().map(|d| d.family.clone());
        let old_title = ws.title().to_string();

        {
            let m = ws.meta_mut();
            if let Some(t) = patch.title.filter(|t| !t.trim().is_empty()) {
                m.title = t;
            }
            // 基线可以清空 → 回到主干，所以这里区分「不改」与「清空」。
            // 清空 = 存空数组（主干是**缺省语义**，不是一个要存的值）；
            // 顺手做严格校验：重复项、命名不合法（如带空格）都在这里报错。
            if let Some(base) = patch.base {
                m.base = gws_core::workspace::lifecycle::normalise_base_chain(&base)?;
            }
            if let Some(links) = patch.links {
                m.links = links;
            }
            if let Some(notes) = patch.notes {
                m.notes = notes;
            }
            // **分类先不写进 meta** —— 挪文档目录要用「旧分类」定位，
            // 写进去之后就找不到源目录了。下面挪完再写。
        }
        ws.save_meta()?;

        let dh = gws_core::dochub::DocHub::ensure_local(hub)?;
        let mut notes: Vec<String> = Vec::new();

        // ① 改分类 → 把文档目录整棵挪过去（用旧分类定位），成功后再写新分类。
        //    不挪的话：元数据指向新分类、内容还在旧分类，文档页就再也读不到（实测反馈）。
        if let Some(family) = patch
            .doc_family
            .as_ref()
            .map(|f| f.trim())
            .filter(|f| !f.is_empty())
        {
            if old_family.as_deref() != Some(family) {
                if let Some(note) = dh.rename_family(&ws, family)? {
                    notes.push(note);
                }
                if ws.meta().doc.is_some() {
                    ws.meta_mut().doc =
                        Some(gws_core::workspace::DocRef::new(family, period.clone()));
                    ws.save_meta()?;
                }
            }
        }

        // ② 改中文名 → 期次元数据里的 `name` 跟着改（文档页左列表显示的就是它）。
        //    放在挪目录**之后**：这时 period_dir 已经指向新分类下的正确路径。
        if ws.title() != old_title && dh.sync_title(&ws)? {
            notes.push("文档里的需求名已同步".to_string());
        }

        Ok(MetaPatchOutcome {
            meta: ws.meta().clone(),
            notes,
        })
    })
}