use gws_desk_lib::shell::{
    detect_terminal, macos_terminal_script, parse_version_from_body, TerminalPreference,
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
    assert!(script.contains("create window with default profile command \"cd '/Users/foo/proj'\""));
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
    // 单引号内的单引号无法直接出现：关闭引号、\' 转义、重新打开（'\''）
    assert!(script.contains("cd '/Users/foo/it'\\''s here'"), "脚本: {script}");
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
