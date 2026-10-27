//! 修复被搬动破坏的 **worktree 链接**。
//!
//! # 为什么需要自动做这件事
//!
//! git worktree 的链接是**绝对路径**，写在两处：
//!
//! 1. 每个 worktree 的 `.git` 文件：`gitdir: <hub>/repos/<仓库>/.git/worktrees/<id>`
//! 2. 主 clone 的 `.git/worktrees/<id>/gitdir`：指回 worktree
//!
//! 所以**把 hub 目录整体搬走**（换盘、归档、整理目录）之后，两处**都指向旧路径**，
//! 全部 worktree 一起坏掉，git 只会说 `fatal: not a git repository`。
//! 实测踩过：搬完 46 个 worktree 全坏，界面上满屏报错。
//!
//! git 官方为此提供了 `git worktree repair`：主工作树与链接工作树**一起**移动时，
//! 从主 clone 里跑它、并把各 worktree 的**新路径**传进去，两个方向都能修好。
//!
//! 这里把它做进「打开 hub」：**只在真的坏了时才动手**（正常打开一次文件读都不多），
//! 并把「修了几个」如实报给界面（走 `migration_notes` 那条通道）。

use std::path::{Path, PathBuf};

use crate::git::Git;
use crate::hub::Hub;

/// worktree 链接的体检结果。
enum Health {
    /// 不是 worktree（没有 `.git` 文件），跳过。
    NotAWorktree,
    /// 链接完好。
    Ok,
    /// 坏了；附带**它归属于哪个仓库**（从旧路径里解析出来的名字）。
    Broken { repo: String },
}

/// 读 `<wt>/.git` 里的 `gitdir:` 目标，判断这条链接还通不通。
///
/// 判「好」的条件（三个文件都在、且互指）：目标目录存在、它的 `gitdir` 存在、
/// 且里面写的正是这个 worktree 的 `.git`。**纯文件读**，不起子进程 ——
/// 打开 hub 是高频路径，不能每次都跑 46 个 `git`。
fn health(wt: &Path) -> Health {
    let dotgit = wt.join(".git");
    let Ok(text) = std::fs::read_to_string(&dotgit) else {
        return Health::NotAWorktree;
    };
    let Some(target) = text.trim().strip_prefix("gitdir:") else {
        return Health::NotAWorktree;
    };
    let target = PathBuf::from(target.trim());

    // 由旧路径解析出仓库名：`<任意>/repos/<仓库>/.git/worktrees/<id>`
    let repo = target
        .to_string_lossy()
        .split("/.git/worktrees/")
        .next()
        .and_then(|before| Path::new(before).file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| {
            // 兜底：worktree 目录名就是仓库名（布局约定）
            wt.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
        });

    // 反向那一条：主 clone 里记录它指向谁
    let back = std::fs::read_to_string(target.join("gitdir")).unwrap_or_default();
    let back_ok = Path::new(back.trim()) == dotgit;

    if target.is_dir() && back_ok {
        Health::Ok
    } else {
        Health::Broken { repo }
    }
}

/// 扫出 hub 里所有「像 worktree 的目录」。
///
/// 只认布局约定的两个位置（`envs/<环境>/<仓库>`、`ws/<分支>/<仓库>`）——
/// 不递归全盘找 `.git` 文件，那在真实 hub 上会很慢。
fn candidates(hub: &Hub) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for area in [hub.envs_dir(), hub.ws_dir()] {
        let Ok(level1) = std::fs::read_dir(&area) else { continue };
        for a in level1.flatten() {
            // `ws/<分支>/<仓库>` 比 `envs/<环境>/<仓库>` 多一层？不 —— 两者都是两层
            let dir = a.path();
            if !dir.is_dir() {
                continue;
            }
            let Ok(modules) = std::fs::read_dir(&dir) else { continue };
            for m in modules.flatten() {
                let p = m.path();
                if p.join(".git").is_file() {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// 修复所有失效的 worktree 链接，返回给人看的说明（空 = 什么都没坏）。
pub fn repair_broken_worktrees(hub: &Hub) -> Vec<String> {
    let mut fixed: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    for wt in candidates(hub) {
        let Health::Broken { repo } = health(&wt) else { continue };
        // 主 clone 按布局在 `repos/<仓库>`；仓库名解析不出来就跳过（不动手比乱动好）
        let repo_dir = hub.repos_dir().join(&repo);
        if !repo_dir.is_dir() {
            continue;
        }
        let rel = wt.strip_prefix(hub.root()).unwrap_or(&wt).to_string_lossy().to_string();
        match Git::new(&repo_dir).worktree_repair(&wt) {
            Ok(()) => fixed.push(rel),
            Err(e) => failed.push(format!("{rel}（{e}）")),
        }
    }

    let mut notes = Vec::new();
    if !fixed.is_empty() {
        notes.push(format!(
            "已修复 {} 个失效的 worktree 链接（hub 目录被移动过？git 的 worktree \
             链接是绝对路径，搬走就会全部失效）",
            fixed.len()
        ));
        // 逐个列出来：动了什么必须能看见（本项目的一贯要求）
        for r in &fixed {
            notes.push(format!("  ✓ {r}"));
        }
    }
    for f in &failed {
        notes.push(format!("⚠ worktree 修复失败：{f}"));
    }
    notes
}
