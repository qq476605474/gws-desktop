use gws_desk_lib::shell::{
    applescript_escape, detect_terminal, iterm_installed, macos_terminal_script,
    parse_version_from_body, shell_quote_path, TerminalPreference,
};

#[test]
fn force_preference_returns_name_as_is() {
    assert_eq!(detect_terminal(&TerminalPreference::Force("Warp".to_string())), "Warp");
    assert_eq!(detect_terminal(&TerminalPreference::Force("iTerm2".to_string())), "iTerm2");
}

#[cfg(target_os = "macos")]
#[test]
fn follow_system_on_macos_returns_known_terminal() {
    let name = detect_terminal(&TerminalPreference::FollowSystem);
    assert!(
        name == "iTerm2" || name == "Terminal.app",
        "FollowSystem 在 macOS 应返回 iTerm2 或 Terminal.app，实际: {name}"
    );
}

#[test]
fn terminal_preference_partial_eq() {
    assert_ne!(
        TerminalPreference::FollowSystem,
        TerminalPreference::Force(String::new()),
        "FollowSystem 与 Force 必不相等"
    );
    assert_eq!(TerminalPreference::FollowSystem, TerminalPreference::FollowSystem);
    assert_eq!(
        TerminalPreference::Force("Warp".to_string()),
        TerminalPreference::Force("Warp".to_string())
    );
    assert_ne!(
        TerminalPreference::Force("Warp".to_string()),
        TerminalPreference::Force("iTerm2".to_string())
    );
}

#[test]
fn terminal_app_script_structure() {
    let script = macos_terminal_script("Terminal.app", "/Users/foo/proj").unwrap();
    assert!(script.contains("tell application \"Terminal\""), "脚本: {script}");
    assert!(script.contains("activate"));
    assert!(script.contains("do script \"cd '/Users/foo/proj'\""));
    assert!(script.contains("end tell"));
}

#[test]
fn iterm2_script_structure() {
    let script = macos_terminal_script("iTerm2", "/Users/foo/proj").unwrap();
    assert!(script.contains("tell application \"iTerm\""), "脚本: {script}");
    assert!(script.contains("activate"));
    assert!(script.contains("set newWindow to (create window with default profile)"));
    assert!(script.contains("tell current session of newWindow"));
    assert!(script.contains("write text \"cd '/Users/foo/proj'\""), "脚本: {script}");
    assert!(script.contains("end tell"));
}

#[test]
fn warp_returns_none_to_use_open_a() {
    assert!(macos_terminal_script("Warp", "/Users/foo/proj").is_none());
}

#[test]
fn script_wraps_path_with_spaces() {
    let script = macos_terminal_script("Terminal.app", "/Users/foo/My Project").unwrap();
    assert!(script.contains("cd '/Users/foo/My Project'"), "脚本: {script}");
}

#[test]
fn script_escapes_single_quote_in_path() {
    let script = macos_terminal_script("Terminal.app", "/Users/foo/it's here").unwrap();
    // 两层转义：shell 层单引号用 '\''（关闭-转义-重开），
    // AppleScript 层把其中的反斜杠再转成 \\ 才能合法嵌入字符串字面量
    assert!(script.contains("do script \"cd '/Users/foo/it'\\\\''s here'\""), "脚本: {script}");
}

#[test]
fn script_escapes_double_quote_in_path() {
    let script = macos_terminal_script("Terminal.app", "/Users/foo/my\"proj").unwrap();
    // shell 层单引号内双引号原样保留；AppleScript 层转义为 \"
    assert!(script.contains("do script \"cd '/Users/foo/my\\\"proj'\""), "脚本: {script}");
}

#[test]
fn script_escapes_backslash_in_path() {
    let script = macos_terminal_script("Terminal.app", "/Users/foo/a\\b").unwrap();
    // shell 层单引号内反斜杠原样保留；AppleScript 层转义为 \\
    assert!(script.contains("do script \"cd '/Users/foo/a\\\\b'\""), "脚本: {script}");
}

#[test]
fn iterm2_write_text_uses_two_layer_escaping() {
    let script = macos_terminal_script("iTerm2", "/Users/foo/it's \"x\"").unwrap();
    // write text 形态与 do script 走同一套两层转义：
    // '\'' 的 \ → \\，路径中的 " → \"
    assert!(script.contains("write text \"cd '/Users/foo/it'\\\\''s \\\"x\\\"'\""), "脚本: {script}");
}

#[test]
fn shell_quote_path_wraps_and_escapes_single_quotes() {
    assert_eq!(shell_quote_path("/Users/foo/proj"), "'/Users/foo/proj'");
    assert_eq!(shell_quote_path("/Users/foo/My Project"), "'/Users/foo/My Project'");
    // 单引号：关闭引号、\' 转义、重新打开
    assert_eq!(shell_quote_path("it's"), "'it'\\''s'");
}

#[test]
fn shell_quote_path_keeps_double_quote_and_backslash_verbatim() {
    // 单引号内双引号与反斜杠都是字面字符，shell 层无需处理
    assert_eq!(shell_quote_path("my\"proj"), "'my\"proj'");
    assert_eq!(shell_quote_path("a\\b"), "'a\\b'");
}

#[test]
fn applescript_escape_only_touches_backslash_and_double_quote() {
    assert_eq!(applescript_escape("plain path"), "plain path");
    assert_eq!(applescript_escape("a\\b"), "a\\\\b");
    assert_eq!(applescript_escape("say \"hi\""), "say \\\"hi\\\"");
    // AppleScript 字符串内单引号是普通字符，无需转义
    assert_eq!(applescript_escape("it's"), "it's");
}

/// 检测结果必须能直接用于脚本构造：None 会走 open -a 兜底，
/// 而 FollowSystem 在 macOS 只产出 iTerm2 / Terminal.app，两者都有 AppleScript 形态。
#[cfg(target_os = "macos")]
#[test]
fn follow_system_detected_terminal_yields_script() {
    let detected = detect_terminal(&TerminalPreference::FollowSystem);
    assert!(
        macos_terminal_script(&detected, "/Users/foo/proj").is_some(),
        "detect_terminal(FollowSystem) 的产出 {detected} 必须能直接构造 AppleScript"
    );
}

/// 语法回归锚点：osacompile 纯编译、不执行，无窗口副作用。
/// Terminal.app 字典系统必带；iTerm 形态的术语（create window with default profile 等）
/// 依赖 iTerm2 应用字典，未安装的机器上 osacompile 无法解析，跳过该形态——
/// 两形态共用同一套转义管线，Terminal 形态已完整覆盖普通/空格/单引号/双引号/反斜杠。
#[cfg(target_os = "macos")]
#[test]
fn generated_scripts_compile_with_osacompile() {
    use std::fs;
    use std::process::Command;

    let paths = [
        "/Users/foo/proj",
        "/Users/foo/My Project",
        "/Users/foo/it's here",
        "/Users/foo/my\"proj",
        "/Users/foo/a\\b",
    ];
    let terminals: &[&str] = if iterm_installed() {
        &["Terminal.app", "iTerm2"]
    } else {
        &["Terminal.app"]
    };
    for terminal in terminals {
        for path in paths {
            let script = macos_terminal_script(terminal, path)
                .unwrap_or_else(|| panic!("{terminal} 应生成 AppleScript 脚本"));
            let out = std::env::temp_dir()
                .join(format!("gws_desk_osacompile_{}.scpt", std::process::id()));
            let result = Command::new("osacompile")
                .arg("-o")
                .arg(&out)
                .arg("-e")
                .arg(&script)
                .output()
                .expect("启动 osacompile 失败");
            let _ = fs::remove_file(&out);
            assert!(
                result.status.success(),
                "{terminal} + {path} 的脚本未通过 osacompile 编译：\n{script}\nstderr: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}

#[test]
fn parse_version_quoted_value() {
    let body = "#!/usr/bin/env bash\nGWS_VERSION=\"0.4.2\"\n";
    assert_eq!(parse_version_from_body(body), Some("0.4.2".to_string()));
}

#[test]
fn parse_version_unquoted_value() {
    assert_eq!(parse_version_from_body("GWS_VERSION=0.4.2\n"), Some("0.4.2".to_string()));
}

#[test]
fn parse_version_leading_spaces() {
    assert_eq!(
        parse_version_from_body("  GWS_VERSION=\"0.4.2\"\n"),
        Some("0.4.2".to_string())
    );
}

#[test]
fn parse_version_not_on_first_line() {
    let body = "#!/usr/bin/env bash\n# some comment\nGWS_VERSION=\"0.4.1\"\necho hello";
    assert_eq!(parse_version_from_body(body), Some("0.4.1".to_string()));
}

#[test]
fn parse_version_missing_returns_none() {
    assert_eq!(parse_version_from_body("#!/usr/bin/env bash\necho hello\n"), None);
    assert_eq!(parse_version_from_body(""), None);
}

#[test]
fn parse_version_empty_value_returns_none() {
    // 对齐 gws 自身更新器的 [ -n "$rv" ] 守卫：空值视为无效
    assert_eq!(parse_version_from_body("GWS_VERSION=\n"), None);
    assert_eq!(parse_version_from_body("GWS_VERSION=\"\"\n"), None);
    assert_eq!(parse_version_from_body("GWS_VERSION=''\n"), None);
}
