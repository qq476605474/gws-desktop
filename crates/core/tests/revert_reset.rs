//! 撤销改动 / 强制回退的集成测试。
//!
//! 这两个操作**会丢东西**，所以重点验证三件事：
//! 1. 预览说的量和实际会丢的一致（预览骗人比不预览更糟）
//! 2. 执行后真的达到预期状态（不是「报成功但没做」）
//! 3. 不该丢的东西没丢（node_modules / 未涉及模块 / 未跟踪文件不该被静默删）

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use gws_core::hub::{Hub, InitOptions};
use gws_core::repo;
use gws_core::workspace::lifecycle::{self, NewOptions};
use gws_core::workspace::revert;
use gws_core::workspace::{ModuleFilter, Workspace};

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(cwd).args(args).output().expect("git");
    assert!(out.status.success(), "git {args:?} 失败:\n{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn seeded_origin(root: &Path, name: &str) -> PathBuf {
    let bare = root.join(format!("{name}.git"));
    git(root, &["init", "--bare", "-b", "main", bare.to_str().unwrap()]);
    let seed = root.join(format!("{name}-seed"));
    git(root, &["init", "-b", "main", seed.to_str().unwrap()]);
    std::fs::write(seed.join("README.md"), "# seed\n").unwrap();
    git(&seed, &["add", "-A"]);
    git(&seed, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "init"]);
    git(&seed, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&seed, &["push", "-q", "origin", "main"]);
    bare
}

struct Fixture {
    _tmp: TempDir,
    hub: Hub,
    origin: PathBuf,
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
        let origin = seeded_origin(tmp.path(), "repo-a");
        repo::add(&hub, &[origin.to_string_lossy().into()], None).unwrap();
        Self { _tmp: tmp, hub, origin }
    }

    fn create(&self, branch: &str) -> Workspace {
        lifecycle::new(
            &self.hub,
            &NewOptions {
                branch: branch.to_string(),
                title: "测试".into(),
                prefix: "feature".into(),
                base: vec!["主干".into()],
                modules: vec!["repo-a".into()],
                doc_family: None,
                notes: String::new(),
                links: Vec::new(),
            },
        )
        .unwrap();
        Workspace::open(&self.hub, branch).unwrap()
    }

    fn wt(&self, branch: &str) -> PathBuf {
        self.hub.ws_dir().join(branch).join("repo-a")
    }
}

// ══ 撤销改动（需求 #3）══════════════════════════════════════════════════

#[test]
fn revert_plan_counts_modified_and_untracked_separately() {
    // 未跟踪文件是最危险的：新写的代码没 git add 的话，撤销就是永久删除
    let f = Fixture::new();
    let ws = f.create("feature-x");
    let wt = f.wt("feature-x");

    std::fs::write(wt.join("README.md"), "改了已有文件").unwrap();
    std::fs::write(wt.join("brand-new.txt"), "新文件，没 add 过").unwrap();

    let plan = revert::revert_plan(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    assert_eq!(plan.modules.len(), 1);
    let m = &plan.modules[0];
    assert_eq!(m.modified, vec!["README.md"]);
    assert_eq!(m.untracked, vec!["brand-new.txt"]);
    assert_eq!(plan.total_files(), 2);
}

#[test]
fn revert_apply_restores_files_and_removes_untracked() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    let wt = f.wt("feature-x");

    std::fs::write(wt.join("README.md"), "改了").unwrap();
    std::fs::write(wt.join("brand-new.txt"), "新的").unwrap();

    let out = revert::revert_apply(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    assert_eq!(out.reverted.len(), 1, "{out:?}");
    assert!(out.failed.is_empty(), "{out:?}");
    // 换行归一化再比：Windows 上 git 默认 `core.autocrlf=true`，checkout 出来是 CRLF
    assert_eq!(
        std::fs::read_to_string(wt.join("README.md"))
            .unwrap()
            .replace("\r\n", "\n"),
        "# seed\n",
        "已跟踪文件应还原"
    );
    assert!(!wt.join("brand-new.txt").exists(), "未跟踪文件应被清掉");
    assert_eq!(git(&wt, &["status", "--porcelain"]), "", "撤销后应完全干净");
}

#[test]
fn revert_keeps_heavy_build_dirs() {
    // 清掉 node_modules / target 会让下次构建非常慢，而这跟「撤销改动」无关
    let f = Fixture::new();
    let ws = f.create("feature-x");
    let wt = f.wt("feature-x");

    std::fs::create_dir_all(wt.join("node_modules/pkg")).unwrap();
    std::fs::write(wt.join("node_modules/pkg/index.js"), "x").unwrap();
    std::fs::create_dir_all(wt.join("target/debug")).unwrap();
    std::fs::write(wt.join("target/debug/bin"), "x").unwrap();
    std::fs::write(wt.join("stray.txt"), "真该被清掉的").unwrap();

    revert::revert_apply(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    assert!(wt.join("node_modules/pkg/index.js").is_file(), "node_modules 应保留");
    assert!(wt.join("target/debug/bin").is_file(), "target 应保留");
    assert!(!wt.join("stray.txt").exists(), "其它未跟踪文件应清掉");
}

#[test]
fn revert_leaves_unselected_modules_alone() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    let wt = f.wt("feature-x");
    std::fs::write(wt.join("keep.txt"), "别动我").unwrap();

    // 只选一个不存在的模块名会被拦下；这里用一个空过滤（没有匹配的模块）
    let filter = ModuleFilter::excluding(["repo-a"]);
    let plan = revert::revert_plan(&f.hub, &ws, &filter).unwrap();
    assert!(plan.is_empty(), "排除掉的模块不该出现在预览里");
    assert!(wt.join("keep.txt").is_file());
}

#[test]
fn revert_plan_is_empty_when_nothing_to_do() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    let plan = revert::revert_plan(&f.hub, &ws, &ModuleFilter::all()).unwrap();
    assert!(plan.is_empty());
    assert_eq!(revert::revert_lines(&plan), vec!["没有未提交的改动".to_string()]);
}

// ══ 强制回退到远程（需求 #10）══════════════════════════════════════════

#[test]
fn reset_plan_reports_discarded_commits_and_dirty_files() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    let wt = f.wt("feature-x");

    // 推一次，让远程存在该分支；然后再本地提交 2 个
    git(&wt, &["push", "-q", "-u", "origin", "feature-x"]);
    for i in 0..2 {
        std::fs::write(wt.join(format!("c{i}.txt")), "x").unwrap();
        git(&wt, &["add", "-A"]);
        git(&wt, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", &format!("c{i}")]);
    }
    std::fs::write(wt.join("README.md"), "还有未提交的改动").unwrap();

    let plan = revert::reset_plan(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    assert_eq!(plan.modules.len(), 1);
    let m = &plan.modules[0];
    assert_eq!(m.local_commits, 2, "应报出 2 个会被丢弃的本地提交");
    assert_eq!(m.dirty_files, 1, "应报出 1 处未提交改动");
    assert!(m.ahead);
    assert_eq!(plan.total_local_commits(), 2);
}

#[test]
fn reset_apply_discards_local_commits_and_matches_remote() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    let wt = f.wt("feature-x");

    git(&wt, &["push", "-q", "-u", "origin", "feature-x"]);
    let pushed = git(&wt, &["rev-parse", "HEAD"]);
    for i in 0..2 {
        std::fs::write(wt.join(format!("c{i}.txt")), "x").unwrap();
        git(&wt, &["add", "-A"]);
        git(&wt, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", &format!("c{i}")]);
    }

    let out = revert::reset_apply(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    assert_eq!(out.reset.len(), 1, "{out:?}");
    assert!(out.failed.is_empty(), "{out:?}");
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), pushed, "HEAD 应回到远程的样子");
    assert!(!wt.join("c0.txt").exists(), "本地提交引入的文件应消失");
}

#[test]
fn reset_skips_modules_without_a_remote_branch() {
    // 从没推送过的需求：远程没有该分支，回退无从谈起，应跳过而不是报错
    let f = Fixture::new();
    let ws = f.create("feature-never-pushed");
    let plan = revert::reset_plan(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    assert!(plan.modules.is_empty());
    assert_eq!(plan.skipped.len(), 1);
    assert!(plan.skipped[0].1.contains("远程没有"), "{:?}", plan.skipped);
}

#[test]
fn reset_pulls_the_remote_forward_when_remote_is_ahead() {
    // 需求 #10 的主场景：别人 force push / 推进了远程，本地落后 → 一键跟上
    let f = Fixture::new();
    let ws = f.create("feature-x");
    let wt = f.wt("feature-x");
    git(&wt, &["push", "-q", "-u", "origin", "feature-x"]);

    // 模拟其他人往前推了一步（直接改裸仓库的该分支）
    let other = f._tmp.path().join("other");
    git(f._tmp.path(), &["clone", "-q", "-b", "feature-x", f.origin.to_str().unwrap(), other.to_str().unwrap()]);
    std::fs::write(other.join("theirs.txt"), "其他人的文件").unwrap();
    git(&other, &["add", "-A"]);
    git(&other, &["-c", "user.email=c@c", "-c", "user.name=c", "commit", "-q", "-m", "colleague"]);
    git(&other, &["push", "-q", "origin", "feature-x"]);

    let out = revert::reset_apply(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    assert_eq!(out.reset.len(), 1, "{out:?}");
    assert!(wt.join("theirs.txt").is_file(), "应跟上远程，拿到其他人的文件");
}

#[test]
fn reset_aborts_a_merge_in_progress() {
    // 合并中途带着 MERGE_HEAD 时直接 reset 会留下让人困惑的状态，应先 abort
    let f = Fixture::new();
    let ws = f.create("feature-x");
    let wt = f.wt("feature-x");
    git(&wt, &["push", "-q", "-u", "origin", "feature-x"]);

    // 制造一个合并冲突现场
    f.colleague_push_main("clash.txt", "from main\n");
    std::fs::write(wt.join("clash.txt"), "from branch\n").unwrap();
    git(&wt, &["add", "-A"]);
    git(&wt, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "mine"]);
    git(&wt, &["push", "-q", "origin", "feature-x"]);
    let _ = Command::new("git").current_dir(&wt).args(["merge", "origin/main"]).output();

    let out = revert::reset_apply(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    assert_eq!(out.reset.len(), 1, "{out:?}");
    assert!(
        !wt.join(".git").is_dir() || git(&wt, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_empty(),
        "MERGE_HEAD 应已被清理"
    );
}

impl Fixture {
    /// 让主干前进一个提交（模拟别人合了东西到主干）。
    fn colleague_push_main(&self, file: &str, content: &str) {
        let other = self._tmp.path().join("colleague-main");
        let _ = std::fs::remove_dir_all(&other);
        git(
            self._tmp.path(),
            &["clone", "-q", "-b", "main", self.origin.to_str().unwrap(), other.to_str().unwrap()],
        );
        std::fs::write(other.join(file), content).unwrap();
        git(&other, &["add", "-A"]);
        git(&other, &["-c", "user.email=c@c", "-c", "user.name=c", "commit", "-q", "-m", "c"]);
        git(&other, &["push", "-q", "origin", "main"]);
    }
}