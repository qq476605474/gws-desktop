//! 危险操作：撤销改动 / 强制回退 / 清理远程分支 / 改分支名。
//!
//! **每个操作都拆成 plan + apply 两步**，界面必须先调 plan 把影响面
//! （丢几个提交、删哪些分支）摆给人看，人确认后才调 apply。
//! core 侧也是这么设计的，这里只是透传。

use tauri::State;
use gws_core::gitlab::Client;
use gws_core::workspace::branch::{self, PruneOutcome, PrunePlan, RenameOutcome, RenamePlan};
use gws_core::workspace::revert::{self, ResetOutcome, ResetPlan, RevertOutcome, RevertPlan};
use gws_core::workspace::{ModuleFilter, Workspace};

use super::{bg, App, CmdResult, CommandError, FilterInput};

/// 当前 hub 的 GitLab 客户端。凭据跟着 hub 走，所以必须传 hub 进来。
fn client(hub: &gws_core::hub::Hub) -> CmdResult<Client> {
    Client::for_hub(hub).map_err(|e| CommandError::plain(e.to_string()))
}

// ── 撤销改动（需求 #3）──────────────────────────────────────────────

#[tauri::command]
pub fn ws_revert_plan(
    branch: String,
    filter: FilterInput,
    app: State<'_, App>,
) -> CmdResult<RevertPlan> {
    app.with_hub(|hub| {
        let ws = Workspace::open(hub, &branch)?;
        let f: ModuleFilter = filter.into();
        revert::revert_plan(hub, &ws, &f)
    })
}

#[tauri::command]
pub fn ws_revert_apply(
    branch: String,
    filter: FilterInput,
    app: State<'_, App>,
) -> CmdResult<RevertOutcome> {
    app.with_hub(|hub| {
        let ws = Workspace::open(hub, &branch)?;
        let f: ModuleFilter = filter.into();
        revert::revert_apply(hub, &ws, &f)
    })
}

// ── 强制回退到远程（需求 #10）──────────────────────────────────────

// 注意：core 的 reset_* 内部会 `git fetch`（要拿到远程最新才能回退到它），
// 也就是**会联网**。所以这两条走 `bg`；写成同步命令会把主线程卡住几十秒
// ——Tauri 的同步命令是在主线程上跑的。
#[tauri::command(async)]
pub async fn ws_reset_plan(
    branch: String,
    filter: FilterInput,
    app: State<'_, App>,
) -> CmdResult<ResetPlan> {
    let hub = app.hub()?;
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        let f: ModuleFilter = filter.into();
        Ok(revert::reset_plan(&hub, &ws, &f)?)
    })
    .await
}

#[tauri::command(async)]
pub async fn ws_reset_apply(
    branch: String,
    filter: FilterInput,
    app: State<'_, App>,
) -> CmdResult<ResetOutcome> {
    let hub = app.hub()?;
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        let f: ModuleFilter = filter.into();
        Ok(revert::reset_apply(&hub, &ws, &f)?)
    })
    .await
}

// ── 清理远程分支（需求 #11）────────────────────────────────────────
//
// 下面四条的 `plan` / `apply` 都要打 GitLab（查/删远程分支），所以一律走 `bg`，
// 而不是把 `#[tauri::command(async)]` 直接加在同步函数上——理由见 `bg` 文档。

#[tauri::command(async)]
pub async fn branch_prune_plan(
    branch: String,
    filter: FilterInput,
    app: State<'_, App>,
) -> CmdResult<PrunePlan> {
    let hub = app.hub()?;
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        let f: ModuleFilter = filter.into();
        Ok(branch::prune_plan(&hub, &ws, &f, &client(&hub)?)?)
    })
    .await
}

#[tauri::command(async)]
pub async fn branch_prune_apply(
    branch: String,
    filter: FilterInput,
    app: State<'_, App>,
) -> CmdResult<PruneOutcome> {
    let hub = app.hub()?;
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        let f: ModuleFilter = filter.into();
        Ok(branch::prune_apply(&hub, &ws, &f, &client(&hub)?)?)
    })
    .await
}

// ── 改分支名（需求 #12）────────────────────────────────────────────

#[tauri::command(async)]
pub async fn branch_rename_plan(
    branch: String,
    new_branch: String,
    filter: FilterInput,
    app: State<'_, App>,
) -> CmdResult<RenamePlan> {
    let hub = app.hub()?;
    bg(move || {
        let ws = Workspace::open(&hub, &branch)?;
        let f: ModuleFilter = filter.into();
        // 未推送到远程的需求不需要令牌也能改名；core 会在有远程分支时要求它
        Ok(branch::rename_plan(
            &hub,
            &ws,
            &f,
            &new_branch,
            client(&hub).ok().as_ref(),
        )?)
    })
    .await
}

#[tauri::command(async)]
pub async fn branch_rename_apply(
    branch: String,
    new_branch: String,
    filter: FilterInput,
    app: State<'_, App>,
) -> CmdResult<RenameOutcome> {
    let hub = app.hub()?;
    bg(move || {
        let mut ws = Workspace::open(&hub, &branch)?;
        let f: ModuleFilter = filter.into();
        Ok(branch::rename_apply(
            &hub,
            &mut ws,
            &f,
            &new_branch,
            client(&hub).ok().as_ref(),
        )?)
    })
    .await
}

// ── 文件浏览与对比（需求 #18）──────────────────────────────────────
//
// 统一按「**目录 + 所属仓库**」定位，而不是按模块名：
// 同一个浏览器要服务四种地方 —— 需求里的模块（`ws/<分支>/<模块>`）、
// 仓库主干镜像（`repos/<仓库>`）、环境检出（`envs/<环境>/<仓库>`）、
// 文档期次（`repos/doc-hub/workspaces/…`）。它们的共同点是
// 「一个 git 工作目录 + 一个该跟谁比的主干」。

/// 一个改动文件：路径 + git 状态字母。
///
/// 光给路径看不出「是新增还是删了」——review 时这两者的性质完全不同。
#[derive(Debug, Clone, serde::Serialize)]
pub struct FileChange {
    pub path: String,
    /// `A` 新增 / `M` 修改 / `D` 删除 / `R` 重命名 / `T` 类型变化 …
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FileDiff {
    pub path: String,
    pub stat: String,
    pub diff: String,
    /// 文件内容。看某个历史提交时给的是**那个提交时的版本**；看工作树改动时为空
    /// （那时内容由 `browse_read` 单独读）。
    #[serde(default)]
    pub content: String,
    /// 最后一次改动这个文件的人（显示名）。
    ///
    /// review 时「谁改的」和「改了什么」一样重要 —— 要追责/追问总得知道找谁。
    pub author: String,
    /// 该提交的日期 `YYYY-MM-DD`。
    pub date: String,
    /// 该提交的说明（一行）。
    pub subject: String,
}

/// 比对基准：显式给了就用它，没给才探测主干。
fn resolve_base(hub: &gws_core::Hub, repo: &str, given: Option<String>) -> CmdResult<String> {
    let base = match given
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
    {
        Some(b) => b,
        None => base_ref_for(hub, repo)?,
    };
    Ok(base)
}

/// 比对前先确认基准 ref **真的存在**，不存在就给一句能看懂的话。
///
/// 实测反馈「与远程对比」报了一串 git 的 usage 文本 —— 真因是这个需求的**分支
/// 还没推送过**，`origin/<分支>` 根本不存在，`git diff origin/X...HEAD` 于是
/// 报 `fatal: ambiguous argument`（git 把用法说明一起打出来了）。
/// 那不是缺陷，但「把 git 的报错原文丢给用户」是缺陷。
fn ensure_base_exists(dir: &std::path::Path, base: &str) -> CmdResult<()> {
    let git = gws_core::git::Git::new(dir);
    if git.ref_exists(base) {
        return Ok(());
    }
    // 说清「是什么情况」+「下一步做什么」
    let hint = if let Some(b) = base.strip_prefix("origin/") {
        format!(
            "远程还没有 `{b}` 这个分支。\n  · 本需求的分支还没推送过 → 先「推送」\n  · 或者换个对比目标（与主干 / 与环境）"
        )
    } else {
        format!("找不到这个基准：{base}")
    };
    Err(CommandError::plain(hint))
}

/// 用来比对的主干 ref。
///
/// 「所属仓库」只用来探测主干名（各仓库 main/master 混用），文档库走它自己的
/// clone。**不要让前端拼 `origin/main`** —— 写死会在 master 的那批仓库上全错。
fn base_ref_for(hub: &gws_core::Hub, repo: &str) -> CmdResult<String> {
    let repo_root = if hub.is_dochub(repo) {
        let d = hub.dochub_dir();
        if !d.join(".git").is_dir() {
            return Err(CommandError::plain(
                "文档库还没拉到本地，先在「仓库」页拉取",
            ));
        }
        d
    } else {
        let r = gws_core::repo::Repo::open(hub, repo)?;
        r.path.clone()
    };
    let main = gws_core::git::detect_main_branch(&repo_root)
        .map_err(|e| CommandError::plain(format!("探测 {repo} 的主干失败：{e}")))?;
    Ok(format!("origin/{main}"))
}

/// 校验目录确实在 hub 内，并返回它的绝对路径。
///
/// **不能省**：这些命令的参数来自前端，不校验的话就能让后端去读 hub 之外的
/// 任意文件（路径穿越）。判据用「规范化后是否以 hub 根为前缀」。
fn checked_dir(hub: &gws_core::Hub, dir: &str) -> CmdResult<std::path::PathBuf> {
    // **去掉 Windows 的 `\\?\` 前缀**：`canonicalize` 在 Windows 上返回逐字路径，
    // 而它会被当成 git 的 cwd / 参数传下去 —— 实测反馈里克隆与 worktree add
    // 就是死在 `\\?\D:\…` 上（`Invalid argument`）。两边都剥，比较才仍然一致。
    let strip = gws_core::paths::strip_verbatim_prefix;
    let root = strip(
        &hub.root()
            .canonicalize()
            .unwrap_or_else(|_| hub.root().to_path_buf()),
    );
    let p = std::path::PathBuf::from(dir);
    let real = p.canonicalize().unwrap_or(p.clone());
    if !resolved_inside(&root, &real) {
        return Err(CommandError::plain(format!(
            "目录不在 hub 内，已拒绝: {dir}"
        )));
    }
    Ok(strip(&real))
}

/// 把 `dir` 里的相对路径 `path` 解析成绝对路径，并确认它**没有越出 `dir`**。
///
/// # 为什么单独抽一个
///
/// `checked_dir` 已经把 Windows 的 `\\?\` 前缀剥掉了，但**里面这第二道检查**
/// 原来各写各的、忘了剥 —— 于是 `canonicalize` 返回的 `\\?\D:\…` 与不带前缀的
/// `dir` 一比，`starts_with` 必然为假：**Windows 上只要文件真的存在，
/// 读它必定报「路径越界」**（实测反馈：对比代码里点任何一项都提示路径越界）。
/// 同一个坑仓库里记过两次（克隆、worktree add），这是第三次 —— 所以这次
/// 不再复制粘贴，两条路都走这个函数。
fn join_inside(dir: &std::path::Path, path: &str) -> CmdResult<std::path::PathBuf> {
    // **先挡住绝对路径与 `..`**：`Path::starts_with` 是**纯词法**比较，不做规范化，
    // 所以 `dir/../../../etc/hosts` 在「文件不存在 → canonicalize 失败」时会
    // **通过**前缀检查（成分逐段对得上）。目标文件存在时确实会被下面的
    // canonicalize 抓到，但不该把安全性押在「那个文件碰巧不存在」上。
    let rel = std::path::Path::new(path);
    // 允许 `.`（`./x` 这种写法无害），挡掉的是绝对路径、盘符前缀与 `..`
    if rel.is_absolute()
        || rel.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(CommandError::plain(format!("路径越界: {path}")));
    }
    let full = dir.join(path);
    // 文件/目录可能不存在（比如 diff 里列出的"别人删掉的"文件）——
    // 那时 canonicalize 会失败，退回原样比较即可，**不是错误**。
    let real = full.canonicalize().unwrap_or(full.clone());
    if !resolved_inside(dir, &real) {
        return Err(CommandError::plain(format!("路径越界: {path}")));
    }
    Ok(gws_core::paths::strip_verbatim_prefix(&real))
}

/// 「规范化后的路径确实在 `dir` 里吗」——**纯函数，便于单测**。
///
/// 剥 `\\?\` 那一步必须在这里做：`canonicalize` 在 Windows 上返回逐字路径，
/// 而 `dir` 是剥过的。不剥的话 `starts_with` 必然为假 ——
/// **Windows 上只要文件真的存在，读它必定报「路径越界」**。
/// 抽成纯函数之后，「喂一对 `\\?\D:\…` / `D:\…` 必须判为在内」这条
/// 在 mac / Linux 上也能测（见 `browse_tests`），否则这个坑只能靠人在
/// Windows 上撞出来 —— 前面已经撞过两次了。
pub(crate) fn resolved_inside(dir: &std::path::Path, resolved: &std::path::Path) -> bool {
    gws_core::paths::strip_verbatim_prefix(resolved).starts_with(dir)
}

/// 判断 `path` 在 `dir` 里且就是它自己（用来识别「未跟踪的新文件」）。
fn is_inside(dir: &std::path::Path, path: &str) -> bool {
    join_inside(dir, path).is_ok()
}

/// 列出某个工作目录相对**某个基准 ref** 的改动文件（带状态）。
///
/// `base` 给了就用它，没给才去探测主干。有了它，同一个浏览器才能回答
/// 「本地环境 vs **远程环境分支**」这类问题 —— 检查合并时关心的正是这个，
/// 而不是「跟主干比」（实测反馈）。
#[tauri::command(async)]
pub async fn browse_changes(
    dir: String,
    repo: String,
    base: Option<String>,
    app: State<'_, App>,
) -> CmdResult<Vec<FileChange>> {
    let hub = app.hub()?;
    bg(move || browse_changes_in(&hub, &dir, &repo, base)).await
}

/// 命令体（取 `&Hub` 而不是 `State`，**好让测试直接调**）。
pub(crate) fn browse_changes_in(
    hub: &gws_core::Hub,
    dir: &str,
    repo: &str,
    base: Option<String>,
) -> CmdResult<Vec<FileChange>> {
    {
        let dir = checked_dir(hub, dir)?;
        let base = resolve_base(hub, repo, base)?;
        let git = gws_core::git::Git::new(&dir);
        // fetch 失败不阻断：对不上主干最新也要能看改动（离线场景）
        let _ = git.fetch();
        ensure_base_exists(&dir, &base)?;
        // `-c core.quotePath=false` 不能省！git 默认把非 ASCII 路径转义成八进制
        // （`"specs/\347\233\264..."`），于是：① 界面显示成一串乱码；
        // ② 拿这个转义串去读文件必然 "No such file"。同一个坑仓库里已记过两次
        // （`dochub::sync` / `archived_in_main`），这次是第三次 —— 涉及路径输出
        // 的 git 命令一律带上它。
        // **比的是「本地代码」，所以基准后面不带 `HEAD`**
        //
        // `git diff <base>` = 基准 ↔ **工作区**（含已提交 + 已 add + 未 add 的改动），
        // 而 `git diff <base> HEAD` 只比**已提交的树**。实测反馈：本地改了 21 个
        // 文件（都还没提交），点「对比代码 → 与主干」显示 0 个 —— 提交层面
        // HEAD 与 origin/main 确实一模一样，但「本地代码跟主干的差异」显然不是 0。
        //
        // 用两点（`base` 与工作区）而不是三点（`base...HEAD`）也一并保留了原因：
        // 三点取「合并基之后的改动」，本地**落后**于基准时合并基就是 HEAD 自己
        // → 必然为空（另一条实测反馈：落后 7500 个提交却说「没有变化」）。
        //
        // 基准是**单独一个参数**，不能拼成 `format!("{base} HEAD")` —— 那会被 git
        // 当成一个非法 revision，报出来是一串用法说明（踩过）。
        let mut out: Vec<FileChange> = git
            .out(&["diff", "--name-status", &base])?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                // 形如 `M\tpath`；重命名是 `R100\told\tnew`，要的是新路径
                let mut it = l.split('\t');
                let status = it
                    .next()
                    .unwrap_or("")
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_string();
                let path = it.next_back().unwrap_or("").to_string();
                FileChange { status, path }
            })
            .filter(|c| !c.path.is_empty())
            .collect();

        // **没被跟踪的新文件也要算** —— `git diff` 看不见它们。
        // 「本地代码」当然包括刚写的新文件：只看 `git diff` 的话，
        // 新建一个文件再来对比又是「没有变化」，跟上面那个 bug 一模一样。
        // （`status_lines(false)` 已经滤掉 .DS_Store 这类系统垃圾。）
        //
        // `-uall`（`status_lines(true)`）**不能省**：默认的 `git status` 会把
        // 一整个未跟踪目录折成一行 `?? workspaces/` —— 那是**目录**，点开没有
        // 内容、也读不出文件（实测反馈：「点某一项新增」报错）。`-uall` 把它
        // 展开成目录里一个个真实文件，列表里就只剩文件了。
        let seen: std::collections::BTreeSet<String> = out.iter().map(|c| c.path.clone()).collect();
        for line in git.status_lines(true).unwrap_or_default() {
            // 形如 `?? path`
            let Some(path) = line.strip_prefix("?? ") else {
                continue;
            };
            // 兜一层：万一 git 还是给了目录（带尾斜杠），它不是"一个改动文件"
            let path = path.trim().trim_end_matches('/').to_string();
            if path.is_empty() || seen.contains(&path) {
                continue;
            }
            out.push(FileChange {
                status: "A".to_string(),
                path,
            });
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }
}

/// 给「没被跟踪的新文件」合成一份 diff。
///
/// `git diff` 只认已跟踪的文件，新写的文件它一个字都不给 —— 界面上就是空白，
/// 而用户明明刚加了文件。这里按 `git diff` 的格式自己拼一份「整文件新增」，
/// 右侧的着色逻辑（`+` 绿 / `@@` 蓝）就能直接用上。
///
/// 超过 1MB 只给个头，不做全量 —— 右侧本来也是给人读的，不是给机器解析的。
fn synthetic_new_file_diff(full: &std::path::Path, path: &str) -> (String, String) {
    const MAX: u64 = 1024 * 1024;
    let Ok(meta) = std::fs::metadata(full) else {
        return (String::new(), String::new());
    };
    if meta.len() > MAX {
        return (
            String::new(),
            format!(
                "（新文件 {path} 有 {} 字节，超过 1MB 不在这里展开）\n",
                meta.len()
            ),
        );
    }
    let Ok(text) = std::fs::read_to_string(full) else {
        // 二进制之类读不成文本：说清楚，不给一份乱码
        return (
            String::new(),
            format!("（新文件 {path} 不是文本，无法显示 diff）\n"),
        );
    };
    let lines: Vec<&str> = text.lines().collect();
    let mut diff = format!(
        "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n",
        lines.len()
    );
    for l in &lines {
        diff.push('+');
        diff.push_str(l);
        diff.push('\n');
    }
    let stat = format!(
        " {path} | {} +\n 1 file changed, {} insertions(+)\n",
        lines.len(),
        lines.len()
    );
    (stat, diff)
}

/// 读某个文件的内容（工作目录里的实际内容）。
#[tauri::command(async)]
pub async fn browse_read(dir: String, path: String, app: State<'_, App>) -> CmdResult<String> {
    let hub = app.hub()?;
    bg(move || browse_read_in(&hub, &dir, &path)).await
}

/// 命令体（同上，便于测试）。
pub(crate) fn browse_read_in(hub: &gws_core::Hub, dir: &str, path: &str) -> CmdResult<String> {
    let dir = checked_dir(hub, dir)?;
    // 二次防穿越 + 剥 `\\?\`（见 `join_inside`）
    let real = join_inside(&dir, path)?;
    let meta =
        std::fs::metadata(&real).map_err(|e| CommandError::plain(format!("读不到 {path}: {e}")))?;
    // 点到**目录**要说明白。改动列表里偶尔会冒出一条目录（未跟踪的目录，
    // git 默认只报目录名、不带里面的文件），点开它既不是文件也读不出内容 ——
    // 给一句人话，别让人以为坏了。
    if meta.is_dir() {
        return Err(CommandError::plain(format!(
            "{path} 是个目录，不是文件（里面还没有被跟踪的文件）"
        )));
    }
    let len = meta.len();
    if len > 2 * 1024 * 1024 {
        return Err(CommandError::plain(format!(
            "文件过大（{len} 字节，上限 2MB）"
        )));
    }
    std::fs::read_to_string(&real).map_err(|e| CommandError::plain(format!("读不到 {path}: {e}")))
}

/// 某文件相对主干的 diff。
///
/// 带 `--no-ext-diff --no-color`：**必须是纯文本**，由前端自己着色（按 `+`/`-`/`@@`
/// 上色）。让 git 输出 ANSI 颜色的话前端就得再解析转义码 —— 那正是本版要消除的
/// 旧版做法（§1）。
#[tauri::command(async)]
pub async fn browse_diff(
    dir: String,
    repo: String,
    path: String,
    base: Option<String>,
    app: State<'_, App>,
) -> CmdResult<FileDiff> {
    let hub = app.hub()?;
    bg(move || browse_diff_in(&hub, &dir, &repo, &path, base)).await
}

/// 命令体（同上，便于测试）。
pub(crate) fn browse_diff_in(
    hub: &gws_core::Hub,
    dir: &str,
    repo: &str,
    path: &str,
    base: Option<String>,
) -> CmdResult<FileDiff> {
    {
        let dir = checked_dir(hub, dir)?;
        let base = resolve_base(hub, repo, base)?;
        let git = gws_core::git::Git::new(&dir);
        ensure_base_exists(&dir, &base)?;
        // 同上：比的是**工作区**（不带 `HEAD`），基准是单独一个参数。
        let stat_args = vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--stat",
            &base,
            "--",
            &path,
        ];
        let diff_args = vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--unified=3",
            &base,
            "--",
            &path,
        ];
        // 没被跟踪的新文件：`git diff` 什么都不给，得自己合成一份「新增」的 diff ——
        // 否则点进去是空白，跟「本地改了却显示 0」是同一类误导。
        //
        // **必须同时满足「磁盘上有这个文件」**：只判「没被 git 跟踪」的话，
        // 「基准里有、本地没有」的文件（本地视角是删除）也会走进这条分支，
        // 结果合成不出来、diff 变空 —— 那条路径本来 `git diff` 是能正常给出内容的。
        let full = dir.join(path);
        let untracked = full.is_file()
            && !git.ok(&["ls-files", "--error-unmatch", "--", path])
            // 防目录穿越：读之前确认它真的在这个目录里（与 `browse_read` 同一套）
            && is_inside(&dir, path);
        // 最近一次改这个文件的人。用 `%an|%ad|%s` 一行取全，避免多次调用 git。
        let who = git
            .out(&[
                "log",
                "-1",
                "--format=%an|%ad|%s",
                "--date=short",
                "--",
                path,
            ])
            .unwrap_or_default();
        let mut parts = who.splitn(3, '|');
        let (stat, diff) = if untracked {
            synthetic_new_file_diff(&full, path)
        } else {
            (
                git.out(&stat_args).unwrap_or_default(),
                git.out(&diff_args).unwrap_or_default(),
            )
        };
        Ok(FileDiff {
            path: path.to_string(),
            stat,
            diff,
            author: parts.next().unwrap_or("").to_string(),
            date: parts.next().unwrap_or("").to_string(),
            subject: parts.next().unwrap_or("").to_string(),
            content: String::new(),
        })
    }
}

// ══ 提交历史 ═══════════════════════════════════════════════════════════

/// 一条提交。时间/作者/说明都要显示 —— 「什么时候、谁、改了什么」是 review 时
/// 最常问的三个问题。
#[derive(Debug, Clone, serde::Serialize)]
pub struct CommitInfo {
    pub sha: String,
    pub author: String,
    /// `YYYY-MM-DD HH:MM`（本地时区，git 自己格式化，不用前端再转一次）。
    pub date: String,
    pub subject: String,
}

/// 最近 N 条提交。
#[tauri::command(async)]
pub async fn browse_commits(
    dir: String,
    limit: Option<usize>,
    app: State<'_, App>,
) -> CmdResult<Vec<CommitInfo>> {
    let hub = app.hub()?;
    bg(move || browse_commits_in(&hub, &dir, limit)).await
}

/// 命令体（便于测试）。
pub(crate) fn browse_commits_in(
    hub: &gws_core::Hub,
    dir: &str,
    limit: Option<usize>,
) -> CmdResult<Vec<CommitInfo>> {
    {
        let dir = checked_dir(hub, dir)?;
        let git = gws_core::git::Git::new(&dir);
        let n = limit.unwrap_or(20).to_string();
        // 用 `%h|%an|%ad|%s` 一行取全，避免每个字段一次 git 调用
        let out = git.out(&[
            "log",
            "-n",
            &n,
            "--date=format:%Y-%m-%d %H:%M",
            "--format=%h|%an|%ad|%s",
        ])?;
        Ok(out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let mut it = l.splitn(4, '|');
                CommitInfo {
                    sha: it.next().unwrap_or("").to_string(),
                    author: it.next().unwrap_or("").to_string(),
                    date: it.next().unwrap_or("").to_string(),
                    subject: it.next().unwrap_or("").to_string(),
                }
            })
            .collect())
    }
}

/// 某个提交改了哪些文件（带 A/M/D 状态）。
#[tauri::command(async)]
pub async fn browse_commit_files(
    dir: String,
    sha: String,
    app: State<'_, App>,
) -> CmdResult<Vec<FileChange>> {
    let hub = app.hub()?;
    bg(move || browse_commit_files_in(&hub, &dir, &sha)).await
}

/// 命令体（便于测试）。
pub(crate) fn browse_commit_files_in(
    hub: &gws_core::Hub,
    dir: &str,
    sha: &str,
) -> CmdResult<Vec<FileChange>> {
    {
        let dir = checked_dir(hub, dir)?;
        let git = gws_core::git::Git::new(&dir);
        // `--format=` 把提交头压掉，只留 --name-status 的清单
        let out = git.out(&["show", "--name-status", "--format=", sha])?;
        Ok(out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let mut it = l.split('\t');
                let status = it
                    .next()
                    .unwrap_or("")
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_string();
                let path = it.next_back().unwrap_or("").to_string();
                FileChange { status, path }
            })
            .filter(|c| !c.path.is_empty())
            .collect())
    }
}

/// 某个文件在某个提交里的改动 + 该提交时的内容。
#[tauri::command(async)]
pub async fn browse_commit_diff(
    dir: String,
    sha: String,
    path: String,
    app: State<'_, App>,
) -> CmdResult<FileDiff> {
    let hub = app.hub()?;
    bg(move || {
        let dir = checked_dir(&hub, &dir)?;
        let git = gws_core::git::Git::new(&dir);
        // 内容取的是**那个提交时的版本**，不是当前工作树 —— 否则"改动"和
        // "内容"对不上，看的人会以为文件被改坏了
        let content = git
            .out(&["show", &format!("{sha}:{path}")])
            .unwrap_or_default();
        Ok(FileDiff {
            path: path.to_string(),
            stat: git
                .out(&[
                    "show",
                    "--no-color",
                    "--stat",
                    "--format=",
                    &sha,
                    "--",
                    &path,
                ])
                .unwrap_or_default(),
            diff: git
                .out(&["show", "--no-color", "--no-ext-diff", &sha, "--", &path])
                .unwrap_or_default(),
            author: String::new(),
            date: String::new(),
            subject: String::new(),
            content,
        })
    })
    .await
}

/// `(本地领先基准数, 本地落后基准数)`。方向要能看出来。
///
/// `base` 不给（`null`）就**探测主干** —— 「与主干」那个视图同样要看
/// 「领先主干多少个提交」，而"主干是 main 还是 master"只有后端知道。
#[tauri::command(async)]
pub async fn browse_ahead_behind(
    dir: String,
    repo: String,
    base: Option<String>,
    app: State<'_, App>,
) -> CmdResult<(u64, u64)> {
    let hub = app.hub()?;
    bg(move || {
        let dir = checked_dir(&hub, &dir)?;
        let base = resolve_base(&hub, &repo, base)?;
        let git = gws_core::git::Git::new(&dir);
        ensure_base_exists(&dir, &base)?;
        Ok(git.ahead_behind(&base)?)
    })
    .await
}
