//! 仓库生命周期的集成测试。
//!
//! **用真实的裸仓库，不 mock git。** 本工具的核心价值就是「与人工敲 git 命令
//! 行为一致」，mock 掉 git 等于把最该验证的部分挖空。
//!
//! 每个测试在自己独立的 `TempDir` 里造一个裸仓库当 origin，完全不碰网络。

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use gws_core::hub::{Hub, InitOptions};
use gws_core::repo::{self, Outcome};

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(cwd).args(args).output().unwrap_or_else(|e| {
        panic!("无法执行 git: {e}");
    });
    assert!(
        out.status.success(),
        "git {args:?} 失败:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 造一个带初始提交的裸仓库当 origin，返回其路径。
///
/// 同时造出 `main` 与 `master` 两种主干名——真实环境里两者混用（设计方案 §5.3），
/// 写死任何一个都会在另一批仓库上失败。
fn seeded_origin(root: &Path, name: &str, branch: &str) -> PathBuf {
    let bare = root.join(format!("{name}.git"));
    git(root, &["init", "--bare", "-b", branch, bare.to_str().unwrap()]);

    let seed = root.join(format!("{name}-seed"));
    git(root, &["init", "-b", branch, seed.to_str().unwrap()]);
    std::fs::write(seed.join("README.md"), format!("# {name}\n")).unwrap();
    git(&seed, &["add", "-A"]);
    git(
        &seed,
        &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "init"],
    );
    git(&seed, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&seed, &["push", "-q", "origin", branch]);
    bare
}

/// 测试脚手架：临时目录 + 一个 hub + 一个带 main 的 origin。
struct Fixture {
    _tmp: TempDir,
    hub: Hub,
    origin: PathBuf,
}

fn fixture() -> Fixture {
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
    let origin = seeded_origin(tmp.path(), "demo-web", "main");
    Fixture { _tmp: tmp, hub, origin }
}

impl Fixture {
    fn add_origin(&self) -> Vec<repo::RepoOutcome> {
        repo::add(&self.hub, &[self.origin.to_string_lossy().to_string()], None).unwrap()
    }

    fn names(&self) -> Vec<String> {
        repo::list(&self.hub).unwrap().into_iter().map(|r| r.name).collect()
    }
}

#[test]
fn add_clones_repo_into_repos_dir() {
    let f = fixture();
    let results = f.add_origin();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "demo-web");
    assert_eq!(results[0].outcome, Outcome::Ok);

    let path = f.hub.repos_dir().join("demo-web");
    assert!(path.join(".git").exists(), "应是一个真实的 clone");
    assert!(path.join("README.md").exists(), "应有 checkout 出来的文件");
    assert_eq!(f.names(), vec!["demo-web"]);
}

#[test]
fn add_is_idempotent() {
    let f = fixture();
    f.add_origin();

    let again = f.add_origin();
    assert_eq!(again[0].outcome, Outcome::Skipped("已存在".to_string()));
    // 重复添加不应产生第二份
    assert_eq!(f.names().len(), 1);
}

#[test]
fn add_reports_failure_per_url_without_aborting_the_batch() {
    let f = fixture();
    let good = f.origin.to_string_lossy().to_string();
    let bad = f._tmp.path().join("does-not-exist.git").to_string_lossy().to_string();

    let results = repo::add(&f.hub, &[bad.clone(), good], None).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results[0].outcome.is_failed(), "不存在的地址应失败");
    assert_eq!(results[1].outcome, Outcome::Ok, "后一个好地址仍应成功");
    assert_eq!(f.names(), vec!["demo-web"]);
}

#[test]
fn failed_clone_leaves_no_half_baked_directory() {
    let f = fixture();
    let bad = f._tmp.path().join("nope.git").to_string_lossy().to_string();
    repo::add(&f.hub, &[bad], None).unwrap();

    // 半个 clone 会以「含 .git 的空仓库」形态混进列表，必须清掉
    assert_eq!(f.names(), Vec::<String>::new());
}

#[test]
fn add_rejects_url_without_usable_name() {
    let f = fixture();
    let results = repo::add(&f.hub, &["https://host/".to_string()], None).unwrap();
    assert!(results[0].outcome.is_failed());
}

#[test]
fn detects_main_branch_for_main_and_master() {
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

    for branch in ["main", "master"] {
        let origin = seeded_origin(tmp.path(), &format!("repo-{branch}"), branch);
        repo::add(&hub, &[origin.to_string_lossy().to_string()], None).unwrap();

        let r = repo::Repo::open(&hub, &format!("repo-{branch}")).unwrap();
        assert_eq!(r.main_branch().unwrap(), branch, "主干探测错误: {branch}");
    }
}

#[test]
fn detects_renamed_main_branch_via_remote_head() {
    // 主干被改名成 develop 时，前两种探测都不命中，必须回退查远程 HEAD
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
    let origin = seeded_origin(tmp.path(), "weird", "develop");
    repo::add(&hub, &[origin.to_string_lossy().to_string()], None).unwrap();

    let r = repo::Repo::open(&hub, "weird").unwrap();
    assert_eq!(r.main_branch().unwrap(), "develop");
}

#[test]
fn repo_records_its_remote_url() {
    let f = fixture();
    f.add_origin();
    let r = repo::Repo::open(&f.hub, "demo-web").unwrap();
    assert_eq!(r.remote_url().unwrap(), f.origin.to_string_lossy());
}

#[test]
fn open_missing_repo_reports_not_found() {
    let f = fixture();
    let err = repo::Repo::open(&f.hub, "nope").unwrap_err();
    assert!(matches!(err, gws_core::Error::RepoNotFound(_)), "{err}");
}

#[test]
fn remove_deletes_unused_repo_and_verifies_it_is_gone() {
    let f = fixture();
    f.add_origin();

    let results = repo::remove(&f.hub, &["demo-web".to_string()]).unwrap();
    assert_eq!(results[0].outcome, Outcome::Ok);

    // 「删除后必须校验真的消失了」——旧版正是在这里静默失败还报成功
    assert!(!f.hub.repos_dir().join("demo-web").exists());
    assert_eq!(f.names(), Vec::<String>::new());
}

#[test]
fn remove_skips_unknown_repo() {
    let f = fixture();
    let results = repo::remove(&f.hub, &["ghost".to_string()]).unwrap();
    assert_eq!(results[0].outcome, Outcome::Skipped("不存在".to_string()));
}

#[test]
fn remove_refuses_repo_still_used_by_a_workspace() {
    let f = fixture();
    f.add_origin();

    // 模拟一个引用该仓库的需求工作区
    let ws = f.hub.ws_dir().join("feature-20261010-demo").join("demo-web");
    std::fs::create_dir_all(&ws).unwrap();

    let results = repo::remove(&f.hub, &["demo-web".to_string()]).unwrap();
    assert!(results[0].outcome.is_failed(), "被工作区占用时应拒绝");
    let msg = match &results[0].outcome {
        Outcome::Failed(m) => m.clone(),
        other => panic!("期望 Failed，实际 {other:?}"),
    };
    assert!(msg.contains("feature-20261010-demo"), "错误里应指出是谁在用: {msg}");
    assert!(f.hub.repos_dir().join("demo-web").exists(), "不应被删掉");
}



#[test]
fn usages_separate_workspaces_from_envs() {
    let f = fixture();
    f.add_origin();
    std::fs::create_dir_all(f.hub.ws_dir().join("req-a").join("demo-web")).unwrap();
    std::fs::create_dir_all(f.hub.env_dir("dev").join("demo-web")).unwrap();

    // 两类分开列：需求引用**拦删除**，环境检出**跟着删**
    // 报的是「需求名称（分支名）」；这里那目录没元数据，退回分支名
    assert_eq!(repo::workspace_usages(&f.hub, "demo-web"), vec!["req-a"]);
    assert_eq!(repo::env_usages(&f.hub, "demo-web"), vec!["envs/dev"]);
}

/// 被需求引用时，报的是**需求名称（分支名）**而不是 `ws/<分支>` 这种路径。
///
/// 实测反馈：提示里只给 `ws/feature-20261010-xxx` —— 那串日期人认不出来，
/// 得自己去翻是哪个需求。有需求名称就直接报名称。
#[test]
fn workspace_usages_report_the_requirement_name_not_the_path() {
    let f = fixture();
    f.add_origin();
    let dir = f.hub.ws_dir().join("feature-20261010-checkout");
    std::fs::create_dir_all(dir.join("demo-web")).unwrap();
    std::fs::write(
        dir.join(gws_core::workspace::META_FILE),
        r#"{"schema":2,"branch":"feature-20261010-checkout","title":"收银台改版","prefix":"feature","created":"2026-10-10","base":[],"modules":["demo-web"],"links":[],"notes":""}"#,
    )
    .unwrap();

    assert_eq!(
        repo::workspace_usages(&f.hub, "demo-web"),
        vec!["收银台改版（feature-20261010-checkout）"]
    );
}

/// 没起名字（名称 == 分支名）时只报分支名，不重复一遍。
#[test]
fn workspace_usages_do_not_repeat_the_branch_when_there_is_no_title() {
    let f = fixture();
    f.add_origin();
    let dir = f.hub.ws_dir().join("feature-20261010-x");
    std::fs::create_dir_all(dir.join("demo-web")).unwrap();

    // 连元数据都没有：退回分支名
    assert_eq!(repo::workspace_usages(&f.hub, "demo-web"), vec!["feature-20261010-x"]);
}

/// **环境里的检出不该拦住「移除仓库」。**
///
/// 实测反馈：原来把它和需求一视同仁，于是提示「它仍被这些地方使用：envs/dev。
/// 先删除对应的需求或环境」—— 可环境是常驻的，不可能为了删一个仓库先把 dev
/// 删掉，等于根本删不掉。
///
/// 环境里的是**镜像**，随仓库一起清掉即可；只有**需求**里有真实的开发工作，
/// 才需要拦。
#[test]
fn repo_with_only_env_checkout_is_removable_and_cleans_it_up() {
    let f = fixture();
    f.add_origin();
    let env_dir = f.hub.env_dir("dev").join("demo-web");
    std::fs::create_dir_all(&env_dir).unwrap();

    assert_eq!(
        repo::remove(&f.hub, &["demo-web".into()]).unwrap()[0].outcome,
        Outcome::Ok,
        "只被环境引用时应能删除"
    );
    assert!(!env_dir.exists(), "环境里的检出应随仓库一起删掉");
}

#[test]
fn repo_becomes_removable_once_workspace_is_dropped() {
    let f = fixture();
    f.add_origin();
    let ws = f.hub.ws_dir().join("req-a").join("demo-web");
    std::fs::create_dir_all(&ws).unwrap();

    assert!(repo::remove(&f.hub, &["demo-web".into()]).unwrap()[0]
        .outcome
        .is_failed());

    std::fs::remove_dir_all(&ws).unwrap();
    assert_eq!(
        repo::remove(&f.hub, &["demo-web".into()]).unwrap()[0].outcome,
        Outcome::Ok
    );
}
/// **半截克隆必须被修掉，不能当成「已存在」跳过。**
///
/// 实测反馈：其他人那边 `git worktree add` 报
/// `fatal: Could not reset index file to revision 'HEAD'` —— 根因是他的
/// `repos/<仓库>` 是上一次**被打断的克隆**留下的空壳，而 `repo add` 只看
/// 「目录在不在」，于是当成"已存在"跳过，谁也没发现副本是坏的。
#[test]
fn a_half_cloned_repo_is_recloned_instead_of_being_skipped() {
    let f = fixture();
    let dest = f.hub.repos_dir().join("demo-web");
    // 造一个只有 `.git` 目录、没有 HEAD 的空壳（正是被打断的克隆留下的形状）
    std::fs::create_dir_all(dest.join(".git")).unwrap();

    let out = repo::add(&f.hub, &[f.origin.to_string_lossy().to_string()], None).unwrap();

    assert!(
        out[0].outcome.is_ok(),
        "半截克隆要被重新拉取，而不是报「已存在」跳过：{:?}",
        out[0]
    );
    // 真的修好了：HEAD 解得出来、内容也在
    let git = gws_core::git::Git::new(&dest);
    assert!(git.out(&["rev-parse", "--verify", "HEAD"]).is_ok(), "重新克隆后应是个能用的仓库");
    assert!(dest.join("README.md").is_file(), "内容也要真的在");
}
