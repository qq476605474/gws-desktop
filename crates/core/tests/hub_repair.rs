//! 打开 hub 时自动修复**被搬动破坏的 worktree 链接**。
//!
//! 场景来自实测：把 hub 目录整体搬走（换盘/归档）之后，**所有** worktree 都坏了
//! —— git worktree 的链接是绝对路径，写在两处：
//!   1. 各 worktree 的 `.git`：`gitdir: <hub>/repos/<仓库>/.git/worktrees/<id>`
//!   2. 主 clone 的 `.git/worktrees/<id>/gitdir`：指回 worktree
//!
//! 搬完两处都指向旧路径，git 只说 `fatal: not a git repository`。
//!
//! 这个测试**真的把目录搬一次**（`fs::rename`），先证明它确实坏了，
//! 再打开 hub 看它有没有自己修好 —— 不是 mock。

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use gws_core::hub::{Hub, InitOptions};
use gws_core::repo;
use gws_core::workspace::lifecycle::{self, NewOptions};
use gws_core::workspace::sync;

const MODULE: &str = "demo-api";

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(cwd).args(args).output().expect("git 可执行");
    assert!(out.status.success(), "git {args:?} 失败:\n{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit_file(dir: &Path, file: &str, content: &str, msg: &str) {
    std::fs::write(dir.join(file), content).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", msg]);
}

fn usable(dir: &Path) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(["status", "--porcelain"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct Fixture {
    _tmp: TempDir,
    work: PathBuf,
    hub_root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let work = tmp.path().to_path_buf();

        let origin = work.join("demo-api.git");
        git(&work, &["init", "--bare", "-b", "main", origin.to_str().unwrap()]);
        let seed = work.join("seed");
        git(&work, &["init", "-b", "main", seed.to_str().unwrap()]);
        commit_file(&seed, "README.md", "# seed\n", "init");
        git(&seed, &["remote", "add", "origin", origin.to_str().unwrap()]);
        git(&seed, &["push", "-q", "origin", "main"]);
        git(&origin, &["branch", "dev", "main"]);

        let hub_root = work.join("hub");
        let mut hub = Hub::init(&hub_root, InitOptions::default()).unwrap();
        // 单测不联网：直接写 GitLab 绑定（`repo::add` 等操作要求 hub 已绑定）。
        // 测试里的仓库都是本地路径，`check_remote` 一律放行，不会真去连这个地址。
        hub.config_mut().gitlab.base_url = "https://gitlab.example.com".into();
        // 提交身份：**别指望机器上的 git 全局配置**（CI runner 上没有，产品又会主动
        // 清掉 local 身份 → `git merge` 直接以 `Committer identity unknown` 失败）。
        hub.config_mut().git = gws_core::hub::GitIdentity {
            author_name: "测试".into(),
            author_email: "test@example.com".into(),
        };
        hub.save_config().unwrap();
        repo::add(&hub, &[origin.to_string_lossy().into()], None).unwrap();
        lifecycle::new(
            &hub,
            &NewOptions {
                branch: "feature-20261014-move".into(),
                title: "搬家测试".into(),
                prefix: "feature".into(),
                base: vec!["主干".into()],
                modules: vec![MODULE.into()],
                doc_family: None,
                notes: String::new(),
                links: Vec::new(),
            },
        )
        .unwrap();
        // 环境 worktree 也建出来（sync 时补建）
        sync::sync_all(&hub, None).unwrap();

        Self { _tmp: tmp, work, hub_root }
    }
}

#[test]
fn moving_the_hub_breaks_every_worktree_and_opening_it_repairs_them() {
    let f = Fixture::new();
    let ws_wt = f.hub_root.join("ws/feature-20261014-move").join(MODULE);
    let env_wt = f.hub_root.join("envs/dev").join(MODULE);
    assert!(usable(&ws_wt), "搬之前应当可用");
    assert!(usable(&env_wt), "搬之前应当可用");

    // ── 真的搬一次（换盘/归档就是这件事）──
    let moved = f.work.join("hub-moved");
    std::fs::rename(&f.hub_root, &moved).unwrap();

    let moved_ws = moved.join("ws/feature-20261014-move").join(MODULE);
    let moved_env = moved.join("envs/dev").join(MODULE);

    // ① 先证明**确实坏了** —— 否则这个测试证明不了修复有用
    assert!(!usable(&moved_ws), "搬完应当坏掉（绝对路径的 worktree 链接失效）");
    assert!(!usable(&moved_env), "搬完应当坏掉");

    // ② 打开 hub —— 应当**自动修好**
    let hub = Hub::open(&moved).unwrap();

    assert!(usable(&moved_ws), "打开 hub 后文档/需求里的 worktree 应当能用了");
    assert!(usable(&moved_env), "环境里的 worktree 也应当能用了");
    // 分支仍然对（修的是链接，不该动检出内容）
    assert_eq!(git(&moved_ws, &["branch", "--show-current"]), "feature-20261014-move");
    assert_eq!(git(&moved_env, &["branch", "--show-current"]), "dev");

    // ③ 「动了什么」必须能看见
    let notes = hub.repairs();
    assert!(!notes.is_empty(), "修复了就要报出来，不能悄悄改");
    assert!(
        notes.iter().any(|n| n.contains("已修复")),
        "说明里要讲清修了几个：{notes:?}"
    );
}

#[test]
fn a_healthy_hub_is_left_alone() {
    // 没坏时**不做任何事**（也不该报「我动了什么」）—— 打开 hub 是高频路径
    let f = Fixture::new();
    let hub = Hub::open(&f.hub_root).unwrap();
    assert!(hub.repairs().is_empty(), "没坏就不该动它: {:?}", hub.repairs());
}
