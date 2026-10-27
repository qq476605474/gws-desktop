//! 桌面端 / 命令行 / Skills 三个版本的状态与同步。
//!
//! 实测反馈（#7）：桌面端能自更新，但 CLI 和 skills 各自装在各处，
//! 装完桌面端还是旧的 CLI / 旧的技能。这一版起桌面端**自带**一份同版本的
//! CLI 与内置 skill（`scripts/bundle-cli.mjs` 打进 resources，技能直接
//! `include_str!`）：
//!
//! - 安装 / 启动时发现版本落后就自动覆盖（用户明确要求「一起自动覆盖」）
//! - 覆盖前把原文件备份成 `*.gws-backup`，不静默丢别人的改动
//! - 本机版本比桌面端**新**时保留，不降级
//! - 设置页仍可手动「立即同步」
//!
//! `tool_versions` 只读；写操作都在 `sync_bundled`。

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use tauri::Manager;
use gws_core::paths;

/// 随桌面端打包的 skill。改仓库里的文件即可，编译时读进来。
const BUNDLED_SKILLS: [(&str, &str); 1] = [
    ("gws", include_str!("../../../skills/gws/SKILL.md")),
];

/// 一个已检测到的 AI CLI 的 skills 落点。
#[derive(Debug, Clone, Serialize)]
pub struct SkillTargetStatus {
    pub label: String,
    pub dir: String,
    /// 同步时写进去的版本标记；没装 / 旧版没标记是 `None`。
    pub version: Option<String>,
    pub installed: bool,
}

/// 三个东西的当前状态。
#[derive(Debug, Clone, Serialize)]
pub struct ToolVersions {
    /// 桌面端版本（编译期就定了）。
    pub desktop: String,
    /// 命令行版本；没装或跑不起来是 `None`。
    pub cli: Option<String>,
    pub cli_path: Option<String>,
    /// 本机已安装的 AI CLI 各自的 skills 目录（只列已存在的）。
    pub skill_targets: Vec<SkillTargetStatus>,
}

/// 一次同步的结果。两条都是**人话**，设置页直接显示。
#[derive(Debug, Clone, Serialize)]
pub struct ToolSyncOutcome {
    pub changed: bool,
    pub cli: String,
    pub skills: String,
}

/// `gws --version` 的输出 → 版本号。
///
/// clap 输出形如 `gws 1.0.5`；只取最后一段，兼容前面带路径 / 其它字样的版本。
fn parse_version(stdout: &str) -> Option<String> {
    let s = stdout.trim();
    if s.is_empty() {
        return None;
    }
    s.split_whitespace().last().map(str::to_string)
}

/// 版本号里的数字段。解析不了就当没有版本（调用方会覆盖）。
fn semver_parts(v: &str) -> Option<Vec<u64>> {
    let core = v.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let parts: Vec<u64> = core
        .split('.')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    (!parts.is_empty()).then_some(parts)
}

/// `a` 是否比 `b` 新。只比数字段，长度不足按 0 补。
fn is_newer(a: &str, b: &str) -> bool {
    let (Some(a), Some(b)) = (semver_parts(a), semver_parts(b)) else {
        return false;
    };
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "gws.exe"
    } else {
        "gws"
    }
}

/// 常见安装位置。**不能只靠 PATH**：从 Finder 启动的 GUI 进程拿不到
/// shell 配置里的 `~/.local/bin`（实测反馈）。
fn cli_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = paths::home_dir() {
        out.push(home.join(".local/bin").join(exe_name()));
        out.push(home.join(".cargo/bin").join(exe_name()));
    }
    out.push(PathBuf::from("/usr/local/bin").join(exe_name()));
    out.push(PathBuf::from("/opt/homebrew/bin").join(exe_name()));
    out
}

fn detect_cli() -> (Option<String>, Option<String>) {
    let path = cli_candidates().into_iter().find(|p| p.is_file());
    let Some(path) = path else {
        return (None, None);
    };
    // GUI 进程起控制台程序会弹黑窗口：启动时这里每跑一次就闪一下（实测反馈）。
    let mut cmd = Command::new(&path);
    crate::platform::hide_console(&mut cmd);
    let version = cmd
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| parse_version(&String::from_utf8_lossy(&o.stdout)));
    (version, Some(path.to_string_lossy().to_string()))
}

/// 同步落点：优先覆盖**已经在用的那个**，一个都没有就装到 `~/.local/bin`。
fn cli_dest() -> Option<PathBuf> {
    cli_candidates()
        .into_iter()
        .find(|p| p.is_file())
        .or_else(|| paths::home_dir().map(|h| h.join(".local/bin").join(exe_name())))
}

/// 各 AI CLI 读 skills 的目录。**只写已经存在的**：用户没装某个 CLI 时
/// 不凭空造一整套目录；手动指定 `GWS_SKILLS_DIR` 时才无条件写那一个。
const SKILL_TARGETS: [(&str, &str); 4] = [
    ("Claude Code", ".claude/skills"),
    ("OpenCode", ".config/opencode/skills"),
    ("Qoder CLI", ".qoder/skills"),
    ("Codex", ".codex/skills"),
];

fn skills_targets_in(home: &Path) -> Vec<(String, PathBuf)> {
    SKILL_TARGETS
        .iter()
        .map(|(label, rel)| ((*label).to_string(), home.join(rel)))
        .filter(|(_, dir)| dir.is_dir())
        .collect()
}

fn skills_targets() -> Vec<(String, PathBuf)> {
    if let Some(dir) = std::env::var_os("GWS_SKILLS_DIR").filter(|s| !s.is_empty()) {
        return vec![(
            "自定义（GWS_SKILLS_DIR）".to_string(),
            PathBuf::from(dir),
        )];
    }
    paths::home_dir()
        .map(|h| skills_targets_in(&h))
        .unwrap_or_default()
}

fn skills_version(dir: &Path) -> Option<String> {
    let marker = dir.join("gws").join(".gws-version");
    std::fs::read_to_string(marker)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 读三个版本 —— 纯本地，不联网。
#[tauri::command]
pub fn tool_versions() -> ToolVersions {
    let (cli, cli_path) = detect_cli();
    // 目录存在不等于技能装过（目录里可能只有别人的技能）
    let skill_targets = skills_targets()
        .into_iter()
        .map(|(label, dir)| SkillTargetStatus {
            label,
            installed: dir.join("gws").join("SKILL.md").is_file(),
            version: skills_version(&dir),
            dir: dir.to_string_lossy().to_string(),
        })
        .collect();
    ToolVersions {
        desktop: env!("CARGO_PKG_VERSION").to_string(),
        cli,
        cli_path,
        skill_targets,
    }
}

// ══ 同步 ═══════════════════════════════════════════════════════════════

/// 把桌面端自带的 CLI 复制到 `dest`。
///
/// 返回 `(是否写了文件, 给用户看的一句话)`；本机版本更新时不动。
fn sync_cli_from(src: &Path, dest: &Path, version: &str) -> Result<(bool, String), String> {
    if dest.is_file() {
        let mut cmd = Command::new(dest);
        crate::platform::hide_console(&mut cmd);
        let installed = cmd
            .arg("--version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| parse_version(&String::from_utf8_lossy(&o.stdout)));
        if let Some(v) = installed {
            if is_newer(&v, version) {
                return Ok((
                    false,
                    format!("本机 CLI {v} 比桌面端 {version} 新，保留未覆盖"),
                ));
            }
            if v == version && same_file_contents(src, dest) {
                return Ok((false, format!("CLI 已是最新 {version}")));
            }
            let backup = dest.with_file_name(format!("{}.gws-backup", file_name(dest)));
            // 备份失败不阻断同步：原文件马上会被原子替换，最坏是没有回滚点
            let _ = std::fs::copy(dest, backup);
        }
    }

    let parent = dest
        .parent()
        .ok_or_else(|| format!("{} 没有父目录", dest.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("创建 {} 失败：{e}", parent.display()))?;

    // 先写同目录临时文件再 rename：跨文件系统 rename 不原子，中途失败
    // 也只会留下一个临时文件，不会把现有二进制写坏。
    let tmp = parent.join(format!(
        ".{}.gws-tmp-{}",
        file_name(dest),
        std::process::id()
    ));
    std::fs::copy(src, &tmp).map_err(|e| format!("复制 CLI 失败：{e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("设置执行权限失败：{e}"))?;
    }
    if let Err(e) = std::fs::rename(&tmp, dest) {
        // Windows 不允许 rename 覆盖已存在的文件
        #[cfg(windows)]
        {
            let _ = e;
            let _ = std::fs::remove_file(dest);
            std::fs::rename(&tmp, dest).map_err(|e| format!("替换 CLI 失败：{e}"))?;
        }
        #[cfg(not(windows))]
        {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("替换 CLI 失败：{e}"));
        }
    }
    Ok((
        true,
        format!("CLI 已更新到 {version}（{}）", dest.display()),
    ))
}

/// 版本号相同时也比内容：同一版本重新构建（本地 SNAPSHOT）后二进制会变，
/// 只比版本号会把旧 CLI 一直留在本机（实测：`gws new` 的默认值没生效）。
fn same_file_contents(a: &Path, b: &Path) -> bool {
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "gws".into())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct SkillStat {
    written: usize,
    backups: usize,
    skipped_newer: bool,
}

/// 把内置 skill 写进 `dir`。
///
/// 内容相同就完全不碰文件（避免每次启动改 mtime）；被改过的先备份再覆盖。
fn sync_skills_in(dir: &Path, version: &str, stat: &mut SkillStat) -> Result<(), String> {
    for (name, text) in BUNDLED_SKILLS {
        let skill_dir = dir.join(name);
        let marker = skill_dir.join(".gws-version");
        let installed = std::fs::read_to_string(&marker)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(v) = installed.as_deref() {
            if is_newer(v, version) {
                stat.skipped_newer = true;
                continue;
            }
        }

        std::fs::create_dir_all(&skill_dir)
            .map_err(|e| format!("创建 {} 失败：{e}", skill_dir.display()))?;
        let file = skill_dir.join("SKILL.md");
        let old = std::fs::read_to_string(&file).ok();
        if old.as_deref() != Some(text) {
            if old.is_some() {
                let backup = skill_dir.join("SKILL.md.gws-backup");
                std::fs::copy(&file, &backup)
                    .map_err(|e| format!("备份 {} 失败：{e}", file.display()))?;
                stat.backups += 1;
            }
            std::fs::write(&file, text)
                .map_err(|e| format!("写入 {} 失败：{e}", file.display()))?;
            stat.written += 1;
        }
        if installed.as_deref() != Some(version) {
            std::fs::write(&marker, format!("{version}\n"))
                .map_err(|e| format!("写入 {} 失败：{e}", marker.display()))?;
        }
    }
    Ok(())
}

/// 把内置 skill 同步到每个已检测到的 AI CLI 目录。
/// 返回 `(给人看的一句话, 是否写了文件)`。
fn sync_skills_all(targets: &[(String, PathBuf)], version: &str) -> (String, bool) {
    if targets.is_empty() {
        return (
            "没有找到已安装的 AI CLI skills 目录（Claude Code / OpenCode / Qoder CLI / Codex），\
             跳过；可用 GWS_SKILLS_DIR 指定"
                .to_string(),
            false,
        );
    }

    let mut parts = Vec::new();
    let mut changed = false;
    let mut backups = 0usize;
    for (label, dir) in targets {
        let mut stat = SkillStat::default();
        match sync_skills_in(dir, version, &mut stat) {
            Ok(()) => {
                changed |= stat.written > 0;
                backups += stat.backups;
                let mut state = if stat.written > 0 {
                    format!("已同步 {} 个", stat.written)
                } else if stat.skipped_newer {
                    "本机版本更新，保留未覆盖".to_string()
                } else {
                    "已是最新".to_string()
                };
                if stat.written > 0 && stat.skipped_newer {
                    state.push_str("（部分技能本机更新，保留）");
                }
                parts.push(format!("{label} {state}"));
            }
            Err(e) => parts.push(format!("{label} 同步失败：{e}")),
        }
    }

    let mut msg = format!("skills：{}", parts.join("；"));
    if backups > 0 {
        msg.push_str(&format!("（原文件备份为 *.gws-backup，{backups} 个）"));
    }
    (msg, changed)
}

fn bundled_cli(app: &tauri::AppHandle) -> Option<PathBuf> {
    let dir = app.path().resource_dir().ok()?;
    let name = exe_name();
    [
        // tauri 会把 `resources/*` 放进 resource_dir 下的 resources/
        dir.join("resources").join(name),
        dir.join(name),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// 同步 CLI 与 skills。**不返回 Err**：任何一步失败都只影响那一行提示，
/// 不能因为同步失败把设置页或启动流程弄挂。
pub fn sync_bundled(app: &tauri::AppHandle) -> ToolSyncOutcome {
    let version = env!("CARGO_PKG_VERSION");
    let mut changed = false;

    let cli = match bundled_cli(app) {
        Some(src) => match cli_dest() {
            Some(dest) => match sync_cli_from(&src, &dest, version) {
                Ok((wrote, msg)) => {
                    changed |= wrote;
                    msg
                }
                Err(e) => format!("CLI 同步失败：{e}"),
            },
            None => "找不到用户主目录，CLI 未同步".to_string(),
        },
        None => "桌面端未内置 CLI（开发模式），CLI 未同步".to_string(),
    };

    let skills = if std::env::var("GWS_NO_SKILLS")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        "已按 GWS_NO_SKILLS=1 跳过 skills".to_string()
    } else {
        let (msg, wrote) = sync_skills_all(&skills_targets(), version);
        changed |= wrote;
        msg
    };

    ToolSyncOutcome {
        changed,
        cli,
        skills,
    }
}

/// 设置页「立即同步」。启动时也会自动跑一次（仅 release 构建）。
#[tauri::command]
pub async fn sync_tools(app: tauri::AppHandle) -> super::CmdResult<ToolSyncOutcome> {
    super::bg(move || Ok(sync_bundled(&app))).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_the_last_token() {
        assert_eq!(parse_version("gws 1.0.5\n").as_deref(), Some("1.0.5"));
        assert_eq!(parse_version("gws-cli 1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(parse_version("   ").as_deref(), None);
    }

    #[test]
    fn numeric_versions_compare_properly() {
        assert!(is_newer("1.0.10", "1.0.9"));
        assert!(is_newer("1.2", "1.1.9"));
        assert!(!is_newer("1.0.5", "1.0.5"));
        assert!(!is_newer("1.0.4", "1.0.5"));
        assert!(!is_newer("dev", "1.0.5"));
    }

    #[cfg(unix)]
    #[test]
    fn same_version_cli_with_different_content_is_refreshed() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::TempDir::new().unwrap();
        let src = tmp.path().join("bundled-gws");
        let dest = tmp.path().join("gws");
        let script = |marker: &str| {
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'gws 1.0.5'; exit 0; fi\necho {marker}\n"
            )
        };
        std::fs::write(&src, script("new")).unwrap();
        std::fs::write(&dest, script("old")).unwrap();
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).unwrap();

        let (wrote, _) = sync_cli_from(&src, &dest, "1.0.5").unwrap();
        assert!(wrote, "版本号相同但内容不同（同版本重新构建）也要更新");
        assert_eq!(
            std::fs::read_to_string(&dest).unwrap(),
            std::fs::read_to_string(&src).unwrap()
        );

        let (wrote, _) = sync_cli_from(&src, &dest, "1.0.5").unwrap();
        assert!(!wrote, "内容一致就不该反复写");
    }

    #[test]
    fn skills_are_written_then_left_alone() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();

        let mut stat = SkillStat::default();
        sync_skills_in(dir, "1.0.5", &mut stat).unwrap();
        // 跟随实际内置数量，别写死 —— 增删 skill 时这里不该跟着改
        assert_eq!(stat.written, BUNDLED_SKILLS.len());
        for (name, text) in BUNDLED_SKILLS {
            assert_eq!(
                std::fs::read_to_string(dir.join(name).join("SKILL.md")).unwrap(),
                text,
                "{name} 内容应与仓库里的一致"
            );
        }
        assert_eq!(skills_version(dir).as_deref(), Some("1.0.5"));

        // 再跑一次：内容没变就不写
        let mut stat = SkillStat::default();
        sync_skills_in(dir, "1.0.5", &mut stat).unwrap();
        assert_eq!(stat.written, 0);
        assert_eq!(stat.backups, 0);
    }

    #[test]
    fn modified_skill_is_backed_up_before_overwrite() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        let mut stat = SkillStat::default();
        sync_skills_in(dir, "1.0.5", &mut stat).unwrap();

        let gws = dir.join("gws/SKILL.md");
        std::fs::write(&gws, "我自己改的\n").unwrap();
        std::fs::write(dir.join("gws/.gws-version"), "1.0.4\n").unwrap();

        let mut stat = SkillStat::default();
        sync_skills_in(dir, "1.0.5", &mut stat).unwrap();
        assert_eq!(stat.written, 1);
        assert_eq!(stat.backups, 1);
        assert_eq!(
            std::fs::read_to_string(dir.join("gws/SKILL.md.gws-backup")).unwrap(),
            "我自己改的\n"
        );
        assert_eq!(std::fs::read_to_string(&gws).unwrap(), BUNDLED_SKILLS[0].1);
    }

    #[test]
    fn newer_local_skill_is_kept() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        let mut stat = SkillStat::default();
        sync_skills_in(dir, "1.0.5", &mut stat).unwrap();

        std::fs::write(dir.join("gws/.gws-version"), "1.0.6\n").unwrap();
        std::fs::write(dir.join("gws/SKILL.md"), "更新的版本\n").unwrap();

        let mut stat = SkillStat::default();
        sync_skills_in(dir, "1.0.5", &mut stat).unwrap();
        assert!(stat.skipped_newer);
        assert_eq!(stat.written, 0);
        assert_eq!(
            std::fs::read_to_string(dir.join("gws/SKILL.md")).unwrap(),
            "更新的版本\n"
        );
    }

    #[test]
    fn only_existing_cli_skill_dirs_are_targets() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude/skills")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".qoder/skills")).unwrap();

        let targets = skills_targets_in(tmp.path());
        let labels: Vec<&str> = targets.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["Claude Code", "Qoder CLI"]);

        // 没装的 CLI 不凭空造目录（实测要求：没有就不新建）
        assert!(!tmp.path().join(".codex/skills").exists());
        assert!(!tmp.path().join(".config/opencode/skills").exists());
    }

    #[test]
    fn skills_are_synced_to_every_detected_cli() {
        let tmp = tempfile::TempDir::new().unwrap();
        let claude = tmp.path().join(".claude/skills");
        let qoder = tmp.path().join(".qoder/skills");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::create_dir_all(&qoder).unwrap();

        let targets = skills_targets_in(tmp.path());
        let (msg, changed) = sync_skills_all(&targets, "1.0.5");
        assert!(changed, "{msg}");
        assert!(msg.contains("Claude Code"), "{msg}");
        assert!(msg.contains("Qoder CLI"), "{msg}");

        for dir in [&claude, &qoder] {
            for (name, text) in BUNDLED_SKILLS {
                assert_eq!(
                    std::fs::read_to_string(dir.join(name).join("SKILL.md")).unwrap(),
                    text,
                    "{name} 应写进 {}",
                    dir.display()
                );
            }
            assert_eq!(skills_version(dir).as_deref(), Some("1.0.5"));
        }

        // 再跑一次：内容没变就不写
        let (msg, changed) = sync_skills_all(&targets, "1.0.5");
        assert!(!changed);
        assert!(msg.contains("已是最新"), "{msg}");
    }

    #[test]
    fn no_cli_means_no_skill_writes() {
        let (msg, changed) = sync_skills_all(&[], "1.0.5");
        assert!(!changed);
        assert!(msg.contains("没有找到已安装的 AI CLI"), "{msg}");
    }

    #[test]
    fn cli_is_copied_and_made_executable() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = tmp.path().join("src-gws");
        let dest = tmp.path().join("bin/gws");
        std::fs::write(&src, b"#!/bin/sh\necho gws 1.0.5\n").unwrap();

        let (wrote, msg) = sync_cli_from(&src, &dest, "1.0.5").unwrap();
        assert!(wrote, "{msg}");
        assert!(dest.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                std::fs::metadata(&dest).unwrap().permissions().mode() & 0o111,
                0
            );
        }
    }

    /// skill 是给 AI 看的操作说明：必须把「先问用户、与界面一致」写在最显眼处，
    /// 否则模型会自行补模块/分类/评审人，造出界面上不会出现的脏数据。
    #[test]
    fn skills_tell_the_ai_to_ask_before_filling_parameters() {
        let gws = BUNDLED_SKILLS
            .iter()
            .find(|(name, _)| *name == "gws")
            .map(|(_, text)| *text)
            .expect("应内置 gws 技能");
        assert!(gws.contains("执行纪律"), "开头要有执行纪律一节");
        assert!(gws.contains("先问用户"), "必填参数必须先问用户");
        assert!(
            gws.contains("所有字段都先问一遍，包括选填"),
            "选填也要问，不能静默用默认"
        );
        assert!(gws.contains("不填用默认"), "选填要说明不填时的默认值");
        assert!(
            gws.contains("留空 = 只挂文档库，不拉任何代码"),
            "新建需求的模块默认要和界面一致"
        );
        assert!(gws.contains("留空 = 按年月归组"), "分类默认要和界面一致");
        assert!(gws.contains("不要自己编分类名"), "禁止臆造分类");
        assert!(gws.contains("不要凭印象写模块名"), "禁止臆造模块");
    }
}
