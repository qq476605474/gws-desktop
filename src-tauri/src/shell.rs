use std::path::{Path, PathBuf};
use std::process::Command;

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

/// (async)：open 触发自动化权限弹窗时可阻塞至用户响应（最长约 2 分钟），
/// 须在主线程外执行（sync fn + async 标记 → 线程池），否则整个 GUI 冻结。
#[tauri::command(async)]
pub fn open_in_finder(path: String) -> Result<(), String> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "linux") {
        "xdg-open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        return Err("不支持的平台".to_string());
    };
    run_status(Command::new(program).arg(&path), "打开目录失败")
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

/// (async)：curl 最长阻塞 60s，须在主线程外执行（sync fn + async 标记 → 线程池）。
#[tauri::command(async)]
pub fn latest_gws_version() -> Result<String, String> {
    let url = std::env::var("GWS_UPDATE_URL").unwrap_or_else(|_| DEFAULT_GWS_UPDATE_URL.to_string());
    let output = Command::new("curl")
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
