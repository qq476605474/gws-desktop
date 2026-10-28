//! 本机「已经装好的程序」清单。
//!
//! 设置里选编辑器时直接从这里挑：macOS 列 `/Applications` 里的 `.app`，
//! Windows 列开始菜单（用户级 + 全局）的快捷方式并解析出真实程序，Linux 读
//! `.desktop` 文件。用户不用再钻进 `Program Files` 或 `/usr/bin` 翻目录 ——
//! 那是实测反馈里最难受的一步。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstalledApp {
    /// 给人看的名字（`Visual Studio Code`、`IntelliJ IDEA`）。
    pub name: String,
    /// 存进编辑器配置的命令模板（后端会把目录追加在末尾）。
    pub command: String,
    /// 真实程序路径 / 快捷方式指向的目标，只用于去重。
    #[serde(skip)]
    pub target: String,
}

/// 本机已安装的程序，按名字排序、去掉重名。
///
/// Windows 上要起一次 PowerShell 解析开始菜单的快捷方式（秒级），放到后台线程
/// 跑，别卡住界面。
#[tauri::command]
pub async fn installed_apps() -> Vec<InstalledApp> {
    tauri::async_runtime::spawn_blocking(list_apps)
        .await
        .unwrap_or_default()
}

fn list_apps() -> Vec<InstalledApp> {
    let mut apps = scan();
    apps.sort_by_cached_key(|a| a.name.to_lowercase());
    let mut seen = HashSet::new();
    apps.retain(|a| seen.insert(a.name.to_lowercase()));
    apps
}

fn scan() -> Vec<InstalledApp> {
    #[cfg(target_os = "macos")]
    {
        let mut apps = mac_apps();
        apps.extend(from_app_dirs(&home_dir(), ".app"));
        dedupe_by_target(apps)
    }
    #[cfg(windows)]
    {
        windows_start_menu_apps()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux_desktop_apps()
    }
    #[cfg(not(any(windows, unix)))]
    {
        Vec::new()
    }
}

#[allow(dead_code)] // Windows 上用开始菜单，不读 HOME
fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn dedupe_by_target(apps: Vec<InstalledApp>) -> Vec<InstalledApp> {
    let mut seen = HashSet::new();
    apps.into_iter()
        .filter(|a| seen.insert(a.target.to_lowercase()))
        .collect()
}

// ── macOS ─────────────────────────────────────────────────────────────

/// `/Applications` 下的应用（`.app` 本质是目录，`open -a` 才能启动）。
#[cfg(target_os = "macos")]
fn mac_apps() -> Vec<InstalledApp> {
    from_app_dirs(&PathBuf::from("/Applications"), ".app")
}

#[cfg(target_os = "macos")]
fn from_app_dirs(dir: &Path, suffix: &str) -> Vec<InstalledApp> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if !file_name.to_lowercase().ends_with(suffix) || file_name.starts_with('.') {
            continue;
        }
        let name = file_name[..file_name.len() - suffix.len()].to_string();
        let target = path.to_string_lossy().into_owned();
        out.push(InstalledApp {
            name,
            command: format!("open -a \"{target}\""),
            target,
        });
    }
    out
}

// ── Windows ───────────────────────────────────────────────────────────

/// 开始菜单里的快捷方式 = 用户心里「装了哪些程序」的样子。
///
/// `.lnk` 不能直接执行，用 PowerShell 的 `WScript.Shell` 一次性解析出所有目标。
#[cfg(windows)]
fn windows_start_menu_apps() -> Vec<InstalledApp> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("ProgramData") {
        roots.push(PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Some(p) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    let roots: Vec<String> = roots
        .iter()
        .filter(|p| p.is_dir())
        .map(|p| format!("'{}'", p.display().to_string().replace('\'', "''")))
        .collect();
    if roots.is_empty() {
        return Vec::new();
    }
    let script = format!(
        "[Console]::OutputEncoding=[Text.Encoding]::UTF8; \
         $sh = New-Object -ComObject WScript.Shell; \
         Get-ChildItem -Path @({}) -Filter *.lnk -Recurse -ErrorAction SilentlyContinue | \
         ForEach-Object {{ $t = $sh.CreateShortcut($_.FullName).TargetPath; \
         if ($t) {{ $_.BaseName + \"`t\" + $t }} }}",
        roots.join(",")
    );
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    crate::platform::hide_console(&mut cmd);
    let Ok(out) = cmd.output() else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut apps = Vec::new();
    for line in text.lines() {
        let Some((name, target)) = line.split_once('\t') else {
            continue;
        };
        let name = name.trim();
        let target = target.trim();
        if name.is_empty() || !looks_like_program(target) || !Path::new(target).is_file() {
            continue;
        }
        if is_background_entry(name) {
            continue;
        }
        apps.push(InstalledApp {
            name: name.to_string(),
            command: format!("\"{target}\""),
            target: target.to_string(),
        });
    }
    dedupe_by_target(apps)
}

/// 只认能直接起进程的程序；文档、网址、文件夹都排除。
#[cfg(windows)]
fn looks_like_program(target: &str) -> bool {
    let lower = target.to_lowercase();
    [".exe", ".cmd", ".bat"].iter().any(|ext| lower.ends_with(ext))
}

/// 「卸载 IntelliJ IDEA」「Readme」「官网」这类快捷方式不是程序。
#[cfg(windows)]
fn is_background_entry(name: &str) -> bool {
    let lower = name.to_lowercase();
    [
        "uninstall",
        "卸载",
        "readme",
        "help",
        "帮助",
        "documentation",
        "文档",
        "website",
        "官网",
        "网站",
        "release notes",
    ]
    .iter()
    .any(|key| lower.contains(key))
}

// ── Linux ─────────────────────────────────────────────────────────────

/// `.desktop` 文件就是 Linux 的「应用列表」，名字比文件名友好得多。
#[cfg(all(unix, not(target_os = "macos")))]
fn linux_desktop_apps() -> Vec<InstalledApp> {
    let mut dirs = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
        PathBuf::from("/var/lib/snapd/desktop/applications"),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
    ];
    let home = home_dir();
    if !home.as_os_str().is_empty() {
        dirs.push(home.join(".local/share/applications"));
        dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
    }
    let mut apps = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some((name, command)) = desktop_entry(&text) else {
                continue;
            };
            apps.push(InstalledApp {
                name,
                command,
                target: path.to_string_lossy().into_owned(),
            });
        }
    }
    dedupe_by_target(apps)
}

/// 从 `.desktop` 里取出「显示名 + 可执行命令」。
///
/// 只读 `[Desktop Entry]` 段；`NoDisplay` / `Hidden` 的是后台项（不出现在应用
/// 菜单里），`Terminal=true` 的是命令行工具，都不适合「用编辑器打开目录」。
#[cfg(all(unix, not(target_os = "macos")))]
fn desktop_entry(text: &str) -> Option<(String, String)> {
    let mut name = None;
    let mut name_zh = None;
    let mut exec = None;
    let mut hidden = false;
    let mut terminal = false;
    let mut kind = None;
    let mut in_main = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key {
            "Name" => name = Some(value.to_string()),
            "Name[zh_CN]" => name_zh = Some(value.to_string()),
            "Exec" => exec = Some(value.to_string()),
            "NoDisplay" | "Hidden" => hidden |= value.eq_ignore_ascii_case("true"),
            "Terminal" => terminal |= value.eq_ignore_ascii_case("true"),
            "Type" => kind = Some(value.to_string()),
            _ => {}
        }
    }
    if hidden || terminal {
        return None;
    }
    if let Some(kind) = kind {
        if kind != "Application" {
            return None;
        }
    }
    let name = name_zh.or(name)?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let command = desktop_command(&exec?)?;
    Some((name, command))
}

/// `Exec=code --new-window %U` → `"/usr/bin/code" --new-window`
///
/// 字段占位（`%U` / `%f` / `%i` …）要去掉；`env VAR=… 程序` 的前缀要剥掉。
/// 程序本身找不到（卸载了）就整条丢掉，免得列表里全是点了没反应的项。
#[cfg(all(unix, not(target_os = "macos")))]
fn desktop_command(exec: &str) -> Option<String> {
    let tokens: Vec<String> = split_command(exec)
        .into_iter()
        .filter(|t| !t.starts_with('%'))
        .collect();
    let mut tokens = tokens.into_iter().skip_while(|t| t == "env" || is_env_assignment(t));
    let program = tokens.next()?;
    if !program_exists(&program) {
        return None;
    }
    let all = std::iter::once(program).chain(tokens);
    Some(all.map(|t| quote_token(&t)).collect::<Vec<_>>().join(" "))
}

/// 带空格的路径 / 参数要重新加引号，否则拼出来的命令会被拆开。
#[cfg(all(unix, not(target_os = "macos")))]
fn quote_token(token: &str) -> String {
    if token.chars().any(|c| c.is_whitespace()) {
        format!("\"{token}\"")
    } else {
        token.to_string()
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn is_env_assignment(token: &str) -> bool {
    match token.split_once('=') {
        Some((key, _)) => {
            !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        }
        None => false,
    }
}

/// 绝对路径看文件在不在；裸命令名在 `PATH` 里找。
#[cfg(all(unix, not(target_os = "macos")))]
fn program_exists(program: &str) -> bool {
    // flatpak / snap / env 的启动器会把目录当成参数交给别的程序，拼出来的命令
    // 不可用。只看 basename：`/usr/bin/env` 与裸 `env` 是同一个问题。
    let base = program.rsplit('/').next().unwrap_or(program);
    if ["flatpak", "snap", "sh", "bash", "env"].contains(&base) {
        return false;
    }
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let candidate = dir.join(program);
                candidate.is_file()
            })
        })
        .unwrap_or(false)
}

/// 按双引号切分命令（`.desktop` 里带空格的路径会写成 `"…"`）。
#[cfg(all(unix, not(target_os = "macos")))]
fn split_command(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut has_content = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                has_content = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if has_content {
                    out.push(std::mem::take(&mut current));
                    has_content = false;
                }
            }
            c => {
                current.push(c);
                has_content = true;
            }
        }
    }
    if has_content {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    // Windows 上下面这些测试都被 cfg 排除了，通配导入会变成 unused
    #[allow(unused_imports)]
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_app_names_drop_the_bundle_suffix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("IntelliJ IDEA.app")).unwrap();
        std::fs::create_dir(dir.path().join(".hidden.app")).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        let apps = from_app_dirs(dir.path(), ".app");
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "IntelliJ IDEA");
        assert!(apps[0].command.starts_with("open -a \""));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn desktop_entry_reads_chinese_name_and_strips_field_codes() {
        let text = "\
[Desktop Entry]
Type=Application
Name=Visual Studio Code
Name[zh_CN]=代码编辑器
Exec=/bin/ls --new-window %U
";
        let (name, command) = desktop_entry(text).unwrap();
        assert_eq!(name, "代码编辑器");
        assert_eq!(command, "/bin/ls --new-window");

        let chinese = text.replace("/bin/ls", "/usr/bin/env");
        assert!(desktop_entry(&chinese).is_none(), "env 启动器不是真程序");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn desktop_entry_skips_hidden_and_terminal_apps() {
        let base = "[Desktop Entry]\nType=Application\nName=Tool\nExec=/bin/ls\n";
        assert!(desktop_entry(base).is_some());
        assert!(desktop_entry(&format!("{base}NoDisplay=true\n")).is_none());
        assert!(desktop_entry(&format!("{base}Hidden=true\n")).is_none());
        assert!(desktop_entry(&format!("{base}Terminal=true\n")).is_none());
        assert!(desktop_entry(&base.replace("Application", "Link")).is_none());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn desktop_command_drops_env_prefix_and_missing_programs() {
        assert_eq!(
            desktop_command("env FOO=1 /bin/ls -la").as_deref(),
            Some("/bin/ls -la")
        );
        assert!(desktop_command("/definitely/not/here/app %U").is_none());
        assert!(desktop_command("flatpak run org.example.App").is_none());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn split_command_keeps_quoted_paths_together() {
        assert_eq!(
            split_command(r#""/opt/My App/editor" --flag"#),
            vec!["/opt/My App/editor", "--flag"]
        );
        // 拆完再拼回去要补回引号，否则带空格的路径会被拆坏
        assert_eq!(
            quote_token("/opt/My App/editor"),
            "\"/opt/My App/editor\""
        );
        assert_eq!(quote_token("--flag"), "--flag");
    }
}