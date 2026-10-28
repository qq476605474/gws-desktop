//! 平台相关能力：打开文件管理器 / 终端 / 浏览器、写剪贴板、读写本地文件。
//!
//! 这些是**唯一**需要感知操作系统的地方（core 里已全部收敛到 `paths`）。
//! 三个平台都要支持：macOS / Windows / Linux。
//!
//! 从开源版 port 过来并做了删减：**去掉了「检查 gws 是否安装」与「查 GitHub
//! 最新版本」**——本版内嵌 core，不再依赖任何外部程序，那两项已无意义。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

#[derive(Debug, Clone, PartialEq)]
pub enum TerminalPreference {
    FollowSystem,
    Force(String),
}

/// 从 GUI 进程起**控制台子进程**时，Windows 默认会弹一个黑窗口（抢焦点）。
///
/// `git` 在 core 里已经统一加了 `CREATE_NO_WINDOW`；这里管桌面端自己起的
/// 短命令：`gws --version`（每次启动同步 CLI 时跑一次，实测反馈“打开闪一下
/// cmd”）、`cmd /C call paseo.cmd`、自定义编辑器命令。
pub(crate) fn hide_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// `.cmd` / `.bat` 不能直接 `Command::new`：Windows 的 CreateProcess 只认可执行文件。
///
/// 两类程序会撞上：Paseo 的 CLI（`paseo.cmd`）、VS Code 系的编辑器入口
/// （`code.cmd` / `cursor.cmd` / `qoder.cmd` / `windsurf.cmd`）。
pub(crate) fn needs_cmd_wrapper(path: &Path, windows: bool) -> bool {
    if !windows {
        return false;
    }
    path.extension()
        .map(|ext| {
            let ext = ext.to_string_lossy().to_ascii_lowercase();
            ext == "cmd" || ext == "bat"
        })
        .unwrap_or(false)
}

/// 建一条能执行 `.exe` / `.cmd` / `.bat` 的命令；批处理走 `cmd /C call`。
/// 一律隐藏控制台窗口（GUI 进程里每一个黑窗口都会闪一下、抢焦点）。
pub(crate) fn script_command(path: &Path, windows: bool) -> Command {
    let mut cmd = if needs_cmd_wrapper(path, windows) {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg("call").arg(path);
        cmd
    } else {
        Command::new(path)
    };
    hide_console(&mut cmd);
    cmd
}

/// PATH 查找可执行文件。
fn find_in_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path).map(|dir| dir.join(name)).find(|c| c.is_file())
    })
}

/// macOS `.app` 是否安装（系统或用户 Applications）。
fn app_installed(name: &str) -> bool {
    if Path::new("/Applications").join(format!("{name}.app")).exists() {
        return true;
    }
    std::env::var_os("HOME").is_some_and(|home| {
        Path::new(&home).join("Applications").join(format!("{name}.app")).exists()
    })
}

fn iterm_installed() -> bool {
    app_installed("iTerm")
}

/// 跟随系统时用哪个终端。
pub fn detect_terminal(pref: &TerminalPreference) -> String {
    match pref {
        TerminalPreference::Force(name) => name.clone(),
        TerminalPreference::FollowSystem => {
            if cfg!(target_os = "macos") {
                if iterm_installed() {
                    "iTerm2".to_string()
                } else {
                    "Terminal.app".to_string()
                }
            } else if cfg!(target_os = "linux") {
                for name in ["gnome-terminal", "konsole", "xfce4-terminal", "x-terminal-emulator"] {
                    if find_in_path(name).is_some() {
                        return name.to_string();
                    }
                }
                "x-terminal-emulator".to_string()
            } else {
                "wt".to_string()
            }
        }
    }
}

/// 等待命令退出并捕获输出。
///
/// 失败时附 stderr 尾部一行——osascript 的语法错误、应用未找到等真实原因只会
/// 出现在子进程 stderr，不捕获就只剩一个无从排查的退出码。
fn run_status(cmd: &mut Command, err_prefix: &str) -> Result<(), String> {
    let output = cmd.output().map_err(|e| format!("{err_prefix}: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let reason = match output.status.code() {
        Some(code) => format!("{err_prefix}: 退出码 {code}"),
        None => format!("{err_prefix}: 进程被信号终止"),
    };
    Err(match stderr_tail_line(&output.stderr) {
        Some(tail) => format!("{reason}（{tail}）"),
        None => reason,
    })
}

fn stderr_tail_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

/// Windows 路径规范化：`/` → `\`，连续分隔符压成一个（保留 UNC 开头 `\\`）。
///
/// explorer 只认反斜杠：正斜杠或双反斜杠都会解析失败，静默回退打开「文档」。
pub fn normalize_win_path(path: &str) -> String {
    let p = path.replace('/', "\\");
    if let Some(rest) = p.strip_prefix("\\\\") {
        format!("\\\\{}", rest.replace("\\\\", "\\"))
    } else {
        p.replace("\\\\", "\\")
    }
}

// ══ 打开 ═══════════════════════════════════════════════════════════════

/// 打开前的存在性预检。
///
/// # 为什么必须自己判
///
/// 路径不存在时，**三个平台都不会给出前端能看见的反馈**：
/// `open` 的报错只落在子进程 stderr，`explorer` 更是恒以退出码 1 返回
/// 且会转头去开「文档」目录，`xdg-open` 同样只写 stderr。
/// 结果就是点了按钮什么都没发生（实测反馈：环境页点「文件管理」没反应 ——
/// 那个环境还没同步过，`envs/<环境>/<模块>` 根本不存在）。
///
/// 所以先自己判一下，当场把原因说清楚，而不是让它静默失败。
fn ensure_exists(path: &str) -> Result<(), String> {
    if Path::new(path).exists() {
        return Ok(());
    }
    Err(format!("目录不存在：{path}（可能还没检出到这里）"))
}

/// (async)：open 触发自动化权限弹窗时可阻塞至用户响应（最长约 2 分钟），
/// 必须放线程池，否则整个 GUI 冻结。
#[tauri::command(async)]
pub fn open_in_finder(path: String) -> Result<(), String> {
    ensure_exists(&path)?;
    if cfg!(target_os = "windows") {
        // explorer 无论成败恒以退出码 1 返回，不能按退出码判错——spawn 成功即视为已打开
        Command::new("explorer")
            .arg(normalize_win_path(&path))
            .spawn()
            .map_err(|e| format!("打开目录失败: {e}"))?;
        return Ok(());
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "linux") {
        "xdg-open"
    } else {
        return Err("不支持的平台".to_string());
    };
    run_status(Command::new(program).arg(&path), "打开目录失败")
}

/// 用系统默认应用打开任意路径：目录→文件管理器，文件→关联应用。
#[tauri::command(async)]
pub fn open_path(path: String) -> Result<(), String> {
    ensure_exists(&path)?;
    if cfg!(target_os = "windows") {
        return tauri_plugin_opener::open_path(normalize_win_path(&path), None::<&str>)
            .map_err(|e| format!("打开失败: {e}"));
    }
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(target_os = "linux") {
        Command::new("xdg-open")
    } else {
        return Err("不支持的平台".to_string());
    };
    cmd.arg(&path);
    run_status(&mut cmd, "打开失败")
}

/// 用默认浏览器打开 URL（需求 #7：点击发布地址跳转）。
#[tauri::command(async)]
pub fn open_url(url: String) -> Result<(), String> {
    // 只允许 http(s)：file:// 之类会被系统当成「打开本地文件」，
    // 让一个配置错误变成意外行为
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(format!("只支持 http/https 链接: {url}"));
    }
    if cfg!(target_os = "windows") {
        return tauri_plugin_opener::open_url(&url, None::<&str>)
            .map_err(|e| format!("打开链接失败: {e}"));
    }
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    run_status(Command::new(program).arg(&url), "打开链接失败")
}

/// 写系统剪贴板。
///
/// 走 Rust 侧而不是 JS 插件 API：WKWebView 的 JS 剪贴板层在部分环境静默失效
/// （点击无反应且无错误可见），Rust 直写 pasteboard 稳定。
#[tauri::command(async)]
pub fn copy_text(text: String, app: AppHandle) -> Result<(), String> {
    app.clipboard().write_text(text).map_err(|e| format!("复制失败: {e}"))
}

// ══ 终端 ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TerminalOption {
    pub id: String,
    pub label: String,
}

/// 设置面板的终端候选：按当前 OS 给出该 OS 合理的列表（只列实际安装的）。
#[tauri::command]
pub fn terminal_options() -> Vec<TerminalOption> {
    let mut opts = Vec::new();
    let mut push = |id: &str, label: String| {
        opts.push(TerminalOption { id: id.to_string(), label });
    };
    let detected = detect_terminal(&TerminalPreference::FollowSystem);
    if cfg!(target_os = "macos") {
        push("system", format!("跟随系统（当前 {detected}）"));
        if iterm_installed() {
            push("iTerm2", "iTerm2".to_string());
        }
        push("Terminal.app", "Terminal.app".to_string());
        if app_installed("Warp") {
            push("Warp", "Warp".to_string());
        }
    } else if cfg!(target_os = "linux") {
        push("system", format!("跟随系统（当前 {detected}）"));
        for name in ["gnome-terminal", "konsole", "xfce4-terminal", "x-terminal-emulator"] {
            if find_in_path(name).is_some() {
                push(name, name.to_string());
            }
        }
    } else {
        // Windows：open_in_terminal 不读 terminal 参数（恒 wt→cmd 回退），
        // 与其给出不起作用的选项，不如只留跟随系统
        push("system", format!("跟随系统（当前 {detected}）"));
    }
    opts
}

/// 第一层（shell）：路径包成单引号 token，形如 `'it'\''s'`。
pub fn shell_quote_path(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// 第二层（AppleScript）：嵌入 AppleScript 双引号字符串前的转义。
/// AppleScript 只认 `\"` 与 `\\`，`\'` 是语法错误（osacompile 报 -2741）。
pub fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// 构造 macOS AppleScript；`None` 表示该终端不支持 AppleScript（如 Warp）。
pub fn macos_terminal_script(terminal: &str, path: &str) -> Option<String> {
    if terminal == "Warp" {
        return None;
    }
    let cmd = format!("cd {}", shell_quote_path(path));
    if terminal == "iTerm2" {
        // 不能用 `create window with default profile command "<cmd>"`：command 参数会
        // 替换该会话的 shell 命令，cd 毫秒级退出 → 会话立即结束、窗口闪退且无报错。
        // 官方模式是先建普通交互会话，再 write text 把命令写进 tty 输入缓冲。
        Some(format!(
            "tell application \"iTerm\"\n\
             activate\n\
             set newWindow to (create window with default profile)\n\
             tell current session of newWindow\n\
             write text \"{}\"\n\
             end tell\n\
             end tell",
            applescript_escape(&cmd)
        ))
    } else {
        Some(format!(
            "tell application \"Terminal\"\n\
             activate\n\
             do script \"{}\"\n\
             end tell",
            applescript_escape(&cmd)
        ))
    }
}

/// (async)：osascript/open 可能阻塞在主线程外（自动化权限弹窗最长约 2 分钟）。
///
/// # 「跟随系统」有两种表示，必须都认
///
/// 前端存的是字符串 `"system"`（`terminalOptions` 里给的 id 就是它），
/// 而这里原来只在 `None` 时才去自动检测 —— 于是 `"system"` 直接掉进最后那个
/// else 分支，**永远打开 Terminal.app**，用户在设置里选 iTerm2 也一样
/// （实测反馈：设了跟随系统 / iTerm2，打开的还是终端.app）。
///
/// 现在 `None` / `"system"` / 空串一律按「跟随系统」解析。
#[tauri::command(async)]
pub fn open_in_terminal(path: String, terminal: Option<String>) -> Result<(), String> {
    // 同 open_in_finder：路径不存在时 `osascript` 里的 `cd` 会失败，
    // 终端照样打开但停在 $HOME —— 看着像「打开了」，其实去错了地方。
    ensure_exists(&path)?;
    let pref = terminal.unwrap_or_default();
    let pref = pref.trim();
    let terminal = if pref.is_empty() || pref == "system" {
        detect_terminal(&TerminalPreference::FollowSystem)
    } else {
        pref.to_string()
    };
    if cfg!(target_os = "macos") {
        match macos_terminal_script(&terminal, &path) {
            Some(script) => {
                run_status(Command::new("osascript").arg("-e").arg(&script), "打开终端失败")
            }
            None => run_status(Command::new("open").arg("-a").arg("Warp").arg(&path), "打开终端失败"),
        }
    } else if cfg!(target_os = "linux") {
        match terminal.as_str() {
            "gnome-terminal" | "xfce4-terminal" => run_status(
                Command::new(terminal.as_str()).arg("--working-directory").arg(&path),
                "打开终端失败",
            ),
            // konsole 的工作目录参数是 --workdir，不是 --working-directory
            "konsole" => run_status(
                Command::new("konsole").arg("--workdir").arg(&path),
                "打开终端失败",
            ),
            _ => {
                let mut cmd = Command::new("x-terminal-emulator");
                run_status(&mut cmd, "打开终端失败")
            }
        }
    } else if cfg!(target_os = "windows") {
        if run_status(Command::new("wt").arg("-d").arg(&path), "打开终端失败").is_ok() {
            return Ok(());
        }
        let mut cmd = Command::new("cmd");
        cmd.arg("/K").arg("cd").arg("/d").arg(&path);
        run_status(&mut cmd, "打开终端失败")
    } else {
        Err("不支持的平台".to_string())
    }
}

// ══ 编辑器 ═══════════════════════════════════════════════════════════

/// 设置面板里的「用编辑器打开」候选。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EditorOption {
    pub id: String,
    pub label: String,
    /// 本机是否找到可执行入口（找不到也能选，点了会给明确提示）。
    pub installed: bool,
}

/// 编辑器清单。
///
/// # 为什么要写死 app 包名/包内路径
///
/// 从 Finder 启动的 GUI 进程**没有用户 shell 的 PATH**（实测：Tauri 里
/// `/usr/bin:/bin:/usr/sbin:/sbin`），所以 `code` / `idea` 在终端里能用、
/// 在应用里却找不到。macOS 上直接调 App 包内的 launcher 最稳：
/// JetBrains 在 `Contents/MacOS/<cli>`，VS Code 系在
/// `Contents/Resources/app/bin/<cli>`；找不到 launcher 时退回 `open -a`。
struct EditorPreset {
    id: &'static str,
    label: &'static str,
    /// macOS 应用名（不带 `.app`），CE 版等别名都列上。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    mac_apps: &'static [&'static str],
    /// macOS 包内可执行文件名。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    mac_bin: &'static str,
    /// VS Code 系的包内布局是 `Resources/app/bin/<bin>`。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    vscode_like: bool,
    /// Linux / Windows 的 PATH 命令名。
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    cli: &'static str,
}

const EDITORS: &[EditorPreset] = &[
    EditorPreset {
        id: "idea",
        label: "IntelliJ IDEA",
        mac_apps: &["IntelliJ IDEA", "IntelliJ IDEA CE"],
        mac_bin: "idea",
        vscode_like: false,
        cli: "idea",
    },
    EditorPreset {
        id: "webstorm",
        label: "WebStorm",
        mac_apps: &["WebStorm"],
        mac_bin: "webstorm",
        vscode_like: false,
        cli: "webstorm",
    },
    EditorPreset {
        id: "pycharm",
        label: "PyCharm",
        mac_apps: &["PyCharm", "PyCharm CE"],
        mac_bin: "pycharm",
        vscode_like: false,
        cli: "pycharm",
    },
    EditorPreset {
        id: "goland",
        label: "GoLand",
        mac_apps: &["GoLand"],
        mac_bin: "goland",
        vscode_like: false,
        cli: "goland",
    },
    EditorPreset {
        id: "phpstorm",
        label: "PhpStorm",
        mac_apps: &["PhpStorm"],
        mac_bin: "phpstorm",
        vscode_like: false,
        cli: "phpstorm",
    },
    EditorPreset {
        id: "clion",
        label: "CLion",
        mac_apps: &["CLion"],
        mac_bin: "clion",
        vscode_like: false,
        cli: "clion",
    },
    EditorPreset {
        id: "rubymine",
        label: "RubyMine",
        mac_apps: &["RubyMine"],
        mac_bin: "rubymine",
        vscode_like: false,
        cli: "rubymine",
    },
    EditorPreset {
        id: "rider",
        label: "Rider",
        mac_apps: &["Rider"],
        mac_bin: "rider",
        vscode_like: false,
        cli: "rider",
    },
    EditorPreset {
        id: "datagrip",
        label: "DataGrip",
        mac_apps: &["DataGrip"],
        mac_bin: "datagrip",
        vscode_like: false,
        cli: "datagrip",
    },
    EditorPreset {
        id: "vscode",
        label: "VS Code",
        mac_apps: &["Visual Studio Code"],
        mac_bin: "code",
        vscode_like: true,
        cli: "code",
    },
    EditorPreset {
        id: "cursor",
        label: "Cursor",
        mac_apps: &["Cursor"],
        mac_bin: "cursor",
        vscode_like: true,
        cli: "cursor",
    },
    EditorPreset {
        id: "qoder",
        label: "Qoder",
        mac_apps: &["Qoder", "Qoder IDE"],
        mac_bin: "qoder",
        vscode_like: true,
        cli: "qoder",
    },
    EditorPreset {
        id: "zed",
        label: "Zed",
        // Zed 包内的 CLI 叫 cli，主程序 zed 不会转发参数
        mac_apps: &["Zed"],
        mac_bin: "cli",
        vscode_like: false,
        cli: "zed",
    },
    EditorPreset {
        id: "windsurf",
        label: "Windsurf",
        mac_apps: &["Windsurf"],
        mac_bin: "windsurf",
        vscode_like: true,
        cli: "windsurf",
    },
    EditorPreset {
        id: "rebased",
        label: "Rebased",
        // IntelliJ 平台的 Git 客户端：包内 launcher 就是 `rebased`
        mac_apps: &["Rebased"],
        mac_bin: "rebased",
        vscode_like: false,
        cli: "rebased",
    },
];

/// 没设置时用 IDEA（用户指定的默认值）。
pub const DEFAULT_EDITOR: &str = "idea";

/// 解析后的启动入口。
#[derive(Debug, Clone, PartialEq)]
enum Launcher {
    /// 直接执行（App 包内 launcher / PATH 命令）。
    Path(PathBuf),
    /// 只有 App 包、没有可执行入口：走 `open -a <app> <path>`。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    MacApp(PathBuf),
}

fn preset(id: &str) -> Option<&'static EditorPreset> {
    EDITORS.iter().find(|p| p.id == id)
}

/// 空/未设置一律回落到 IDEA。
fn editor_id(input: Option<String>) -> String {
    let trimmed = input.unwrap_or_default();
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        DEFAULT_EDITOR.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(target_os = "macos")]
fn mac_app_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join("Applications"));
    }
    dirs
}

/// 在给定目录里找 App 包（纯函数，便于在 mac 上测 Windows 那套目录结构之外的逻辑）。
#[cfg(target_os = "macos")]
fn mac_editor_in(dirs: &[PathBuf], p: &EditorPreset) -> Option<Launcher> {
    for dir in dirs {
        for app in p.mac_apps {
            let bundle = dir.join(format!("{app}.app"));
            if !bundle.is_dir() {
                continue;
            }
            let bin = if p.vscode_like {
                bundle.join("Contents/Resources/app/bin").join(p.mac_bin)
            } else {
                bundle.join("Contents/MacOS").join(p.mac_bin)
            };
            if bin.is_file() {
                return Some(Launcher::Path(bin));
            }
            // 包在、入口不在（用户删过 launcher）：让 `open -a` 兜底
            return Some(Launcher::MacApp(bundle));
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn resolve_editor(p: &EditorPreset) -> Option<Launcher> {
    mac_editor_in(&mac_app_dirs(), p)
}

/// 有界递归找文件（Windows 的 JetBrains Toolbox 版本号目录没法写死）。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn find_file_under(root: &Path, name: &str, max_depth: usize) -> Option<PathBuf> {
    let mut queue = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if ft.is_file() && entry.file_name().to_str() == Some(name) {
                return Some(path);
            }
            if ft.is_dir() && depth < max_depth {
                queue.push((path, depth + 1));
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn resolve_editor(p: &EditorPreset) -> Option<Launcher> {
    let mut candidates = Vec::new();
    if let Some(found) = find_in_path(p.cli) {
        candidates.push(found);
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(Path::new(&home).join(".local/bin").join(p.cli));
    }
    for dir in ["/usr/local/bin", "/usr/bin", "/snap/bin"] {
        candidates.push(Path::new(dir).join(p.cli));
    }
    candidates
        .into_iter()
        .find(|c| c.is_file() && !(p.id == "qoder" && c.components().any(|part| part.as_os_str() == ".qoder")))
        .map(Launcher::Path)
}

#[cfg(target_os = "windows")]
fn resolve_editor(p: &EditorPreset) -> Option<Launcher> {
    // PATH 里的 .cmd / .exe
    for name in [format!("{}.cmd", p.cli), format!("{}.exe", p.cli)] {
        if let Some(found) = find_in_path(&name) {
            return Some(Launcher::Path(found));
        }
    }
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let programs = std::env::var_os("ProgramFiles").map(PathBuf::from);
    // 已知安装位置：winget 默认装到 ProgramFiles，部分安装器装到用户目录
    for rel in windows_known_editor_paths(p.id) {
        for root in [local.as_ref(), programs.as_ref()].into_iter().flatten() {
            let candidate = root.join(rel);
            if candidate.is_file() {
                return Some(Launcher::Path(candidate));
            }
        }
    }
    // Rebased 是 IntelliJ 平台，目录布局可能带版本号，兜底递归找 launcher
    if p.id == "rebased" {
        let mut roots = Vec::new();
        if let Some(root) = &programs {
            roots.push(root.join("detachhead/Rebased"));
        }
        if let Some(root) = &local {
            roots.push(root.join("Programs/Rebased"));
        }
        for root in roots {
            for exe in ["rebased64.exe", "rebased.exe"] {
                if let Some(found) = find_file_under(&root, exe, 4) {
                    return Some(Launcher::Path(found));
                }
            }
        }
    }
    // JetBrains：Toolbox 的版本号目录只能递归找
    let is_jetbrains = matches!(
        p.id,
        "idea" | "webstorm" | "pycharm" | "goland" | "phpstorm" | "clion" | "rubymine" | "rider" | "datagrip"
    );
    if is_jetbrains {
        let exe = format!("{}64.exe", p.cli);
        let mut roots = Vec::new();
        if let Some(root) = &local {
            roots.push(root.join("JetBrains/Toolbox/apps"));
        }
        if let Some(root) = &programs {
            roots.push(root.join("JetBrains"));
        }
        for root in roots {
            if let Some(found) = find_file_under(&root, &exe, 8) {
                return Some(Launcher::Path(found));
            }
        }
    }
    None
}

/// Windows 上的常见安装位置（相对 `%LOCALAPPDATA%` / `%ProgramFiles%`）。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn windows_known_editor_paths(id: &str) -> &'static [&'static str] {
    match id {
        "vscode" => &["Programs/Microsoft VS Code/bin/code.cmd"],
        "cursor" => &["Programs/cursor/resources/app/bin/cursor.cmd"],
        "qoder" => &["Programs/Qoder/resources/app/bin/qoder.cmd"],
        "windsurf" => &["Programs/Windsurf/bin/windsurf.cmd"],
        "zed" => &["Programs/Zed/zed.exe"],
        // winget 清单的 DefaultInstallLocation 是 `%ProgramFiles%\detachhead\Rebased`
        "rebased" => &[
            "detachhead/Rebased/bin/rebased.exe",
            "detachhead/Rebased/bin/rebased64.exe",
            "detachhead/Rebased/bin/rebased.cmd",
            "Programs/Rebased/bin/rebased.exe",
            "Programs/Rebased/bin/rebased64.exe",
            "Programs/Rebased/bin/rebased.cmd",
        ],
        _ => &[],
    }
}

/// 设置里的候选：**全部列出**，装没装都标注出来（换机器后还能沿用同一份习惯）。
#[tauri::command]
pub fn editor_options() -> Vec<EditorOption> {
    let mut out: Vec<EditorOption> = EDITORS
        .iter()
        .map(|p| EditorOption {
            id: p.id.to_string(),
            label: p.label.to_string(),
            installed: resolve_editor(p).is_some(),
        })
        .collect();
    out.push(EditorOption {
        id: "custom".to_string(),
        label: "自定义命令".to_string(),
        installed: true,
    });
    out
}

/// 把自定义命令拆成参数。只处理本项目会生成的写法（路径用双引号包起来），
/// 不做完整的 shell 解析 —— 够用来找「第一个 token 是哪个程序」。
fn shell_tokens(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut has = false;
    for ch in input.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            c if c.is_whitespace() && !quoted => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

/// 命令里的程序还在不在：路径查文件系统，裸命令名查 PATH。
fn program_missing(program: &str) -> bool {
    if program.is_empty() {
        return true;
    }
    let looks_like_path = program.contains('/')
        || program.contains('\\')
        || program.to_ascii_lowercase().ends_with(".app");
    if looks_like_path {
        return !Path::new(program).exists();
    }
    if find_in_path(program).is_some() {
        return false;
    }
    #[cfg(windows)]
    for ext in ["exe", "cmd", "bat"] {
        if find_in_path(&format!("{program}.{ext}")).is_some() {
            return false;
        }
    }
    true
}

/// 自定义命令指向的程序如果已经不在了，返回它。
///
/// `None` = 还在，或无法判断（`open -a "IntelliJ IDEA"` 用的是应用名而不是路径，
/// 交给 `open` 自己解析）。
fn custom_program_missing(template: &str) -> Option<String> {
    let tokens = shell_tokens(template.trim());
    let first = tokens.first()?;
    if first == "open" {
        let app = tokens
            .iter()
            .position(|t| t == "-a")
            .and_then(|i| tokens.get(i + 1))?;
        let looks_like_path = app.contains('/')
            || app.contains('\\')
            || app.to_ascii_lowercase().ends_with(".app");
        if !looks_like_path {
            return None;
        }
        return (!Path::new(app).exists()).then(|| app.clone());
    }
    program_missing(first).then(|| first.clone())
}

/// 设置页用：这些自定义命令指向的程序是否已不存在（卸载后要能标出来）。
#[tauri::command]
pub fn editor_programs_missing(commands: Vec<String>) -> Vec<bool> {
    commands
        .iter()
        .map(|c| custom_program_missing(c).is_some())
        .collect()
}

/// 「选择程序」对话框的起点与可执行文件过滤。
#[derive(Debug, Clone, Serialize)]
pub struct ProgramPickerDefault {
    /// 默认打开的目录；找不到合适的就 `None`（交给对话框自己决定）。
    pub dir: Option<String>,
    /// 只允许选的扩展名；空 = 不过滤。
    pub extensions: Vec<String>,
}

/// 「选择程序」该从哪儿开始找。
///
/// 不给 defaultPath 时三个平台的对话框都会落在「文档」（Windows / Linux 实测
/// 反馈），用户在那儿根本找不到编辑器，所以按平台给最可能的程序目录。
/// GTK / Win32 的文件对话框都支持「起点目录」，macOS 直接给 `.app` 所在的
/// `/Applications`。
#[tauri::command]
pub fn program_picker_default() -> ProgramPickerDefault {
    let (candidates, extensions): (Vec<PathBuf>, Vec<&str>) = if cfg!(windows) {
        let mut dirs = Vec::new();
        for key in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(p) = std::env::var_os(key) {
                dirs.push(PathBuf::from(p));
            }
        }
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(p).join("Programs"));
        }
        (dirs, vec!["exe", "cmd", "bat"])
    } else if cfg!(target_os = "macos") {
        (vec![PathBuf::from("/Applications")], vec![])
    } else {
        (
            vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/snap/bin"),
                PathBuf::from("/opt"),
            ],
            vec![],
        )
    };
    ProgramPickerDefault {
        dir: candidates
            .into_iter()
            .find(|p| p.is_dir())
            .map(|p| p.to_string_lossy().into_owned()),
        extensions: extensions.into_iter().map(str::to_string).collect(),
    }
}

/// 自定义命令里的路径要按 shell 规则包起来；没写 `{path}` 就追加在末尾。
fn custom_editor_script(template: &str, path: &str, windows: bool) -> String {
    let quoted = if windows {
        format!("\"{path}\"")
    } else {
        shell_quote_path(path)
    };
    if template.contains("{path}") {
        template.replace("{path}", &quoted)
    } else {
        format!("{} {}", template.trim(), quoted)
    }
}

/// 启动后最多等 800ms 让它自证失败；还在跑就认为成功（IDE 进程本来就会一直活着）。
///
/// 这一步是为了「失败必须说话」：命令写错时通常几毫秒就退出非零，
/// 只看 `spawn` 成功会把「什么都没打开」当成功。
fn spawn_and_watch(mut cmd: Command, what: &str) -> Result<(), String> {
    let mut child = cmd.spawn().map_err(|e| format!("{what}启动失败: {e}"))?;
    let deadline = Instant::now() + Duration::from_millis(800);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                let code = status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "被信号终止".to_string());
                return Err(format!("{what}退出码 {code}（没有把目录打开）"));
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                // 进程还活着（IDE 本体）：留个线程回收，不让它变僵尸
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(());
            }
            Err(e) => return Err(format!("{what}状态读取失败: {e}")),
        }
    }
}

fn run_custom_editor(template: &str, path: &str) -> Result<(), String> {
    let template = template.trim();
    if let Some(missing) = custom_program_missing(template) {
        return Err(format!(
            "找不到程序「{missing}」（可能已卸载）。请到「设置 → 用编辑器打开」里移除，或重新选择程序"
        ));
    }
    let tokens = shell_tokens(template);
    // 只有程序本身、没有 shell 语法：直接起进程，不经过 shell。
    // Windows 上 `cmd /C "带空格的路径" 参数` 的引号规则很容易把路径拆坏。
    if tokens.len() == 1 {
        let mut cmd = script_command(Path::new(&tokens[0]), cfg!(target_os = "windows"));
        cmd.arg(path);
        return spawn_and_watch(cmd, "自定义编辑器");
    }
    let script = custom_editor_script(template, path, cfg!(target_os = "windows"));
    if cfg!(target_os = "windows") {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(&script);
        hide_console(&mut cmd);
        spawn_and_watch(cmd, "自定义编辑器")
    } else {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(&script);
        spawn_and_watch(cmd, "自定义编辑器")
    }
}

/// (async)：用设置的编辑器打开目录。
///
/// - 默认 IDEA；设置为空时同样回落到 IDEA。
/// - 编辑器已经开着时，CLI 会把路径转发给已有实例并切到对应窗口
///   （JetBrains 的 launcher、VS Code/Cursor/Qoder 的 CLI、Zed 的 CLI 都是这个行为）。
/// - 找不到编辑器/路径不存在都会当场报错，不静默失败。
#[tauri::command(async)]
pub fn open_in_editor(
    path: String,
    editor: Option<String>,
    custom_command: Option<String>,
) -> Result<(), String> {
    ensure_exists(&path)?;
    let id = editor_id(editor);
    if id == "custom" {
        let template = custom_command.unwrap_or_default();
        let template = template.trim();
        if template.is_empty() {
            return Err("还没配置自定义编辑器命令（设置 → 打开方式）".to_string());
        }
        return run_custom_editor(template, &path);
    }
    let Some(p) = preset(&id) else {
        return Err(format!("未知的编辑器：{id}（请在设置里重新选择）"));
    };
    match resolve_editor(p) {
        Some(Launcher::Path(bin)) => {
            // Windows 上 VS Code 系是 `code.cmd` 这类批处理，必须经 cmd 执行。
            let mut cmd = script_command(&bin, cfg!(target_os = "windows"));
            cmd.arg(&path);
            spawn_and_watch(cmd, &format!("打开 {} 失败", p.label))
        }
        Some(Launcher::MacApp(app)) => run_status(
            Command::new("open").arg("-a").arg(&app).arg(&path),
            "打开编辑器失败",
        ),
        None => Err(format!(
            "没找到 {}，请在「设置 → 打开方式」里改成本机已安装的编辑器，或用自定义命令",
            p.label
        )),
    }
}

// ══ 本地文件 ═══════════════════════════════════════════════════════════

/// 文档查看器用途：超大文件整串过 IPC 再渲染会卡死 webview，读前拦下。
const MAX_READ_BYTES: u64 = 2 * 1024 * 1024;

/// 读取 UTF-8 文本文件。文件不存在/编码无效/过大时返回错误，**不会用空串冒充成功**。
#[tauri::command(async)]
pub fn read_text_file(path: String) -> Result<String, String> {
    let len = std::fs::metadata(&path)
        .map_err(|e| format!("读取文件失败 {path}: {e}"))?
        .len();
    if len > MAX_READ_BYTES {
        return Err(format!("文件过大（{len} 字节，上限 2MB）: {path}"));
    }
    // io::Error 不含路径，前缀补上才好定位是哪个文件读失败
    std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败 {path}: {e}"))
}

/// 列目录下的一级子目录名（文件不返回），排序保证稳定显示。
#[tauri::command(async)]
pub fn list_dir(path: String) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(&path).map_err(|e| format!("列出目录失败 {path}: {e}"))?;
    let mut dirs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("列出目录失败 {path}: {e}"))?;
        let is_dir = entry
            .file_type()
            .map_err(|e| format!("列出目录失败 {path}: {e}"))?
            .is_dir();
        if is_dir {
            // 非 UTF-8 文件名无法过 IPC 的 JSON 序列化，跳过而非让整个调用失败
            if let Some(name) = entry.file_name().to_str() {
                dirs.push(name.to_string());
            }
        }
    }
    dirs.sort();
    Ok(dirs)
}

/// 列目录下**所有**条目（文件与目录都返回），供文件浏览（需求 #18）。
#[tauri::command(async)]
pub fn list_entries(path: String) -> Result<Vec<EntryInfo>, String> {
    let entries = std::fs::read_dir(&path).map_err(|e| format!("列出目录失败 {path}: {e}"))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("列出目录失败 {path}: {e}"))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        // .git 内部目录对用户没有意义，翻起来还极慢
        if name == ".git" {
            continue;
        }
        let ft = entry.file_type().map_err(|e| format!("列出目录失败 {path}: {e}"))?;
        out.push(EntryInfo {
            name: name.clone(),
            path: entry.path().to_string_lossy().to_string(),
            is_dir: ft.is_dir(),
        });
    }
    // 目录在前、同类按名字排序
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct EntryInfo {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_path_normalisation() {
        assert_eq!(normalize_win_path(r"C:\a\/b"), r"C:\a\b");
        assert_eq!(normalize_win_path("C:/a/b"), r"C:\a\b");
        // UNC 前缀要保留双反斜杠
        assert_eq!(normalize_win_path(r"\\server\share\a"), r"\\server\share\a");
    }

    #[test]
    fn shell_quote_handles_single_quotes() {
        assert_eq!(shell_quote_path("/tmp/a b"), "'/tmp/a b'");
        assert_eq!(shell_quote_path("it's"), "'it'\\''s'");
    }

    #[test]
    fn applescript_escape_handles_quotes_and_backslashes() {
        assert_eq!(applescript_escape(r#"a"b"#), r#"a\"b"#);
        assert_eq!(applescript_escape(r"a\b"), r"a\\b");
        // 单引号在 AppleScript 里无需转义
        assert_eq!(applescript_escape("it's"), "it's");
    }

    #[test]
    fn iterm_script_uses_write_text_not_command_param() {
        // 用 command 参数会导致会话立即退出、窗口闪退
        let s = macos_terminal_script("iTerm2", "/tmp/x").unwrap();
        assert!(s.contains("write text"), "{s}");
        assert!(!s.contains("command"), "不能用 command 参数: {s}");
    }

    #[test]
    fn warps_has_no_applescript_form() {
        assert!(macos_terminal_script("Warp", "/tmp/x").is_none());
    }

    #[test]
    fn stderr_tail_takes_last_non_empty_line() {
        assert_eq!(stderr_tail_line(b"a\nb\n\n"), Some("b".into()));
        assert_eq!(stderr_tail_line(b""), None);
    }

    /// 不存在的目录必须**当场**报错，而不是丢给 `open` 去静默失败。
    ///
    /// 实测反馈：环境页点「文件管理」没反应 —— `envs/<环境>/<模块>` 还没检出，
    /// `open` 只在 stderr 里抱怨，界面上什么都看不到。
    #[test]
    fn a_missing_directory_is_reported_instead_of_silently_opened() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("envs/dev/not-checked-out");
        let path = missing.to_string_lossy().to_string();

        let err = ensure_exists(&path).unwrap_err();
        assert!(err.contains(&path), "错误里要带上是哪个路径: {err}");
        assert!(err.contains("不存在"), "{err}");

        // 存在时放行
        std::fs::create_dir_all(&missing).unwrap();
        assert!(ensure_exists(&path).is_ok());
    }

    /// `open_path` / `open_in_terminal` 同样要拦 —— 它们也走 ensure_exists。
    #[test]
    fn opening_a_missing_path_fails_without_spawning_anything() {
        let err = open_path("/definitely/not/here-gws-test".to_string()).unwrap_err();
        assert!(err.contains("不存在"), "{err}");
        assert!(open_in_terminal("/definitely/not/here-gws-test".to_string(), None).is_err());
    }

    // ══ 编辑器 ══════════════════════════════════════════════════════

    #[cfg(target_os = "macos")]
    fn preset_by_id(id: &str) -> &'static EditorPreset {
        preset(id).unwrap_or_else(|| panic!("缺少编辑器预设 {id}"))
    }

    /// 主流编辑器都要在清单里；顺序上 IDEA 第一（设置里的默认值）。
    #[test]
    fn editor_catalog_covers_mainstream_editors() {
        for id in [
            "idea", "webstorm", "pycharm", "goland", "phpstorm", "clion", "rubymine", "rider",
            "datagrip", "vscode", "cursor", "qoder", "zed", "windsurf", "rebased",
        ] {
            assert!(preset(id).is_some(), "缺少编辑器 {id}");
        }
        assert_eq!(EDITORS[0].id, DEFAULT_EDITOR);
    }

    /// Windows 已知安装位置必须覆盖 Rebased（winget 默认装 ProgramFiles）。
    #[test]
    fn windows_known_paths_cover_rebased() {
        let paths = windows_known_editor_paths("rebased");
        assert!(
            paths.iter().any(|p| p.ends_with("bin/rebased64.exe")),
            "{paths:?}"
        );
        assert!(paths.iter().any(|p| p.starts_with("detachhead/Rebased/")));
        assert!(windows_known_editor_paths("nope").is_empty());
        // IDEA 走 Toolbox 递归，本身没有固定位置
        assert!(!windows_known_editor_paths("vscode").is_empty());
    }

    /// 没设置 / 设置成空白 => IDEA；设置过就按设置的来。
    #[test]
    fn empty_editor_setting_falls_back_to_idea() {
        assert_eq!(editor_id(None), "idea");
        assert_eq!(editor_id(Some("   ".into())), "idea");
        assert_eq!(editor_id(Some(" vscode ".into())), "vscode");
    }

    /// mac 上必须能从 App 包里找到 launcher —— GUI 进程的 PATH 里没有
    /// `code` / `idea`（实测只有 `/usr/bin:/bin:/usr/sbin:/sbin`）。
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_launchers_are_found_inside_app_bundles() {
        let dir = tempfile::TempDir::new().unwrap();
        let dirs = vec![dir.path().to_path_buf()];
        let idea = dir.path().join("IntelliJ IDEA.app/Contents/MacOS/idea");
        let code = dir
            .path()
            .join("Visual Studio Code.app/Contents/Resources/app/bin/code");
        let zed = dir.path().join("Zed.app/Contents/MacOS/cli");
        let rebased = dir.path().join("Rebased.app/Contents/MacOS/rebased");
        for file in [&idea, &code, &zed, &rebased] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, "").unwrap();
        }

        assert_eq!(
            mac_editor_in(&dirs, preset_by_id("idea")),
            Some(Launcher::Path(idea))
        );
        assert_eq!(
            mac_editor_in(&dirs, preset_by_id("vscode")),
            Some(Launcher::Path(code))
        );
        // Zed 的包内 CLI 叫 cli，不是 zed
        assert_eq!(mac_editor_in(&dirs, preset_by_id("zed")), Some(Launcher::Path(zed)));
        assert_eq!(
            mac_editor_in(&dirs, preset_by_id("rebased")),
            Some(Launcher::Path(rebased))
        );
    }

    /// 包在、包内 launcher 不在时退回 `open -a`，而不是判定"没装"。
    #[cfg(target_os = "macos")]
    #[test]
    fn app_bundle_without_launcher_falls_back_to_open() {
        let dir = tempfile::TempDir::new().unwrap();
        let bundle = dir.path().join("Cursor.app");
        std::fs::create_dir_all(bundle.join("Contents")).unwrap();
        let dirs = vec![dir.path().to_path_buf()];
        assert_eq!(
            mac_editor_in(&dirs, preset_by_id("cursor")),
            Some(Launcher::MacApp(bundle))
        );
    }

    /// Windows 的 Toolbox 目录带版本号，只能递归找；深度上限要生效。
    #[test]
    fn find_file_under_walks_toolbox_dirs_with_a_depth_limit() {
        let dir = tempfile::TempDir::new().unwrap();
        let exe = dir
            .path()
            .join("Toolbox/apps/IDEA-U/ch-0/243.1/bin/idea64.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();

        assert_eq!(find_file_under(dir.path(), "idea64.exe", 8), Some(exe));
        assert_eq!(find_file_under(dir.path(), "idea64.exe", 2), None);
        assert_eq!(find_file_under(dir.path(), "webstorm64.exe", 8), None);
    }

    /// 自定义命令：路径要按 shell 规则引用；没写 `{path}` 就追加。
    #[test]
    fn custom_editor_script_quotes_and_appends_the_path() {
        assert_eq!(
            custom_editor_script("idea {path}", "/tmp/a b", false),
            "idea '/tmp/a b'"
        );
        assert_eq!(
            custom_editor_script("code", "/tmp/a b", false),
            "code '/tmp/a b'"
        );
        assert_eq!(
            custom_editor_script(r#"C:\Tools\ide.exe {path}"#, r"C:\a b", true),
            r#"C:\Tools\ide.exe "C:\a b""#
        );
    }

    /// 程序被卸载 / 路径写错时要能认出来，别等用户点开只看到退出码。
    #[test]
    fn custom_program_missing_detects_removed_programs() {
        let dir = tempfile::TempDir::new().unwrap();
        let exe = dir.path().join("editor");
        std::fs::write(&exe, "").unwrap();

        assert_eq!(custom_program_missing(&format!("\"{}\"", exe.display())), None);
        let gone = dir.path().join("gone");
        assert_eq!(
            custom_program_missing(&format!("\"{}\"", gone.display())),
            Some(gone.display().to_string())
        );
        assert_eq!(
            custom_program_missing(r#"open -a "/definitely/not/here.app""#),
            Some("/definitely/not/here.app".into())
        );
        // 应用名不是路径，交给 open 自己解析，不能误判成"已卸载"
        assert_eq!(custom_program_missing(r#"open -a "IntelliJ IDEA""#), None);
        assert_eq!(custom_program_missing(""), None);
        assert_eq!(shell_tokens(r#"open -a "A B.app""#), ["open", "-a", "A B.app"]);
    }

    #[test]
    fn program_picker_default_points_at_an_existing_dir() {
        let def = program_picker_default();
        if let Some(dir) = &def.dir {
            assert!(Path::new(dir).is_dir(), "起点目录必须真的存在: {dir}");
        }
        if cfg!(windows) {
            assert_eq!(def.extensions, ["exe", "cmd", "bat"]);
            assert!(def.dir.is_some(), "Windows 必须给一个程序目录，不能再落到「文档」");
        } else {
            // macOS 要能选 `.app` 包（本质是目录），按扩展名过滤会让它选不中
            assert!(def.extensions.is_empty(), "非 Windows 不能按扩展名过滤");
        }
    }

    /// 未知编辑器 / 自定义命令为空 / 程序已卸载 / 路径不存在，都要当场说清楚。
    #[test]
    fn editor_errors_are_explicit() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_string_lossy().to_string();

        let err = open_in_editor(path.clone(), Some("nope".into()), None).unwrap_err();
        assert!(err.contains("未知的编辑器"), "{err}");

        let err = open_in_editor(path.clone(), Some("custom".into()), None).unwrap_err();
        assert!(err.contains("自定义编辑器命令"), "{err}");

        let err = open_in_editor(
            path,
            Some("custom".into()),
            Some(r#""/definitely/not/here-gws-editor""#.into()),
        )
        .unwrap_err();
        assert!(err.contains("可能已卸载"), "{err}");

        let err = open_in_editor(
            "/definitely/not/here-gws-test".to_string(),
            Some("idea".into()),
            None,
        )
        .unwrap_err();
        assert!(err.contains("不存在"), "{err}");
    }

    /// 设置里的清单：全部预设 + 自定义，且默认项标成 IDEA。
    #[test]
    fn editor_options_list_every_preset_plus_custom() {
        let opts = editor_options();
        assert_eq!(opts.len(), EDITORS.len() + 1);
        assert_eq!(opts[0].id, DEFAULT_EDITOR);
        assert!(opts.iter().any(|o| o.id == "custom"));
    }

    /// Windows 上 VS Code 系装的是 `code.cmd` 这类批处理，直接 spawn 会失败。
    #[test]
    fn editor_and_paseo_batch_files_go_through_cmd() {
        let code = Path::new(r"C:\Users\u\AppData\Local\Programs\Microsoft VS Code\bin\code.cmd");
        let cmd = script_command(code, true);
        assert_eq!(cmd.get_program().to_string_lossy(), "cmd");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(args[0], "/C");
        assert_eq!(args[1], "call");
        assert_eq!(args[2], code.to_string_lossy());

        // .exe 直接执行；mac/Linux 不套 cmd
        assert_eq!(
            script_command(Path::new(r"C:\Zed\zed.exe"), true)
                .get_program()
                .to_string_lossy(),
            r"C:\Zed\zed.exe"
        );
        assert_eq!(
            script_command(Path::new("/usr/bin/idea"), false)
                .get_program()
                .to_string_lossy(),
            "/usr/bin/idea"
        );
    }
}