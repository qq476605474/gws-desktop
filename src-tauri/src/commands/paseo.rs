//! 从需求详情直接开一个 Paseo AI 会话（#6）。
//!
//! **cwd 固定在 hub 根目录**（实测反馈）：开在 `ws/<分支>` 里的话，AI 读不到
//! 根目录的 `AGENTS.md` / `CLAUDE.md`，也看不到 `repos/` 里的主干代码。
//! 这些约定由 `AGENTS.md` 自己说明，prompt 只给分支、让 AI 直接开始 ——
//! 不在每次开新会话时重复一遍目录说明。
//!
//! 用 `paseo run -d --json` 创建后，再 `paseo agent open <id>` 把桌面端叫起来。
//! Paseo 没装时给一句能照做的提示，不静默失败。
//!
//! **启动前先确保 hub 根目录已在 Paseo 里注册成 project**（实测反馈：Paseo
//! 桌面端里看不到当前 hub，会话像"凭空"出现的）。已注册就跳过，没注册才
//! `paseo project create`，不会每次重复建。
//!
//! **所有 paseo 调用都要清掉从宿主进程继承的 `PASEO_AGENT_ID` /
//! `PASEO_AGENT_CWD` / `PASEO_WORKSPACE_ID`**（实测反馈：新会话目录不对）。
//! 桌面端若是从 Paseo 的终端里启动的，环境里会带这些变量；`paseo run` 一旦
//! 看到它们就把新会话归到「调用者的工作区」，`--cwd` 直接被忽略 —— 会话于是
//! 永远开在宿主会话的目录（例如源码仓库），而不是当前 hub。
//!
//! **复用工作区，不再每次新建**（实测反馈：左侧出现一堆同名的 `my-hub`）：
//! 先 `paseo workspace ls` 找「本地 + 路径=hub 根 + 名称=hub 目录名」的那个
//! 工作区（就是本工具第一次 `--cwd` 时 Paseo 自动创建的），找到就用
//! `--workspace <id>` —— 每次只是往里新开一个 agent（Paseo 里就是一个新标签页）。
//! 名字同时匹配是为了**不劫持用户自己命名的工作区**（如「查日志」「需求开发」）。
//! 找不到才用 `--cwd` 建一个；下次就能复用。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use tauri::State;
use gws_core::paths;
use gws_core::workspace::Workspace;

use super::{bg, App, CmdResult, CommandError};

/// Paseo 里注册的一个 project（`paseo project ls --json` 的一项）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectEntry {
    name: String,
    path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaseoSession {
    pub branch: String,
    /// 会话的工作目录（= hub 根目录）。
    pub cwd: String,
    /// `paseo --json` 返回的 agent id；没解析出来是 `None`。
    pub agent_id: Option<String>,
    /// 是否已经请求 Paseo 打开这个会话。
    pub opened: bool,
    pub message: String,
}

/// Paseo CLI 是否可用（「开 AI 会话」的前置探测）。
///
/// 没装时前端不必弹 provider 弹窗（弹了也只能报错），而是复制 hub 目录并提示
/// 去安装 —— 那条路径可以直接粘到别的 AI 平台当工作目录。
#[tauri::command]
pub fn paseo_available() -> bool {
    find_paseo().is_some()
}

/// 可选的 Paseo provider（`status=available` 且 `enabled=Enabled`）。
///
/// 前端弹窗要求**必选**：`paseo run` 不传 `--provider` 会直接报
/// `MISSING_PROVIDER`（实测反馈），所以选哪个必须由人明确决定。
#[tauri::command(async)]
pub async fn paseo_providers() -> CmdResult<Vec<String>> {
    bg(providers).await
}

/// 从需求详情开一个 AI 会话。会话的 cwd 是 **hub 根目录**，不是需求目录。
#[tauri::command(async)]
pub async fn paseo_open_session(
    branch: String,
    provider: String,
    app: State<'_, App>,
) -> CmdResult<PaseoSession> {
    let hub = app.hub()?;
    bg(move || open_session(&hub, &branch, &provider)).await
}

/// 开一个**不绑定需求**的会话：适合建需求前聊需求、直接查线上日志。
///
/// 同样是 hub 根目录、同样复用工作区；只是没有 `ws/<分支>`，prompt 也不提分支。
#[tauri::command(async)]
pub async fn paseo_open_free_session(
    title: String,
    provider: String,
    app: State<'_, App>,
) -> CmdResult<PaseoSession> {
    let hub = app.hub()?;
    bg(move || open_free_session(&hub, &title, &provider)).await
}

/// 自由会话的默认标题（用户没填时）。
const FREE_SESSION_TITLE: &str = "临时会话";

fn paseo_binary() -> CmdResult<PathBuf> {
    find_paseo().ok_or_else(|| {
        // Windows 上 Paseo 装出来的是 `paseo.cmd`（桌面端「集成 → CLI 文档 → 命令行」
        // → ~/.local/bin，npm 全局 → %APPDATA%\npm），不是 .exe —— 提示里要把这两条路都写清楚。
        CommandError::plain(if cfg!(windows) {
            "没有找到 paseo。请先安装 Paseo 桌面端，再在「集成」→「CLI 文档」→「命令行」\
             里安装 CLI（装到 %USERPROFILE%\\.local\\bin\\paseo.cmd），\
             或运行 npm install -g @getpaseo/cli；装完重启本应用。"
        } else {
            "没有找到 paseo 命令。请先安装 Paseo；若已安装，确认它在 PATH 或 ~/.local/bin 里。"
        })
    })
}

fn providers() -> CmdResult<Vec<String>> {
    let paseo = paseo_binary()?;
    let out = paseo_command(&paseo)
        .args(["provider", "ls", "--json"])
        .output()
        .map_err(|e| CommandError::plain(format!("运行 paseo provider ls 失败：{e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(CommandError::plain(format!(
            "paseo provider ls 失败：{}",
            stderr.trim()
        )));
    }
    Ok(parse_providers(&String::from_utf8_lossy(&out.stdout)))
}

fn open_session(hub: &gws_core::Hub, branch: &str, provider: &str) -> CmdResult<PaseoSession> {
    let paseo = paseo_binary()?;
    open_session_with(&paseo, hub, branch, provider)
}

fn open_free_session(
    hub: &gws_core::Hub,
    title: &str,
    provider: &str,
) -> CmdResult<PaseoSession> {
    let paseo = paseo_binary()?;
    open_free_session_with(&paseo, hub, title, provider)
}

/// 真正的实现；`paseo` 单独传进来，测试可以用假脚本跑完整流程。
fn open_session_with(
    paseo: &Path,
    hub: &gws_core::Hub,
    branch: &str,
    provider: &str,
) -> CmdResult<PaseoSession> {
    // 校验需求确实存在（读得到 .workspace.json），别对着一个打错的名字开会话
    let ws = Workspace::open(hub, branch)?;
    let branch = ws.branch().to_string();
    // Paseo 标签页默认用**需求描述**（给人看的名字），不是分支名（实测反馈）
    let title = ws.title().to_string();
    let prompt = session_prompt(&branch);
    launch_session(paseo, hub, &title, &branch, &prompt, provider)
}

/// 自由会话的实现：没有需求可校验，标题留空就用默认值。
fn open_free_session_with(
    paseo: &Path,
    hub: &gws_core::Hub,
    title: &str,
    provider: &str,
) -> CmdResult<PaseoSession> {
    let title = free_session_title(title);
    launch_session(paseo, hub, &title, "", &free_prompt(), provider)
}

fn free_session_title(title: &str) -> String {
    let t = title.trim();
    if t.is_empty() {
        FREE_SESSION_TITLE.to_string()
    } else {
        t.to_string()
    }
}

/// 需求会话与自由会话共用的启动路径。
///
/// cwd 固定 hub 根目录；工作区能复用就复用（新会话只是它的一个新标签页）。
fn launch_session(
    paseo: &Path,
    hub: &gws_core::Hub,
    title: &str,
    branch: &str,
    prompt: &str,
    provider: &str,
) -> CmdResult<PaseoSession> {
    let cwd = session_cwd(hub);

    let provider = provider.trim();
    if provider.is_empty() {
        return Err(CommandError::plain("请先选择一个 Paseo provider"));
    }

    // 先让 Paseo 认识 hub 根目录：桌面端项目列表里能看到它，会话才有"出处"。
    // 已注册就原样返回，不会重复建。
    let (project, project_created) = ensure_project(paseo, &cwd)?;

    // 有工具之前建好的工作区就复用（新会话只是它的一个新标签页）；
    // 没有才让 Paseo 按 --cwd 建一个，下次就能找到并复用。
    let workspace = reusable_workspace(paseo, &cwd).unwrap_or(None);

    let out = paseo_command(paseo)
        .args(session_args(
            title,
            prompt,
            provider,
            &cwd,
            workspace.as_ref(),
        ))
        .output()
        .map_err(|e| CommandError::plain(format!("启动 paseo 失败：{e}")))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let detail = stderr.trim();
        return Err(CommandError::plain(format!(
            "paseo 创建会话失败：{}",
            if detail.is_empty() {
                "没有输出错误信息"
            } else {
                detail
            }
        )));
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let agent_id = parse_agent_id(&stdout);
    let opened = agent_id
        .as_deref()
        .map(|id| {
            paseo_command(paseo)
                .args(["agent", "open", id])
                .spawn()
                .is_ok()
        })
        .unwrap_or(false);

    let message = match (&agent_id, opened) {
        (Some(id), true) => format!("已在 Paseo 打开会话 {id}"),
        (Some(id), false) => format!("会话已创建（{id}），请到 Paseo 里打开"),
        (None, _) => "会话已创建，但没解析到 agent id；请到 Paseo 里查看".to_string(),
    };
    let message = match &workspace {
        Some(w) => format!("{message}（复用工作区「{}」）", w.name),
        None => message,
    };
    let message = if project_created {
        format!("已把 hub 注册为 Paseo 项目「{project}」。{message}")
    } else {
        message
    };

    Ok(PaseoSession {
        branch: branch.to_string(),
        cwd: cwd.to_string_lossy().to_string(),
        agent_id,
        opened,
        message,
    })
}

/// 确保 hub 根目录在 Paseo 里是一个 project；返回 `(项目名, 是否是这次新建的)`。
///
/// 先 `paseo project ls --json` 按**规范化后的真实路径**找：命中就不再建。
/// 没命中才 `paseo project create <hub> --json`；create 失败时再列一次兜底
/// （可能刚被别处建过），仍找不到才报错。
fn ensure_project(paseo: &Path, hub_root: &Path) -> CmdResult<(String, bool)> {
    let root = std::fs::canonicalize(hub_root).unwrap_or_else(|_| hub_root.to_path_buf());
    let projects = list_projects(paseo)?;
    if let Some(p) = find_project(&projects, &root) {
        return Ok((p.name.clone(), false));
    }

    let out = paseo_command(paseo)
        .args(["project", "create"])
        .arg(&root)
        .arg("--json")
        .output()
        .map_err(|e| CommandError::plain(format!("注册 Paseo 项目失败：{e}")))?;
    if !out.status.success() {
        // create 可能是"已经存在"：再列一次，能找到就当已注册。
        if let Ok(projects) = list_projects(paseo) {
            if let Some(p) = find_project(&projects, &root) {
                return Ok((p.name.clone(), false));
            }
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(CommandError::plain(format!(
            "注册 Paseo 项目失败：{}",
            stderr.trim()
        )));
    }

    let created =
        project_entries(&String::from_utf8_lossy(&out.stdout)).and_then(|v| v.into_iter().next());
    let name = created
        .map(|p| p.name)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| {
            root.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| root.to_string_lossy().to_string())
        });
    Ok((name, true))
}

fn list_projects(paseo: &Path) -> CmdResult<Vec<ProjectEntry>> {
    let out = paseo_command(paseo)
        .args(["project", "ls", "--json"])
        .output()
        .map_err(|e| CommandError::plain(format!("运行 paseo project ls 失败：{e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(CommandError::plain(format!(
            "paseo project ls 失败：{}",
            stderr.trim()
        )));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    project_entries(&text).ok_or_else(|| {
        CommandError::plain("看不懂 paseo project ls 的输出（不是 JSON 数组），请检查 Paseo 版本")
    })
}

/// 解析 `project ls/create --json` 的结果；格式不认识就返回 `None`。
fn project_entries(json: &str) -> Option<Vec<ProjectEntry>> {
    let v: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    let items: Vec<&serde_json::Value> = match &v {
        serde_json::Value::Array(items) => items.iter().collect(),
        serde_json::Value::Object(_) => vec![&v],
        _ => return None,
    };
    Some(
        items
            .into_iter()
            .filter_map(|it| {
                let path = it.get("path")?.as_str()?.to_string();
                let name = it
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(ProjectEntry { name, path })
            })
            .collect(),
    )
}

/// 按真实路径找 project：两边都能规范化就比规范化的结果（符号链接、
/// `..`、尾斜杠都不影响），否则退化为字符串比较。
fn find_project<'a>(projects: &'a [ProjectEntry], path: &Path) -> Option<&'a ProjectEntry> {
    let want = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    projects.iter().find(|p| {
        let got = Path::new(&p.path);
        let got = std::fs::canonicalize(got).unwrap_or_else(|_| got.to_path_buf());
        got == want
    })
}

/// Paseo 里的一个工作区（`paseo workspace ls --json` 的一项）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceEntry {
    id: String,
    name: String,
    cwd: String,
    isolation: String,
}

/// 找本工具在 hub 根目录建的那个工作区：本地、路径相同、名称等于 hub 目录名。
///
/// 名称必须同时匹配：hub 根下还有用户自己命名的工作区（「查日志」「需求开发」等），
/// 只按路径匹配会把会话塞进别人的工作区。
fn reusable_workspace(paseo: &Path, hub_root: &Path) -> CmdResult<Option<WorkspaceEntry>> {
    let out = paseo_command(paseo)
        .args(["workspace", "ls", "--json"])
        .output()
        .map_err(|e| CommandError::plain(format!("运行 paseo workspace ls 失败：{e}")))?;
    if !out.status.success() {
        // 列不出来不是致命错误：退回 --cwd，让 Paseo 自己建/选工作区
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(workspace_entries(&text).and_then(|all| pick_session_workspace(&all, hub_root).cloned()))
}

/// 解析 `workspace ls --json`；格式不认识就返回 `None`。
fn workspace_entries(json: &str) -> Option<Vec<WorkspaceEntry>> {
    let v: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    let items = v.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|it| {
                Some(WorkspaceEntry {
                    id: it.get("workspaceId")?.as_str()?.to_string(),
                    name: it
                        .get("name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    cwd: it.get("cwd")?.as_str()?.to_string(),
                    isolation: it
                        .get("isolation")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
            })
            .collect(),
    )
}

/// 从工作区列表里挑可复用的那个：第一个「本地 + 路径=hub 根 + 名称=hub 目录名」。
///
/// 不按路径兜底选用户命名的工作区：那些可能有各自的会话与配置，工具不该占用。
fn pick_session_workspace<'a>(
    entries: &'a [WorkspaceEntry],
    hub_root: &Path,
) -> Option<&'a WorkspaceEntry> {
    let want_name = hub_root.file_name()?.to_string_lossy().to_string();
    let want = canonical(hub_root);
    entries.iter().find(|w| {
        w.isolation == "local" && w.name == want_name && canonical(Path::new(&w.cwd)) == want
    })
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// 从 `provider ls --json` 里取所有**可用且启用**的 provider，保持原顺序。
///
/// 前端拿它填选择弹窗，默认选第一个。
fn parse_providers(json: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json.trim()) else {
        return Vec::new();
    };
    let Some(items) = v.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|it| {
            let available = it.get("status").and_then(|x| x.as_str()) == Some("available");
            let enabled = it
                .get("enabled")
                .and_then(|x| x.as_str())
                .map(|s| s.eq_ignore_ascii_case("enabled"))
                .unwrap_or(false);
            if available && enabled {
                it.get("provider")
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
            } else {
                None
            }
        })
        .collect()
}

/// 会话的 cwd：**hub 根目录**。需求目录 `ws/<分支>` 里读不到根目录的
/// `AGENTS.md`，也看不到 `repos/` 里的主干代码（实测反馈）。
fn session_cwd(hub: &gws_core::Hub) -> PathBuf {
    hub.root().to_path_buf()
}

/// 给 AI 的开场说明：**只说分支和一句「直接开始」**。
///
/// 目录语义、只读边界、提交约定都在 hub 根目录的 `AGENTS.md` 里，AI 自己会读 ——
/// 塞进 prompt 只会变成一段每次都要复述的啰嗦话（实测反馈）。
fn session_prompt(branch: &str) -> String {
    format!("当前需求分支：`{branch}`。直接开始，不用复述我知道的信息，等待我下达指示。")
}

/// 自由会话（还没绑定需求）的开场说明。
///
/// 实测需求：先聊清需求再建需求、或直接查线上日志定位 bug；这类会话没有分支，
/// 所以不能套 `session_prompt`（那会让 AI 去找一个不存在的 `ws/<分支>`）。
fn free_prompt() -> String {
    "这是一个还没绑定需求的自由会话。直接开始，不用复述我知道的信息，等待我下达指示。".to_string()
}

/// `paseo run` 的参数。**有可复用工作区时不再传 `--cwd`** —— 工作区已经绑定了
/// 目录，传 `--cwd` 会被当成另建一个，左侧就又多一个同名工作区（实测反馈）。
fn session_args(
    title: &str,
    prompt: &str,
    provider: &str,
    cwd: &Path,
    workspace: Option<&WorkspaceEntry>,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("run"),
        OsString::from("-d"),
        OsString::from("--json"),
    ];
    match workspace {
        Some(w) => {
            args.push("--workspace".into());
            args.push(w.id.clone().into());
        }
        None => {
            args.push("--cwd".into());
            args.push(cwd.as_os_str().to_os_string());
        }
    }
    args.extend([
        OsString::from("--title"),
        OsString::from(title),
        OsString::from("--provider"),
        OsString::from(provider),
        OsString::from(prompt),
    ]);
    args
}

/// 启动 paseo 的统一入口：**清掉宿主进程继承的会话作用域变量**。
///
/// 实测反馈：桌面端从 Paseo 的终端里启动时，环境里带着 `PASEO_AGENT_ID` /
/// `PASEO_AGENT_CWD`；`paseo run` 看到它们就把新会话归到「调用者的工作区」，
/// `--cwd` 被忽略 —— 新会话永远开在宿主会话的目录，而不是当前 hub。
/// `PASEO_HOME` / `PASEO_CLI` 保留：它们只决定连哪个 daemon。
fn paseo_command(paseo: &Path) -> Command {
    paseo_command_for(paseo, cfg!(windows))
}

/// 构造调 paseo 的 `Command`；`windows` 单独传入，好在 mac 上测 Windows 分支。
///
/// Windows 上 Paseo 的 CLI 是 `paseo.cmd`（桌面端「集成 → CLI 文档 → 命令行」或 npm 全局），
/// CreateProcess 不能直接执行批处理 —— `script_command` 会交给 `cmd /C call`，
/// 并隐藏控制台窗口。
fn paseo_command_for(paseo: &Path, windows: bool) -> Command {
    let mut cmd = crate::platform::script_command(paseo, windows);
    for key in ["PASEO_AGENT_ID", "PASEO_AGENT_CWD", "PASEO_WORKSPACE_ID"] {
        cmd.env_remove(key);
    }
    cmd
}

/// Windows 上 Paseo 装出来的是批处理文件，不是 `.exe`：
/// 桌面端「集成 → CLI 文档 → 命令行」→ `%USERPROFILE%\.local\bin\paseo.cmd`，
/// `npm install -g @getpaseo/cli` → `%APPDATA%\npm\paseo.cmd`。
fn paseo_file_names(windows: bool) -> &'static [&'static str] {
    if windows {
        &["paseo.exe", "paseo.cmd", "paseo.bat"]
    } else {
        &["paseo"]
    }
}

/// GUI 进程的 PATH 往往没有这些目录（实测），按常见安装位置兜底。
fn paseo_extra_dirs(
    home: Option<&Path>,
    appdata: Option<&Path>,
    windows: bool,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home {
        dirs.push(home.join(".local/bin"));
        if !windows {
            dirs.push(home.join(".npm-global/bin"));
        }
    }
    if windows {
        // npm 全局 bin 目录（`npm config get prefix` 的结果）
        if let Some(appdata) = appdata {
            dirs.push(appdata.join("npm"));
        }
    } else {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    dirs
}

/// 按 PATH 顺序在目录里找 paseo；同一目录内 `.exe` 优先于 `.cmd` / `.bat`。
fn find_paseo_in(dirs: &[PathBuf], windows: bool) -> Option<PathBuf> {
    for dir in dirs {
        for name in paseo_file_names(windows) {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// 找 paseo。GUI 进程的 PATH 往往没有 `~/.local/bin`（实测），所以按
/// PATH → 常见安装目录（含 Windows 的 `%APPDATA%\npm`）的顺序找。
fn find_paseo() -> Option<PathBuf> {
    let windows = cfg!(windows);
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(paths) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&paths));
    }
    let home = paths::home_dir();
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    dirs.extend(paseo_extra_dirs(home.as_deref(), appdata.as_deref(), windows));
    find_paseo_in(&dirs, windows)
}

/// `paseo run --json` 输出的 agent id。
///
/// 输出是单行 JSON（`--json`）；也容忍 pretty-print 与数组包裹，字段名
/// 不同版本用过 `id` / `agentId` / `agent_id`。
fn parse_agent_id(stdout: &str) -> Option<String> {
    fn pick(v: &serde_json::Value) -> Option<String> {
        let v = match v {
            serde_json::Value::Array(items) => items.first()?,
            other => other,
        };
        ["id", "agentId", "agent_id"]
            .iter()
            .find_map(|k| v.get(*k).and_then(|x| x.as_str()).map(str::to_string))
    }

    for line in stdout.lines().rev() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) {
            if let Some(id) = pick(&v) {
                return Some(id);
            }
        }
    }
    serde_json::from_str::<serde_json::Value>(stdout.trim())
        .ok()
        .and_then(|v| pick(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_id_is_parsed_from_json_variants() {
        assert_eq!(
            parse_agent_id(r#"{"id":"abc-1","status":"running"}"#).as_deref(),
            Some("abc-1")
        );
        assert_eq!(
            parse_agent_id("{\n  \"agentId\": \"abc-2\"\n}\n").as_deref(),
            Some("abc-2")
        );
        assert_eq!(
            parse_agent_id(r#"[{"agent_id":"abc-3"}]"#).as_deref(),
            Some("abc-3")
        );
        assert_eq!(parse_agent_id("started"), None);
        assert_eq!(parse_agent_id(""), None);
    }

    #[test]
    fn providers_are_listed_in_order_and_only_when_available_and_enabled() {
        // 实测反馈：不带 --provider 时 paseo 报 MISSING_PROVIDER。
        // 弹窗只列 status=available 且 enabled=Enabled 的，默认取第一个。
        let json = r#"[
          {"provider":"copilot","status":"unavailable","enabled":"Disabled"},
          {"provider":"opencode","status":"unavailable","enabled":"Enabled"},
          {"provider":"claude","status":"available","enabled":"Enabled"},
          {"provider":"qoder","status":"available","enabled":"Enabled"}
        ]"#;
        assert_eq!(parse_providers(json), vec!["claude", "qoder"]);
        assert!(parse_providers("[]").is_empty());
        assert!(parse_providers("not json").is_empty());
    }

    #[test]
    fn windows_finds_paseo_cmd_not_just_exe() {
        // 实测根因：Windows 上 Paseo 装的是 `paseo.cmd`，只找 `paseo.exe` 会报
        // 「没有可用的 provider」。
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("npm");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("paseo.cmd"), "@echo off\r\n").unwrap();

        let dirs = vec![dir.clone()];
        assert_eq!(find_paseo_in(&dirs, true), Some(dir.join("paseo.cmd")));
        assert!(
            find_paseo_in(&dirs, false).is_none(),
            "非 Windows 只认无扩展名的 paseo"
        );

        // 同一目录里 .exe 优先
        std::fs::write(dir.join("paseo.exe"), "").unwrap();
        assert_eq!(find_paseo_in(&dirs, true), Some(dir.join("paseo.exe")));
    }

    #[test]
    fn extra_dirs_cover_the_windows_install_locations() {
        let dirs = paseo_extra_dirs(
            Some(Path::new("/home/u")),
            Some(Path::new("/appdata")),
            true,
        );
        assert!(dirs.contains(&PathBuf::from("/home/u/.local/bin")), "{dirs:?}");
        assert!(
            dirs.contains(&PathBuf::from("/appdata/npm")),
            "npm 全局 bin 在 %APPDATA%\\npm：{dirs:?}"
        );

        let unix = paseo_extra_dirs(Some(Path::new("/appdata")), None, false);
        assert!(unix.contains(&PathBuf::from("/usr/local/bin")), "{unix:?}");
        assert!(
            unix.contains(&PathBuf::from("/appdata/.npm-global/bin")),
            "{unix:?}"
        );
    }

    #[test]
    fn batch_files_are_invoked_through_cmd_call() {
        // `.cmd` / `.bat` 不能直接 spawn：必须 `cmd /C call <path>`，参数原样透传。
        let batch = Path::new(r"C:\Users\u\.local\bin\paseo.cmd");
        let cmd = paseo_command_for(batch, true);
        assert_eq!(cmd.get_program().to_string_lossy(), "cmd");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            vec![
                "/C".to_string(),
                "call".to_string(),
                r"C:\Users\u\.local\bin\paseo.cmd".to_string()
            ]
        );

        // .exe 与 unix 的 paseo 都直连
        assert_eq!(
            paseo_command_for(Path::new(r"C:\x\paseo.exe"), true)
                .get_program()
                .to_string_lossy(),
            r"C:\x\paseo.exe"
        );
        assert_eq!(
            paseo_command_for(Path::new("/usr/local/bin/paseo"), false)
                .get_program()
                .to_string_lossy(),
            "/usr/local/bin/paseo"
        );
    }

    #[test]
    fn needs_cmd_wrapper_is_case_insensitive_and_windows_only() {
        use crate::platform::needs_cmd_wrapper;
        assert!(needs_cmd_wrapper(Path::new("paseo.CMD"), true));
        assert!(needs_cmd_wrapper(Path::new("paseo.bat"), true));
        assert!(!needs_cmd_wrapper(Path::new("paseo.exe"), true));
        assert!(!needs_cmd_wrapper(Path::new("paseo"), true));
        assert!(!needs_cmd_wrapper(Path::new("paseo.cmd"), false));
    }

    /// 只有 Windows 能真跑：`cmd /C call` 调一个真的 `.cmd`，确认 stdout 拿得到。
    #[cfg(windows)]
    #[test]
    fn batch_file_output_is_actually_read_through_cmd() {
        let tmp = tempfile::TempDir::new().unwrap();
        let script = tmp.path().join("paseo.cmd");
        std::fs::write(
            &script,
            "@echo off\r\necho [{\"provider\":\"claude\",\"status\":\"available\",\"enabled\":\"Enabled\"}]\r\n",
        )
        .unwrap();

        let out = paseo_command_for(&script, true)
            .args(["provider", "ls", "--json"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "cmd /C call 执行失败：{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            parse_providers(&String::from_utf8_lossy(&out.stdout)),
            vec!["claude"]
        );
    }

    #[test]
    fn project_entries_read_both_list_and_single_create_shapes() {
        let list = r#"[
          {"projectId":"prj_1","name":"my-hub","path":"/tmp/hub","kind":"non_git"},
          {"projectId":"prj_2","name":"other","path":"/tmp/other","kind":"git"}
        ]"#;
        assert_eq!(
            project_entries(list).unwrap(),
            vec![
                ProjectEntry {
                    name: "my-hub".into(),
                    path: "/tmp/hub".into()
                },
                ProjectEntry {
                    name: "other".into(),
                    path: "/tmp/other".into()
                },
            ]
        );
        // `project create --json` 返回的是单个对象
        assert_eq!(
            project_entries(r#"{"projectId":"prj_1","name":"my-hub","path":"/tmp/hub"}"#).unwrap(),
            vec![ProjectEntry {
                name: "my-hub".into(),
                path: "/tmp/hub".into()
            }]
        );
        assert!(project_entries("not json").is_none());
        assert!(project_entries("42").is_none());
    }

    #[test]
    fn find_project_compares_real_paths_not_strings() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hub = tmp.path().join("hub");
        std::fs::create_dir_all(&hub).unwrap();
        // 带 `..` 与尾斜杠的写法也应命中（/x/hub/../hub/）
        let messy = format!("{}/../hub/", hub.display());
        let projects = vec![ProjectEntry {
            name: "my-hub".into(),
            path: messy,
        }];
        let found = find_project(&projects, &hub).expect("同一真实路径应命中");
        assert_eq!(found.name, "my-hub");
    }

    #[cfg(unix)]
    #[test]
    fn the_hub_is_registered_as_a_paseo_project_only_once() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let hub = tmp.path().join("hub");
        std::fs::create_dir_all(&hub).unwrap();
        let marker = tmp.path().join("created");
        let script = tmp.path().join("paseo");
        let body = format!(
            r#"#!/bin/sh
if [ "$1" = "project" ] && [ "$2" = "ls" ]; then
  if [ -f '{marker}' ]; then
    echo '[{{"projectId":"prj_1","name":"hub","path":"{hub}","kind":"non_git"}}]'
  else
    echo '[]'
  fi
  exit 0
fi
if [ "$1" = "project" ] && [ "$2" = "create" ]; then
  echo x > '{marker}'
  echo '{{"projectId":"prj_1","name":"hub","path":"'"$3"'","kind":"non_git"}}'
  exit 0
fi
exit 1
"#,
            marker = marker.display(),
            hub = hub.display(),
        );
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let (name, created) = ensure_project(&script, &hub).unwrap();
        assert_eq!(name, "hub");
        assert!(created, "第一次应自动把 hub 注册成 project");
        assert!(marker.is_file(), "应真的调用 project create");

        let (name, created) = ensure_project(&script, &hub).unwrap();
        assert_eq!(name, "hub");
        assert!(!created, "已注册就不该重复创建");
    }

    #[test]
    fn prompt_is_short_and_only_names_the_branch() {
        // 实测反馈：每次开新会话都让 AI「先读这个、先看那个」，罗里吧嗦；
        // 目录语义由根目录 AGENTS.md 负责，prompt 只说分支 + 直接开始。
        let p = session_prompt("feature-20261021-x");
        assert!(p.contains("当前需求分支：`feature-20261021-x`"));
        assert!(p.contains("直接开始"));
        assert!(!p.contains("AGENTS.md"), "不再让 AI 先去读文件：{p}");
        assert!(!p.contains("先简要说明"), "不要复述已知信息：{p}");
        assert!(!p.contains("ws/"), "不再罗列目录：{p}");
        assert!(p.chars().count() < 60, "开场说明要短：{p}");
    }

    #[test]
    fn workspace_entries_read_the_ls_json() {
        let json = r#"[
          {"workspaceId":"wks_1","project":"my-hub","name":"my-hub","isolation":"local","cwd":"/tmp/hub"},
          {"workspaceId":"wks_2","project":"my-hub","name":"查日志","isolation":"local","cwd":"/tmp/hub"}
        ]"#;
        let entries = workspace_entries(json).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "wks_1");
        assert_eq!(entries[1].name, "查日志");
        assert!(workspace_entries("not json").is_none());
        assert!(workspace_entries(r#"{"workspaceId":"x"}"#).is_none());
    }

    #[test]
    fn the_tool_created_workspace_is_reused_and_user_named_ones_are_left_alone() {
        // 实测反馈：每次派发都新建一个 my-hub，左侧堆了一排同名工作区。
        // 规则：本地 + 同一路径 + 名称等于 hub 目录名 → 就是工具第一次建的，复用。
        let tmp = tempfile::TempDir::new().unwrap();
        let hub = tmp.path().join("my-hub");
        let other = tmp.path().join("other");
        std::fs::create_dir_all(&hub).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let ws = |id: &str, name: &str, cwd: &Path, iso: &str| WorkspaceEntry {
            id: id.into(),
            name: name.into(),
            cwd: cwd.to_string_lossy().to_string(),
            isolation: iso.into(),
        };
        let entries = vec![
            ws("w_user", "查日志", &hub, "local"),
            ws("w_other", "my-hub", &other, "local"),
            ws("w_wt", "my-hub", &hub, "worktree"),
            ws("w_ours", "my-hub", &hub, "local"),
        ];
        let picked = pick_session_workspace(&entries, &hub).expect("应复用工具建的工作区");
        assert_eq!(picked.id, "w_ours");
        assert!(
            pick_session_workspace(&entries[..1], &hub).is_none(),
            "只有用户自己命名的工作区时不能占用它"
        );
    }

    #[test]
    fn reuse_workspace_means_a_new_tab_not_a_new_workspace() {
        let cwd = Path::new("/tmp/hub");
        let ws = WorkspaceEntry {
            id: "wks_1".into(),
            name: "hub".into(),
            cwd: "/tmp/hub".into(),
            isolation: "local".into(),
        };
        let prompt = session_prompt("feature-20261021-x");
        let args = session_args("手机号加密需求2期-3", &prompt, "claude", cwd, Some(&ws));
        let strs: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert!(
            strs.windows(2).any(|w| w == ["--workspace", "wks_1"]),
            "应复用工作区：{strs:?}"
        );
        assert!(
            !strs.iter().any(|a| a == "--cwd"),
            "复用时不能再传 --cwd，否则会另建一个：{strs:?}"
        );
        // Tab 标签用需求描述；分支只出现在 prompt 里
        assert!(
            strs.windows(2)
                .any(|w| w == ["--title", "手机号加密需求2期-3"]),
            "标签页标题应是需求描述：{strs:?}"
        );
        assert_eq!(strs.last().unwrap(), &prompt);

        let args = session_args("手机号加密需求2期-3", &prompt, "claude", cwd, None);
        let strs: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert!(
            strs.windows(2).any(|w| w == ["--cwd", "/tmp/hub"]),
            "没有可复用工作区时才用 --cwd：{strs:?}"
        );
        assert!(!strs.iter().any(|a| a == "--workspace"));
    }

    #[test]
    fn free_session_title_falls_back_when_empty() {
        assert_eq!(free_session_title(""), FREE_SESSION_TITLE);
        assert_eq!(free_session_title("   "), FREE_SESSION_TITLE);
        assert_eq!(free_session_title("  查线上日志  "), "查线上日志");
    }

    #[test]
    fn free_prompt_has_no_branch_and_stays_short() {
        // 自由会话没有分支：不能像需求会话那样让 AI 去找 ws/<分支>；
        // 也不罗列目录（AGENTS.md 里已经有）。
        let p = free_prompt();
        assert!(!p.contains("当前需求分支"), "{p}");
        assert!(p.contains("还没绑定需求"));
        assert!(p.contains("直接开始"));
        assert!(!p.contains("AGENTS.md"), "{p}");
        assert!(!p.contains("repos/"), "{p}");
        assert!(p.chars().count() < 60, "开场说明要短：{p}");
    }

    #[cfg(unix)]
    #[test]
    fn a_free_session_uses_the_typed_title_and_no_branch() {
        use std::os::unix::fs::PermissionsExt;

        // 实测需求：建需求前先聊、或直接查线上日志。标签页标题由人填，
        // 没分支可校验，也不能把「当前需求分支」塞进 prompt。
        let tmp = tempfile::TempDir::new().unwrap();
        let hub = gws_core::Hub::init(
            &tmp.path().join("hub"),
            gws_core::hub::InitOptions::default(),
        )
        .unwrap();

        let log = tmp.path().join("args.log");
        let script = tmp.path().join("paseo");
        let hub_root = hub.root().display().to_string();
        let body = format!(
            "#!/bin/sh\n\
             echo \"$@\" >> '{log}'\n\
             if [ \"$1\" = \"project\" ] && [ \"$2\" = \"ls\" ]; then echo '[{{\"projectId\":\"p1\",\"name\":\"hub\",\"path\":\"{hub_root}\"}}]'; exit 0; fi\n\
             if [ \"$1\" = \"workspace\" ] && [ \"$2\" = \"ls\" ]; then echo '[{{\"workspaceId\":\"wks_1\",\"name\":\"hub\",\"isolation\":\"local\",\"cwd\":\"{hub_root}\"}}]'; exit 0; fi\n\
             if [ \"$1\" = \"run\" ]; then echo '{{\"id\":\"agent-free\"}}'; exit 0; fi\n\
             if [ \"$1\" = \"agent\" ] && [ \"$2\" = \"open\" ]; then exit 0; fi\n\
             exit 1\n",
            log = log.display(),
            hub_root = hub_root,
        );
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let out = open_free_session_with(&script, &hub, "查线上日志", "claude")
            .expect("应能创建自由会话");
        assert_eq!(out.branch, "", "自由会话没有分支");
        assert_eq!(out.agent_id.as_deref(), Some("agent-free"));

        let logged = std::fs::read_to_string(&log).unwrap();
        assert!(
            logged.contains("--title 查线上日志 --provider claude"),
            "标签页标题应是用户填的：{logged}"
        );
        assert!(
            logged.contains("--workspace wks_1"),
            "应复用工作区：{logged}"
        );
        assert!(
            logged.contains("还没绑定需求"),
            "prompt 应是自由会话版：{logged}"
        );
        assert!(!logged.contains("当前需求分支"), "{logged}");
    }

    #[cfg(unix)]
    #[test]
    fn the_agent_title_defaults_to_the_requirement_title() {
        use std::os::unix::fs::PermissionsExt;

        // 实测反馈：Paseo 左侧标签一直是分支名，看不出是哪个需求。
        // 这里跑一遍完整的 open_session：`.workspace.json` 的 title 必须进 --title。
        let tmp = tempfile::TempDir::new().unwrap();
        let hub = gws_core::Hub::init(
            &tmp.path().join("hub"),
            gws_core::hub::InitOptions::default(),
        )
        .unwrap();
        let branch = "feature-20261021-tab-title";
        let ws_dir = hub.ws_dir().join(branch);
        std::fs::create_dir_all(&ws_dir).unwrap();
        gws_core::workspace::WorkspaceMeta::new(
            branch,
            "手机号加密需求2期-3",
            "feature",
            vec![],
            vec![],
        )
        .save(&ws_dir.join(gws_core::workspace::META_FILE))
        .unwrap();

        let log = tmp.path().join("args.log");
        let projects = tmp.path().join("projects.json");
        let run = tmp.path().join("run.json");
        std::fs::write(
            &projects,
            format!(
                r#"[{{"projectId":"p1","name":"hub","path":"{}"}}]"#,
                hub.root().display()
            ),
        )
        .unwrap();
        std::fs::write(&run, r#"{"id":"agent-1"}"#).unwrap();

        let script = tmp.path().join("paseo");
        let body = format!(
            "#!/bin/sh\n\
             echo \"$@\" >> '{log}'\n\
             if [ \"$1\" = \"project\" ] && [ \"$2\" = \"ls\" ]; then cat '{projects}'; exit 0; fi\n\
             if [ \"$1\" = \"project\" ] && [ \"$2\" = \"create\" ]; then cat '{projects}'; exit 0; fi\n\
             if [ \"$1\" = \"workspace\" ] && [ \"$2\" = \"ls\" ]; then echo '[]'; exit 0; fi\n\
             if [ \"$1\" = \"run\" ]; then cat '{run}'; exit 0; fi\n\
             if [ \"$1\" = \"agent\" ] && [ \"$2\" = \"open\" ]; then exit 0; fi\n\
             exit 1\n",
            log = log.display(),
            projects = projects.display(),
            run = run.display(),
        );
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let out = open_session_with(&script, &hub, branch, "claude").expect("应能创建会话");
        assert_eq!(out.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(out.cwd, hub.root().to_string_lossy());

        let logged = std::fs::read_to_string(&log).unwrap();
        assert!(
            logged.contains("--title 手机号加密需求2期-3 --provider claude"),
            "标签页标题应是需求描述：{logged}"
        );
        assert!(
            !logged.contains("--title feature-20261021-tab-title"),
            "不该再用分支名当标题：{logged}"
        );
        // prompt 里仍然要指明分支（AI 靠它找 ws/<分支>）
        assert!(
            logged.contains("当前需求分支：`feature-20261021-tab-title`"),
            "{logged}"
        );
    }

    #[test]
    fn paseo_calls_strip_the_host_session_scope() {
        // 实测反馈：桌面端从 Paseo 会话里启动时，`paseo run` 会把新会话塞进
        // 宿主工作区、忽略 --cwd。这三个变量必须显式清掉（None = 删除）。
        let cmd = paseo_command(Path::new("/tmp/paseo"));
        let removed: Vec<String> = cmd
            .get_envs()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.to_string_lossy().to_string())
            .collect();
        for key in ["PASEO_AGENT_ID", "PASEO_AGENT_CWD", "PASEO_WORKSPACE_ID"] {
            assert!(
                removed.iter().any(|k| k == key),
                "应清掉 {key}，实测 {removed:?}"
            );
        }
        // 连哪个 daemon 的变量不能动，否则会连不上本机 Paseo
        assert!(!removed.iter().any(|k| k == "PASEO_HOME"));
        assert!(!removed.iter().any(|k| k == "PASEO_CLI"));
    }

    #[test]
    fn session_cwd_is_the_hub_root_not_the_requirement_dir() {
        // 实测反馈：开在需求目录里，AI 读不到根目录的 CLAUDE.md / AGENTS.md，
        // 也看不到 repos/ 里的主干代码。这个断言钉住「永远是 hub 根」。
        let tmp = tempfile::TempDir::new().unwrap();
        let hub = gws_core::Hub::init(
            &tmp.path().join("hub"),
            gws_core::hub::InitOptions::default(),
        )
        .unwrap();
        assert_eq!(session_cwd(&hub), hub.root());
        assert_ne!(session_cwd(&hub), hub.ws_dir().join("feature-20261021-x"));
    }
}
