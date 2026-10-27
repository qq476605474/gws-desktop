//! 「与基线」对比：按配置链**倒序**取最后一个可用基线，缺失时逐个降级，最后兜底主干。

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use gws_core::hub::{Hub, InitOptions};
use gws_core::repo;
use gws_core::workspace::lifecycle::{self, NewOptions};

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
    bare: PathBuf,
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
        let bare = seeded_origin(tmp.path(), "demo-web");
        repo::add(&hub, &[bare.to_string_lossy().into()], None).unwrap();
        Self {
            _tmp: tmp,
            hub,
            bare,
        }
    }

    fn remote_branch(&self, name: &str) {
        git(&self.bare, &["branch", name, "main"]);
    }
}

fn v(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn picks_the_last_configured_candidate_not_the_newest_date() {
    let f = Fixture::new();
    f.remote_branch("feature-20261001-old");
    f.remote_branch("feature-20261020-new");

    // 链尾优先：后配置的 new 胜出
    let r = lifecycle::resolve_latest_base(
        &f.hub,
        "demo-web",
        &v(&["feature-20261001-old", "feature-20261020-new"]),
    )
    .unwrap();
    assert_eq!(r.reference, "origin/feature-20261020-new");
    assert!(!r.fell_back_to_main);
    assert!(r.missed.is_empty(), "{:?}", r.missed);

    // 反过来配置也是链尾优先：日期更旧但写在后面，仍然选它
    let r = lifecycle::resolve_latest_base(
        &f.hub,
        "demo-web",
        &v(&["feature-20261020-new", "feature-20261001-old"]),
    )
    .unwrap();
    assert_eq!(r.reference, "origin/feature-20261001-old");
}

#[test]
fn degrades_to_the_next_candidate_when_the_last_is_missing() {
    let f = Fixture::new();
    f.remote_branch("feature-20261001-old");

    let r = lifecycle::resolve_latest_base(
        &f.hub,
        "demo-web",
        &v(&["feature-20261001-old", "feature-20261025-missing"]),
    )
    .unwrap();
    assert_eq!(r.reference, "origin/feature-20261001-old");
    assert!(!r.fell_back_to_main);
    assert_eq!(r.missed, v(&["feature-20261025-missing"]));
    assert!(r.note().unwrap().contains("feature-20261025-missing"));
}

#[test]
fn falls_back_to_main_when_nothing_matches() {
    let f = Fixture::new();
    let r = lifecycle::resolve_latest_base(
        &f.hub,
        "demo-web",
        &v(&["feature-20261001-a", "feature-20261020-b"]),
    )
    .unwrap();
    assert_eq!(r.reference, "origin/main");
    assert!(r.fell_back_to_main);
    assert_eq!(r.missed.len(), 2);
}

#[test]
fn ws_upstream_is_expanded_and_used_in_chain_order() {
    let f = Fixture::new();
    lifecycle::new(
        &f.hub,
        &NewOptions {
            branch: "feature-20261015-upstream".into(),
            title: "上游需求".into(),
            prefix: "feature".into(),
            base: vec![],
            modules: vec!["demo-web".into()],
            doc_family: None,
            notes: String::new(),
            links: Vec::new(),
        },
    )
    .unwrap();
    let wt = f
        .hub
        .ws_dir()
        .join("feature-20261015-upstream/demo-web");
    git(
        &wt,
        &["push", "-q", "-u", "origin", "feature-20261015-upstream"],
    );

    f.remote_branch("feature-20261001-old");
    let r = lifecycle::resolve_latest_base(
        &f.hub,
        "demo-web",
        &v(&["feature-20261001-old", "ws:feature-20261015-upstream"]),
    )
    .unwrap();
    assert_eq!(r.reference, "origin/feature-20261015-upstream");
}

#[test]
fn chain_order_wins_regardless_of_date_form() {
    let f = Fixture::new();
    f.remote_branch("feature-20261001-old");
    f.remote_branch("feature-261020-six");
    f.remote_branch("feature-2026-10-30-dashed");

    // 链尾是 six，即使 dashed 的日期更新也选 six
    let r = lifecycle::resolve_latest_base(
        &f.hub,
        "demo-web",
        &v(&[
            "feature-2026-10-30-dashed",
            "feature-20261001-old",
            "feature-261020-six",
        ]),
    )
    .unwrap();
    assert_eq!(r.reference, "origin/feature-261020-six");
}

#[test]
fn missing_upstream_workspace_is_reported_not_fatal() {
    let f = Fixture::new();
    f.remote_branch("feature-20261001-old");
    let r = lifecycle::resolve_latest_base(
        &f.hub,
        "demo-web",
        &v(&["ws:feature-20990101-gone", "feature-20261001-old"]),
    )
    .unwrap();
    assert_eq!(r.reference, "origin/feature-20261001-old");
    assert_eq!(r.missed, v(&["ws:feature-20990101-gone"]));
}
