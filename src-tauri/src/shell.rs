use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

const DEFAULT_GWS_UPDATE_URL: &str = "https://raw.githubusercontent.com/qq476605474/gws/main/gws";

#[derive(Debug, Clone, PartialEq)]
pub enum TerminalPreference {
    FollowSystem,
    Force(String),
}

/// PATH 查找可执行文件，风格与 gws_runner::find_gws 一致。
fn find_in_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path).map(|dir| dir.join(name)).find(|candidate| candidate.is_file())
    })
}

/// iTerm2 是否安装（系统或用户 Applications）。
/// 除终端检测外，osacompile 编译 iTerm 脚本需要其应用字典，
/// 测试据此判断 iTerm 形态能否做语法校验。
pub fn iterm_installed() -> bool {
    if Path::new("/Applications/iTerm.app").exists() {
        return true;
    }
    std::env::var_os("HOME")
        .is_some_and(|home| Path::new(&home).join("Applications/iTerm.app").exists())
}

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

/// 等待命令退出并捕获输出：spawn 失败与非零退出码都转为带前缀的中文错误。
/// 失败时附 stderr 尾部一行——osascript 的语法错误、应用未找到等真实原因
/// 只会出现在子进程 stderr，不捕获就只剩一个无从排查的退出码。
/// open/xdg-open 正常路径 stderr 为空，不受影响。
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

/// stderr 的最后一个非空行（去首尾空白）；无效 UTF-8 按 lossy 解码。
fn stderr_tail_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

/// Windows 路径规范化：/ → \，连续分隔符压成一个（保留 UNC 开头 \\）。
/// 前端拼路径用 / 连接，用户输入也可能是混合分隔符（如 C:\a\/b），
/// 直接 replace 会留下 \\ 双反斜杠——explorer 解析失败即回退打开"文档"。
pub fn normalize_win_path(path: &str) -> String {
    let p = path.replace('/', "\\");
    if let Some(rest) = p.strip_prefix("\\\\") {
        format!("\\\\{}", rest.replace("\\\\", "\\"))
    } else {
        p.replace("\\\\", "\\")
    }
}

/// (async)：open 触发自动化权限弹窗时可阻塞至用户响应（最长约 2 分钟），
/// 须在主线程外执行（sync fn + async 标记 → 线程池），否则整个 GUI 冻结。
#[tauri::command(async)]
pub fn open_in_finder(path: String) -> Result<(), String> {
    if cfg!(target_os = "windows") {
        // explorer 只认反斜杠路径：正斜杠/双反斜杠都会解析失败，
        // 回退打开"文档"（用户报告的现象）。且 explorer 无论成败恒以
        // 退出码 1 返回，不能按退出码判错——spawn 成功即视为已打开。
        let win_path = normalize_win_path(&path);
        Command::new("explorer")
            .arg(&win_path)
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

/// 用系统默认应用打开任意路径：目录→文件管理器，文件→关联应用（区别于
/// open_in_finder 的 explorer：Windows 下后者对文件只会在资源管理器中选中）。
#[tauri::command(async)]
pub fn open_path(path: String) -> Result<(), String> {
    if cfg!(target_os = "windows") {
        // ShellExecuteW（tauri-plugin-opener）比 cmd /C start 稳定：正斜杠、
        // 中文路径、文件/目录分发都交给系统 API，且无闪黑窗问题
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

/// 写系统剪贴板。走 Rust 侧而非 JS 插件 API：WKWebView 的 JS 剪贴板层
/// 在部分环境静默失效（点击无反应且无错误可见），Rust 直写 pasteboard 稳定。
#[tauri::command(async)]
pub fn copy_text(text: String, app: AppHandle) -> Result<(), String> {
    app.clipboard()
        .write_text(text)
        .map_err(|e| format!("复制失败: {e}"))
}

/// 设置面板的终端候选项。id 传回 open_in_terminal 的 terminal 参数（"system" 表示跟随系统）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TerminalOption {
    pub id: String,
    pub label: String,
}

/// macOS .app 是否安装（系统或用户 Applications）。
fn app_installed(name: &str) -> bool {
    if Path::new("/Applications").join(format!("{name}.app")).exists() {
        return true;
    }
    std::env::var_os("HOME").is_some_and(|home| {
        Path::new(&home).join("Applications").join(format!("{name}.app")).exists()
    })
}

/// 设置面板的终端候选：按当前 OS 给出该 OS 合理的列表（只列实际安装的），
/// "system" 恒在首位并标注实际会用的终端——硬编码 iTerm2 等 mac 专属项在
/// Linux/Windows 上既不可选也不诚实。
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

/// 第一层（shell）：路径包成单引号 token，形如 'it'\''s'。
/// 单引号内的单引号无法直接出现——关闭引号、\' 转义、重新打开；
/// 双引号与反斜杠在单引号内是字面字符，无需处理。
pub fn shell_quote_path(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// 第二层（AppleScript）：嵌入 AppleScript 双引号字符串前的转义。
/// AppleScript 只认 \" 与 \\，`\'` 是语法错误（osacompile 报 -2741）；单引号无需转义。
pub fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// 构造 macOS AppleScript；返回 None 表示该终端（如 Warp）不支持 AppleScript，
/// 由调用方走 open -a 分支。
/// 转义两层，顺序严格：先 shell 层构造 cd 命令（shell_quote_path），
/// 再 AppleScript 层把整条命令嵌入字符串字面量（applescript_escape）。
pub fn macos_terminal_script(terminal: &str, path: &str) -> Option<String> {
    if terminal == "Warp" {
        return None;
    }
    let cmd = format!("cd {}", shell_quote_path(path));
    if terminal == "iTerm2" {
        // 不能用 `create window with default profile command "<cmd>"`：command 参数会
        // 替换该会话的 shell 命令，cd 毫秒级退出 → 会话立即结束、窗口闪退且无报错。
        // 官方模式是先建普通交互会话，再 write text 把命令写进 tty 输入缓冲（不丢）。
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

/// (async)：osascript/open 触发自动化权限弹窗时可阻塞至用户响应（最长约 2 分钟），
/// 须在主线程外执行（sync fn + async 标记 → 线程池），否则整个 GUI 冻结。
/// terminal 为 None 时跟随系统自动检测（前端传 null 表示跟随系统）。
#[tauri::command(async)]
pub fn open_in_terminal(path: String, terminal: Option<String>) -> Result<(), String> {
    let terminal = terminal.unwrap_or_else(|| detect_terminal(&TerminalPreference::FollowSystem));
    if cfg!(target_os = "macos") {
        match macos_terminal_script(&terminal, &path) {
            Some(script) => {
                run_status(Command::new("osascript").arg("-e").arg(&script), "打开终端失败")
            }
            None => run_status(
                Command::new("open").arg("-a").arg("Warp").arg(&path),
                "打开终端失败",
            ),
        }
    } else if cfg!(target_os = "linux") {
        match terminal.as_str() {
            "gnome-terminal" | "xfce4-terminal" => run_status(
                Command::new(terminal.as_str()).arg("--working-directory").arg(&path),
                "打开终端失败",
            ),
            // konsole 的工作目录参数是 --workdir（man page），不是 --working-directory
            "konsole" => run_status(
                Command::new(terminal.as_str()).arg("--workdir").arg(&path),
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

#[tauri::command]
pub fn check_gws_installed() -> bool {
    crate::gws_runner::find_gws().is_some()
}

#[tauri::command]
pub fn hub_exists(path: String) -> bool {
    Path::new(&path).join(".gws-hub").is_file()
}

/// (async)：本地小文件读取通常毫秒级，但网络盘/超大文件仍可能阻塞——
/// 与其他 IO 命令一致放线程池执行，防主线程卡顿。
#[tauri::command(async)]
pub fn read_text_file(path: String) -> Result<String, String> {
    // 文档查看器用途：超大文件整串过 IPC 再渲染单个 pre 会卡死 webview，读前拦下
    const MAX_BYTES: u64 = 2 * 1024 * 1024;
    let len = std::fs::metadata(&path)
        .map_err(|e| format!("读取文件失败 {path}: {e}"))?
        .len();
    if len > MAX_BYTES {
        return Err(format!("文件过大（{len} 字节，上限 2MB）: {path}"));
    }
    // io::Error 不含路径，前缀补上才好定位是哪个文件读失败
    std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败 {path}: {e}"))
}

/// (async)：网络盘上的 read_dir 可能阻塞，与其他 IO 命令一致放线程池执行。
/// 列目录下的一级子目录名（文件不返回），排序保证稳定显示。
#[tauri::command(async)]
pub fn list_dir(path: String) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(&path).map_err(|e| format!("列出目录失败 {path}: {e}"))?;
    let mut dirs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("列出目录失败 {path}: {e}"))?;
        // file_type 基于目录项本身（不追随符号链接）：非目录一律不返回
        let is_dir = entry.file_type().map_err(|e| format!("列出目录失败 {path}: {e}"))?.is_dir();
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

/// (async)：curl 最长阻塞 60s，须在主线程外执行（sync fn + async 标记 → 线程池）。
#[tauri::command(async)]
pub fn latest_gws_version() -> Result<String, String> {
    let url = std::env::var("GWS_UPDATE_URL").unwrap_or_else(|_| DEFAULT_GWS_UPDATE_URL.to_string());
    let mut cmd = Command::new("curl");
    #[cfg(windows)]
    {
        // curl 是控制台程序：GUI 进程拉它会闪黑窗，用 CREATE_NO_WINDOW 抑制
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let output = cmd
        .args(["-fsSL", "--connect-timeout", "10", "--max-time", "60", "--"])
        .arg(&url)
        .output()
        .map_err(|e| format!("获取最新版本失败: {e}"))?;
    if !output.status.success() {
        return Err(match output.status.code() {
            Some(code) => format!("获取最新版本失败: curl 退出码 {code}"),
            None => "获取最新版本失败: curl 被信号终止".to_string(),
        });
    }
    let body = String::from_utf8_lossy(&output.stdout);
    parse_version_from_body(&body).ok_or_else(|| "下载内容不是 gws 脚本".to_string())
}

pub fn parse_version_from_body(body: &str) -> Option<String> {
    let line = body.lines().find(|l| l.trim_start().starts_with("GWS_VERSION="))?;
    let rest = line.trim_start().strip_prefix("GWS_VERSION=")?;
    let value = rest.trim().trim_matches('"').trim_matches('\'').trim();
    // 空值视为无效：对齐 gws 自身更新器的 [ -n "$rv" ] 守卫
    (!value.is_empty()).then(|| value.to_string())
}
