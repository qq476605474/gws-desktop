//! 改分支名：没推送时不需要 GitLab 令牌。
//!
//! 真实场景：建需求时把日期写重了，得到
//! `bugfix-20261020-20261020-ca-mobile-vault`，还没推送就想改掉。

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use gws_core::hub::{Hub, InitOptions};
use gws_core::repo;
use gws_core::workspace::branch;
use gws_core::workspace::lifecycle::{self, NewOptions};
use gws_core::workspace::{ModuleFilter, Workspace};

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("无法执行 git: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} 失败:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn seeded_origin(root: &Path, name: &str) -> PathBuf {
    let bare = root.join(format!("{name}.git"));
    git(
        root,
        &["init", "--bare", "-b", "main", bare.to_str().unwrap()],
    );

    let seed = root.join(format!("{name}-seed"));
    git(root, &["init", "-b", "main", seed.to_str().unwrap()]);
    std::fs::write(seed.join("README.md"), format!("# {name}\n")).unwrap();
    git(&seed, &["add", "-A"]);
    git(
        &seed,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "init",
        ],
    );
    git(&seed, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&seed, &["push", "-q", "origin", "main"]);
    bare
}

struct Fixture {
    _tmp: TempDir,
    hub: Hub,
}

impl Fixture {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let mut hub = Hub::init(&tmp.path().join("hub"), InitOptions::default()).unwrap();
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
        let a = seeded_origin(tmp.path(), "demo-web");
        repo::add(&hub, &[a.to_string_lossy().into()], None).unwrap();
        Self { _tmp: tmp, hub }
    }

    fn create(&self, branch: &str) {
        lifecycle::new(
            &self.hub,
            &NewOptions {
                branch: branch.to_string(),
                title: "收银台改版".to_string(),
                prefix: "bugfix".to_string(),
                base: vec![],
                modules: vec!["demo-web".to_string()],
                doc_family: None,
                notes: String::new(),
                links: Vec::new(),
            },
        )
        .unwrap();
    }
}

#[test]
fn unpushed_branch_can_be_renamed_without_a_token() {
    let f = Fixture::new();
    let old = "bugfix-20261020-20261020-ca-mobile-vault";
    let new = "bugfix-20261020-ca-mobile-vault";
    f.create(old);

    let ws = Workspace::open(&f.hub, old).unwrap();
    let plan = branch::rename_plan(&f.hub, &ws, &ModuleFilter::all(), new, None).unwrap();
    assert!(
        plan.can_apply(),
        "未推送时应可纯本地改名: {:?}",
        plan.problems
    );
    assert_eq!(plan.modules.len(), 1);
    assert!(!plan.modules[0].exists, "这条分支从未推送过");

    let mut ws = ws;
    let out = branch::rename_apply(&f.hub, &mut ws, &ModuleFilter::all(), new, None).unwrap();
    assert_eq!(out.renamed, vec!["demo-web"]);
    assert!(out.pushed.is_empty(), "没推送就不该尝试推送");
    assert!(out.failed.is_empty(), "{:?}", out.failed);

    assert!(!f.hub.ws_dir().join(old).exists(), "旧目录应消失");
    let new_dir = f.hub.ws_dir().join(new);
    assert!(new_dir.is_dir());
    assert_eq!(
        git(
            &new_dir.join("demo-web"),
            &["branch", "--show-current"]
        ),
        new
    );

    let reopened = Workspace::open(&f.hub, new).unwrap();
    assert_eq!(reopened.branch(), new);
    assert_eq!(reopened.meta().prefix, "bugfix");
}

#[test]
fn pushed_branch_without_a_token_is_rejected_instead_of_half_renamed() {
    let f = Fixture::new();
    let old = "feature-20261020-ca-mobile-vault";
    f.create(old);
    let wt = f.hub.ws_dir().join(old).join("demo-web");
    git(&wt, &["push", "-q", "-u", "origin", old]);

    let ws = Workspace::open(&f.hub, old).unwrap();
    let plan = branch::rename_plan(
        &f.hub,
        &ws,
        &ModuleFilter::all(),
        "feature-20261020-fixed",
        None,
    )
    .unwrap();
    assert!(!plan.can_apply());
    let text = plan.problems.join("\n");
    assert!(text.contains("GitLab 令牌"), "{text}");
    assert!(text.contains("demo-web"), "{text}");

    // 什么都没动
    assert!(f.hub.ws_dir().join(old).is_dir());
    assert_eq!(git(&wt, &["branch", "--show-current"]), old);
}

#[test]
fn existing_target_directory_blocks_rename_before_any_change() {
    let f = Fixture::new();
    let old = "feature-20261020-old";
    f.create(old);
    // 目标目录已存在：不能先把本地分支改掉再失败
    std::fs::create_dir_all(f.hub.ws_dir().join("feature-20261020-taken")).unwrap();

    let ws = Workspace::open(&f.hub, old).unwrap();
    let plan = branch::rename_plan(
        &f.hub,
        &ws,
        &ModuleFilter::all(),
        "feature-20261020-taken",
        None,
    )
    .unwrap();
    assert!(!plan.can_apply());
    assert!(
        plan.problems
            .iter()
            .any(|p| p.contains("目标工作区目录已存在")),
        "{:?}",
        plan.problems
    );
}
