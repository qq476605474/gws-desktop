//! 「把需求配置复制给别人」的集成测试。
//!
//! 全程用**真实的裸仓库与 worktree**，不 mock。要验的是两件最容易出错的事：
//!
//! 1. **其他人那边没有的仓库要能自己补上** —— 配置里必须带着地址，而且补仓库
//!    失败时不能建出一个「少了两个模块」的需求
//! 2. 建出来的需求要跟导出方**一致**：模块集合、中文名、需求分类、基线、备注

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use gws_core::hub::{Hub, InitOptions};
use gws_core::repo;
use gws_core::workspace::share::{self, SharedModule, SharedRequirement, SHARE_SCHEMA};
use gws_core::workspace::{lifecycle, Workspace};

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(cwd).args(args).output().expect("git 可执行");
    assert!(out.status.success(), "git {args:?} 失败:\n{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 造一个裸仓库 + 一个 seed 检出（seed 的 origin 指向裸仓库，用来模拟「其他人推的分支」）。
fn seeded(root: &Path, name: &str) -> (PathBuf, PathBuf) {
    let bare = root.join(format!("{name}.git"));
    git(root, &["init", "--bare", "-b", "main", bare.to_str().unwrap()]);
    let seed = root.join(format!("{name}-seed"));
    git(root, &["init", "-b", "main", seed.to_str().unwrap()]);
    std::fs::write(seed.join("README.md"), format!("# {name}\n")).unwrap();
    git(&seed, &["add", "-A"]);
    git(&seed, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "init"]);
    git(&seed, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&seed, &["push", "-q", "origin", "main"]);
    (bare, seed)
}

/// 「其他人」在某仓库上开一条需求分支并推上去（构造出「远程有这条分支」）。
fn colleague_branch(seed: &Path, branch: &str, file: &str) {
    git(seed, &["checkout", "-q", "-b", branch]);
    std::fs::write(seed.join(file), "改了点什么\n").unwrap();
    git(seed, &["add", "-A"]);
    git(seed, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "work"]);
    git(seed, &["push", "-q", "origin", branch]);
}

struct Fixture {
    _tmp: TempDir,
    hub: Hub,
    /// 本地**已经有**的仓库。
    a_bare: PathBuf,
    /// A 的 seed 检出 —— 「其他人在这上面开分支」要用它。
    /// **不能拿 hub 里的 clone 当 seed**：那条分支会被主 clone 占住，
    /// 之后 `worktree add` 会失败（`already used by worktree`）。
    a_seed: PathBuf,
    /// 本地**没有**的仓库（其他人那边才有）—— 它才是这个功能的关键。
    b_bare: PathBuf,
    b_seed: PathBuf,
}

impl Fixture {
    /// `hub_has` 决定本地要不要先加 B。
    fn new(hub_has_b: bool) -> Self {
        let tmp = TempDir::new().unwrap();
        let work = tmp.path().to_path_buf();
        let mut hub = Hub::init(&work.join("hub"), InitOptions::default()).unwrap();
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

        let (a_bare, a_seed) = seeded(&work, "demo-api");
        let (b_bare, b_seed) = seeded(&work, "demo-web");

        let mut urls = vec![a_bare.to_string_lossy().to_string()];
        if hub_has_b {
            urls.push(b_bare.to_string_lossy().to_string());
        }
        repo::add(&hub, &urls, None).unwrap();

        Self { _tmp: tmp, hub, a_bare, a_seed, b_bare, b_seed }
    }

    /// 其他人导出的那份配置：两个模块，B 在他那边有地址。
    fn payload(&self, branch: &str) -> SharedRequirement {
        SharedRequirement {
            schema: SHARE_SCHEMA,
            branch: branch.to_string(),
            title: "示例需求v1.3".into(),
            base: vec!["ws:feature-20260901-smart-reach".into()],
            doc_family: Some("示例需求".into()),
            doc_hub: Some("doc-hub".into()),
            notes: "测试环境：dev\n发布地址见自定义信息".into(),
            links: vec![gws_core::workspace::Link::new("需求", "https://tapd.example/1")],
            modules: vec![
                SharedModule {
                    name: "demo-api".into(),
                    url: self.a_bare.to_string_lossy().to_string(),
                },
                SharedModule {
                    name: "demo-web".into(),
                    url: self.b_bare.to_string_lossy().to_string(),
                },
            ],
        }
    }
}

#[test]
fn import_adds_the_missing_repos_and_builds_the_same_requirement() {
    let f = Fixture::new(false); // 本地只有 A，B 要补
    let branch = "feature-20261013-private-agent-v3";
    colleague_branch(&f.a_seed, branch, "a.txt");
    colleague_branch(&f.b_seed, branch, "b.txt");

    assert!(!f.hub.repos_dir().join("demo-web").exists(), "B 本来不在本地");

    let out = share::import(&f.hub, &f.payload(branch), None).unwrap();

    // ① 缺的仓库补上了
    assert_eq!(out.added_repos, vec!["demo-web"], "应当自动补建 B");
    assert!(f.hub.repos_dir().join("demo-web").join(".git").exists());

    // ② 需求建出来了，两个模块都在（跟其他人那边一致）
    assert_eq!(out.modules, vec!["demo-api", "demo-web"]);
    let ws = Workspace::open(&f.hub, branch).unwrap();
    assert_eq!(ws.meta().modules, vec!["demo-api", "demo-web"]);

    // ③ 配置逐项一致 —— 这才是「跟我本地一样」
    assert_eq!(ws.title(), "示例需求v1.3");
    assert_eq!(ws.meta().base, vec!["ws:feature-20260901-smart-reach"], "基线要带过来");
    assert_eq!(ws.meta().doc.as_ref().map(|d| d.family.as_str()), Some("示例需求"));
    assert!(ws.meta().notes.contains("测试环境：dev"));
    assert_eq!(ws.meta().links[0].url, "https://tapd.example/1");

    // ④ 两个模块都真的检出在这条分支上
    for m in ["demo-api", "demo-web"] {
        let wt = f.hub.ws_dir().join(branch).join(m);
        assert_eq!(git(&wt, &["branch", "--show-current"]), branch);
    }
}

#[test]
fn import_works_when_every_repo_is_already_local() {
    // 最常见的另一半场景：同一台机器/其他人本来就有这些仓库 —— 不该多克隆一遍
    let f = Fixture::new(true);
    let branch = "feature-20261013-all-local";
    colleague_branch(&f.a_seed, branch, "a.txt");
    colleague_branch(&f.b_seed, branch, "b.txt");

    let out = share::import(&f.hub, &f.payload(branch), None).unwrap();

    assert!(out.added_repos.is_empty(), "都已有，不该再补：{:?}", out.added_repos);
    assert_eq!(out.modules.len(), 2);
}

#[test]
fn import_refuses_when_a_missing_repo_has_no_url() {
    // 配置里没带地址 → 补不了。必须**当场说清楚是哪个模块**，
    // 而不是建出一个少了模块的需求。
    let f = Fixture::new(false);
    let mut payload = f.payload("feature-20261013-no-url");
    payload.modules[1].url = String::new();

    let err = share::import(&f.hub, &payload, None).unwrap_err().to_string();
    assert!(err.contains("demo-web"), "要点名是哪个模块：{err}");
    assert!(err.contains("重新复制"), "要给出下一步：{err}");
    assert!(!f.hub.ws_dir().join("feature-20261013-no-url").exists(), "不该留下半成品");
}

#[test]
fn import_reports_modules_whose_remote_branch_is_missing() {
    // 配置里写着某个模块，但那条分支在那个仓库上根本没推过（其他人漏推了）。
    // 这不算错误，但要**如实报出来** —— 否则对方以为自己拿到了完整的模块集合。
    let f = Fixture::new(true);
    let branch = "feature-20261013-partial";
    colleague_branch(&f.a_seed, branch, "a.txt");
    // 故意不给 demo-web 推这条分支

    let out = share::import(&f.hub, &f.payload(branch), None).unwrap();

    assert_eq!(out.modules, vec!["demo-api"]);
    assert_eq!(out.not_involved, vec!["demo-web"], "要如实报出没参与的那个");
    // 没参与的模块不该在元数据里登记（登记了界面会显示一个空的模块行）
    let ws = Workspace::open(&f.hub, branch).unwrap();
    assert_eq!(ws.meta().modules, vec!["demo-api"]);
}

#[test]
fn export_then_import_is_the_same_requirement() {
    // 导出 → 渲染成文本 → 对面解析 → 导入：这一整条链路要闭合。
    let f = Fixture::new(true);
    let branch = "feature-20261013-roundtrip";
    colleague_branch(&f.a_seed, branch, "a.txt");
    colleague_branch(&f.b_seed, branch, "b.txt");

    // 先在「我这边」按正常流程建一个需求（用 new，模拟建完再复制配置）
    let opts = lifecycle::NewOptions {
        branch: branch.to_string(),
        title: "示例需求v1.3".into(),
        prefix: "feature".into(),
        base: vec![],
        modules: vec!["demo-api".into(), "demo-web".into()],
        doc_family: Some("示例需求".into()),
        notes: "备注也要一起过去".into(),
        links: vec![gws_core::workspace::Link::new("需求", "https://tapd.example/1")],
    };
    // 分支已在远程 → new 会按设计拒绝（那是「导入」的活），所以这里改用 get
    let got = lifecycle::get(
        &f.hub,
        &lifecycle::GetOptions {
            branch: branch.to_string(),
            title: "示例需求v1.3".into(),
            doc_family: Some("示例需求".into()),
            notes: "备注也要一起过去".into(),
            links: vec![gws_core::workspace::Link::new("需求", "https://tapd.example/1")],
            base: vec!["ws:feature-20260901-smart-reach".into()],
            modules: Some(vec!["demo-api".into(), "demo-web".into()]),
        },
        None,
    )
    .unwrap();
    assert_eq!(got.modules.len(), 2);
    let _ = opts; // 上面只用来说明路径，真正建的是 get

    let ws = Workspace::open(&f.hub, branch).unwrap();
    let exported = share::export(&f.hub, &ws).unwrap();
    let text = exported.render();
    // 挂了文档库就要在配置里写出来 —— 不写的话，人一数就以为"少了一个仓库"
    // （它在 `modules` 之外，用 `doc_hub` 单独表达，见那个字段的注释）
    assert_eq!(exported.doc_hub.as_deref(), Some("doc-hub"));

    // 其他人那边：全新 hub，什么都没有
    let tmp = TempDir::new().unwrap();
    let mut peer = Hub::init(&tmp.path().join("hub"), InitOptions::default()).unwrap();
    // 单测不联网：直接写 GitLab 绑定（`repo::add` 等操作要求 hub 已绑定）。
    // 测试里的仓库都是本地路径，`check_remote` 一律放行，不会真去连这个地址。
    peer.config_mut().gitlab.base_url = "https://gitlab.example.com".into();
    // 提交身份：**别指望机器上的 git 全局配置**（CI runner 上没有，产品又会主动
    // 清掉 local 身份 → `git merge` 直接以 `Committer identity unknown` 失败）。
    peer.config_mut().git = gws_core::hub::GitIdentity {
        author_name: "测试".into(),
        author_email: "test@example.com".into(),
    };
    peer.save_config().unwrap();
    let parsed = share::parse(&text).unwrap();
    assert_eq!(parsed.modules[0].url, f.a_bare.to_string_lossy(), "地址要带上");
    let out = share::import(&peer, &parsed, None).unwrap();

    assert_eq!(out.added_repos.len(), 2, "对面两个仓库都没有，都该补上");
    let peer_ws = Workspace::open(&peer, branch).unwrap();
    assert_eq!(peer_ws.title(), ws.title());
    assert_eq!(peer_ws.meta().base, ws.meta().base);
    assert_eq!(peer_ws.meta().notes, ws.meta().notes);
    assert_eq!(peer_ws.meta().doc, ws.meta().doc);
    assert_eq!(peer_ws.meta().modules, ws.meta().modules);
}

#[test]
fn import_says_the_sender_probably_has_not_pushed() {
    // 实测反馈：导入别人的配置失败，但提示是「确认分支名是否写错」——
    // 那是给"自己敲分支名"写的。配置是别人发来的，最常见的原因是他**还没推**，
    // 提示里要把这件事说出来，并给出下一步。
    let f = Fixture::new(true);
    let payload = f.payload("feature-20261013-never-pushed");
    // 故意谁都不推这条分支

    let err = share::import(&f.hub, &payload, None).unwrap_err().to_string();
    assert!(err.contains("还没把这条分支推到远程"), "要点出真正的原因：{err}");
    assert!(err.contains("推送"), "要给下一步：{err}");
    assert!(!f.hub.ws_dir().join("feature-20261013-never-pushed").exists());
}

#[test]
fn import_refuses_when_the_workspace_already_exists() {
    let f = Fixture::new(true);
    let branch = "feature-20261013-twice";
    colleague_branch(&f.a_seed, branch, "a.txt");

    share::import(&f.hub, &f.payload(branch), None).unwrap();
    let err = share::import(&f.hub, &f.payload(branch), None).unwrap_err().to_string();
    assert!(err.contains("已存在") || err.contains(branch), "重复导入要说清楚：{err}");
}