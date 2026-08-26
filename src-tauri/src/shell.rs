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

fn iterm_installed() -> bool {
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

/// 等待命令退出：spawn 失败与非零退出码都转为带前缀的中文错误。
fn run_status(cmd: &mut Command, err_prefix: &str) -> Result<(), String> {
    let status = cmd.status().map_err(|e| format!("{err_prefix}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(match status.code() {
            Some(code) => format!("{err_prefix}: 退出码 {code}"),
            None => format!("{err_prefix}: 进程被信号终止"),
        })
    }
}

#[tauri::command]
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

/// 构造 macOS AppleScript；返回 None 表示该终端（如 Warp）不支持 AppleScript，
/// 由调用方走 open -a 分支。
pub fn macos_terminal_script(terminal: &str, path: &str) -> Option<String> {
    if terminal == "Warp" {
        return None;
    }
    // 路径中的单引号在单引号包裹内无法直接出现：关闭引号、\' 转义、重新打开
    let escaped = path.replace('\'', "'\\''");
    let cmd = format!("cd '{escaped}'");
    if terminal == "iTerm2" {
        Some(format!(
            "tell application \"iTerm\"\nactivate\ncreate window with default profile command \"{cmd}\"\nend tell"
        ))
    } else {
        Some(format!(
            "tell application \"Terminal\"\nactivate\ndo script \"{cmd}\"\nend tell"
        ))
    }
}

#[tauri::command]
pub fn open_in_terminal(path: String, terminal: String) -> Result<(), String> {
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
            "gnome-terminal" | "konsole" | "xfce4-terminal" => run_status(
                Command::new(terminal.as_str()).arg("--working-directory").arg(&path),
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

/// (async)：curl 最长阻塞 60s，须在主线程外执行（sync fn + async 标记 → 线程池）。
#[tauri::command(async)]
pub fn latest_gws_version() -> Result<String, String> {
    let url = std::env::var("GWS_UPDATE_URL").unwrap_or_else(|_| DEFAULT_GWS_UPDATE_URL.to_string());
    let output = Command::new("curl")
        .args(["-fsSL", "--connect-timeout", "10", "--max-time", "60"])
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
    Some(value.to_string())
}
