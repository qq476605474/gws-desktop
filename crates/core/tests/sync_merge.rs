//! 同步与合并的集成测试。
//!
//! 这里是全工具最容易造成损失的部分，也是旧版踩坑最密集的地方。每条测试都
//! 对应设计方案 §8 里的一条经验，或一条「不许静默丢东西」的规则。

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use gws_core::env::EnvDef;
use gws_core::hub::{Hub, InitOptions};
use gws_core::repo;
use gws_core::workspace::lifecycle::{self, NewOptions};
use gws_core::workspace::sync;
use gws_core::workspace::{ModuleFilter, Workspace};

const MODULE: &str = "demo-web";

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(cwd).args(args).output().expect("git 可执行");
    assert!(
        out.status.success(),
        "git {args:?} 失败:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 提交一个文件（每次用固定身份，避免依赖环境里的 git 配置）。
fn commit_file(dir: &Path, file: &str, content: &str, msg: &str) {
    std::fs::write(dir.join(file), content).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", msg]);
}

struct Fixture {
    _tmp: TempDir,
    hub: Hub,
    origin: PathBuf,
    work: PathBuf,
}

impl Fixture {
    /// 建 hub + 一个裸仓库（主干 main，另有 dev 分支）。envs 用默认的 dev。
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let work = tmp.path().to_path_buf();

        let origin = work.join("demo-web.git");
        git(&work, &["init", "--bare", "-b", "main", origin.to_str().unwrap()]);
        let seed = work.join("seed");
        git(&work, &["init", "-b", "main", seed.to_str().unwrap()]);
        commit_file(&seed, "README.md", "# seed\n", "init");
        git(&seed, &["remote", "add", "origin", origin.to_str().unwrap()]);
        git(&seed, &["push", "-q", "origin", "main"]);
        // 环境分支
        git(&origin, &["branch", "dev", "main"]);

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
        repo::add(&hub, &[origin.to_string_lossy().into()], None).unwrap();

        Self { _tmp: tmp, hub, origin, work }
    }

    fn opts(&self, branch: &str) -> NewOptions {
        NewOptions {
            branch: branch.to_string(),
            title: "测试需求".into(),
            prefix: "feature".into(),
            base: vec!["主干".into()],
            modules: vec![MODULE.into()],
            doc_family: None,
            notes: String::new(),
            links: Vec::new(),
        }
    }

    fn create(&self, branch: &str) -> Workspace {
        lifecycle::new(&self.hub, &self.opts(branch)).unwrap();
        Workspace::open(&self.hub, branch).unwrap()
    }

    fn wt(&self, branch: &str) -> PathBuf {
        self.hub.ws_dir().join(branch).join(MODULE)
    }

    fn env_wt(&self, env_id: &str) -> PathBuf {
        self.hub.env_dir(env_id).join(MODULE)
    }

    /// 模拟「其他人往某个分支推了一个提交」。
    fn colleague_pushes(&self, branch: &str, file: &str, content: &str) {
        let dir = self.work.join(format!("colleague-{}", branch.replace('/', "_")));
        let _ = std::fs::remove_dir_all(&dir);
        git(
            &self.work,
            &["clone", "-q", "-b", branch, self.origin.to_str().unwrap(), dir.to_str().unwrap()],
        );
        commit_file(&dir, file, content, "colleague");
        git(&dir, &["push", "-q", "origin", branch]);
    }
}

/// 造一个只有主干、**故意不建 dev 分支**的裸仓库并加入 hub。
///
/// 真实环境里很常见：一个需求横跨多个仓库，但某个仓库并不参与 dev 环境。
fn add_repo_without_dev(f: &Fixture, name: &str) -> PathBuf {
    let bare = f.work.join(format!("{name}.git"));
    git(&f.work, &["init", "--bare", "-b", "main", bare.to_str().unwrap()]);
    let seed = f.work.join(format!("{name}-seed"));
    git(&f.work, &["init", "-b", "main", seed.to_str().unwrap()]);
    commit_file(&seed, "README.md", "# seed\n", "init");
    git(&seed, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(&seed, &["push", "-q", "origin", "main"]);
    repo::add(&f.hub, &[bare.to_string_lossy().into()], None).unwrap();
    bare
}

// ══ push ═══════════════════════════════════════════════════════════════

#[test]
fn push_sends_the_feature_branch_to_origin() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    commit_file(&f.wt("feature-x"), "a.txt", "1", "work");

    let out = sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    assert_eq!(out.pushed, vec![MODULE]);
    assert!(out.is_clean(), "{out:?}");
    assert!(
        git(&f.origin, &["branch", "--list", "feature-x"]).contains("feature-x"),
        "远程应有该分支"
    );
}

#[test]
fn push_refuses_entirely_when_any_module_is_behind_remote() {
    // 他人已推送时直接推会被 git 拒（non-fast-forward）。先整体拦下并提示
    // gws pull，比让 git 抛英文错误直观得多。
    let f = Fixture::new();
    let ws = f.create("feature-x");
    commit_file(&f.wt("feature-x"), "a.txt", "1", "mine");
    sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    f.colleague_pushes("feature-x", "theirs.txt", "2");

    let out = sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();
    assert_eq!(out.behind, vec![(MODULE.to_string(), 1)]);
    assert!(out.pushed.is_empty(), "落后时不应推任何模块");
}

#[test]
fn push_reports_dirty_modules_without_pushing_them() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    std::fs::write(f.wt("feature-x").join("README.md"), "改了点东西").unwrap();

    let out = sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();
    assert_eq!(out.dirty, vec![(MODULE.to_string(), 1)]);
    assert!(out.pushed.is_empty());
    assert!(!out.is_clean());
}

// ══ pull ═══════════════════════════════════════════════════════════════

#[test]
fn pull_brings_in_colleague_commits() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    commit_file(&f.wt("feature-x"), "a.txt", "1", "mine");
    sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    f.colleague_pushes("feature-x", "theirs.txt", "2");

    let out = sync::pull(&f.hub, &ws, &ModuleFilter::all(), false).unwrap();
    assert_eq!(out.updated, vec![(MODULE.to_string(), 1)]);
    assert!(out.conflicts.is_empty());
    assert!(f.wt("feature-x").join("theirs.txt").is_file(), "其他人的文件应已合入");
    assert!(f.wt("feature-x").join("a.txt").is_file(), "自己的文件不该丢");
}

#[test]
fn pull_reports_conflict_and_keeps_the_scene() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    // 先把分支推上去让 origin/feature-x 存在
    sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    // 其他人从我当前的位置分叉出去改同一个文件
    f.colleague_pushes("feature-x", "same.txt", "theirs\n");
    // 我在本地也改同一个文件 —— 两边从同一点分叉，才会真冲突。
    // （若先提交再让其他人在此基础上提交，那只是快进合并，不会冲突。）
    commit_file(&f.wt("feature-x"), "same.txt", "mine\n", "mine");

    let out = sync::pull(&f.hub, &ws, &ModuleFilter::all(), false).unwrap();
    // 断言里带上完整现场：只有「冲突」这一个字段看不出「其实是合并被拒」
    // （实测：CI 的 Linux job 上就是这么红的，光看 `[]` 无从判断）
    assert_eq!(out.conflicts, vec![MODULE], "出问题了：{out:?}");
    // 冲突现场必须保留给人的 IDE 解决，不能被 reset 掉
    let content = std::fs::read_to_string(f.wt("feature-x").join("same.txt")).unwrap();
    assert!(content.contains("<<<<<<<"), "应留下冲突标记: {content}");
}

#[test]
fn pull_skips_modules_whose_branch_is_not_on_remote() {
    let f = Fixture::new();
    let ws = f.create("feature-x"); // 从未 push

    let out = sync::pull(&f.hub, &ws, &ModuleFilter::all(), false).unwrap();
    assert_eq!(out.skipped.len(), 1);
    assert!(out.skipped[0].1.contains("远程无"), "{:?}", out.skipped);
}

// ══ sync-main ══════════════════════════════════════════════════════════

#[test]
fn sync_main_merges_trunk_into_the_requirement() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    f.colleague_pushes("main", "trunk.txt", "from trunk");

    let out = sync::sync_main(&f.hub, &ws, &ModuleFilter::all(), None).unwrap();

    assert!(out.is_clean(), "{out:?}");
    assert!(f.wt("feature-x").join("trunk.txt").is_file(), "主干的新文件应合进来");
}

#[test]
fn sync_main_autostashes_uncommitted_changes() {
    // 有未提交改动时用 --autostash：git 先 stash、合完回填。不加的话 git 会因为
    // 「本地改动会被覆盖」整个拒绝合并，等于把人挡在门外。
    let f = Fixture::new();
    let ws = f.create("feature-x");
    std::fs::write(f.wt("feature-x").join("draft.txt"), "我的草稿").unwrap();
    f.colleague_pushes("main", "trunk.txt", "from trunk");

    let out = sync::sync_main(&f.hub, &ws, &ModuleFilter::all(), None).unwrap();

    assert!(out.is_clean(), "不该因为未提交改动而失败: {out:?}");
    assert!(f.wt("feature-x").join("trunk.txt").is_file(), "主干改动应合进来");
    assert!(f.wt("feature-x").join("draft.txt").is_file(), "未提交的草稿应回填");
}

#[test]
fn sync_main_reports_conflict_without_losing_anything() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    commit_file(&f.wt("feature-x"), "same.txt", "mine\n", "mine");
    f.colleague_pushes("main", "same.txt", "trunk\n");

    let out = sync::sync_main(&f.hub, &ws, &ModuleFilter::all(), None).unwrap();

    assert_eq!(out.conflicts, vec![MODULE], "出问题了：{out:?}");
    let content = std::fs::read_to_string(f.wt("feature-x").join("same.txt")).unwrap();
    assert!(content.contains("<<<<<<<"), "冲突现场要留给人解决");
}

#[test]
fn sync_main_falls_back_to_trunk_for_modules_missing_the_upstream_branch() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    f.colleague_pushes("main", "trunk.txt", "x");

    // 上游工作区不存在 → 应降级主干而不是报错
    let out = sync::sync_main(
        &f.hub,
        &ws,
        &ModuleFilter::all(),
        Some(&["ws:不存在的上游".to_string()]),
    )
    .unwrap();

    assert!(out.is_clean(), "{out:?}");
    assert!(f.wt("feature-x").join("trunk.txt").is_file());
}

#[test]
fn sync_main_uses_the_base_registered_in_metadata() {
    // 需求 #15：基线可随时修改，改完后继同步走新值
    let f = Fixture::new();
    let mut ws = f.create("feature-x");
    ws.meta_mut().base = vec!["ws:不存在的上游".to_string()];
    ws.save_meta().unwrap();
    f.colleague_pushes("main", "trunk.txt", "x");

    let out = sync::sync_main(&f.hub, &ws, &ModuleFilter::all(), None).unwrap();
    assert_eq!(out.from, vec!["ws:不存在的上游".to_string()], "应取元数据里的基线");
    assert!(out.is_clean());
}

// ══ merge ══════════════════════════════════════════════════════════════

#[test]
fn merge_brings_the_requirement_into_the_env_locally_without_pushing() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    commit_file(&f.wt("feature-x"), "feat.txt", "1", "feature work");
    sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    let out = sync::merge(&f.hub, &ws, "dev", &ModuleFilter::all(), false, false).unwrap();

    assert_eq!(out.clean, vec![MODULE]);
    assert!(!out.did_push, "默认只合到本地，绝不自动推");
    assert!(f.env_wt("dev").join("feat.txt").is_file());

    // 远程 dev 分支不该被改动
    let remote_dev = git(&f.origin, &["rev-parse", "dev"]);
    let remote_main = git(&f.origin, &["rev-parse", "main"]);
    assert_eq!(remote_dev, remote_main, "远程 dev 不该被推");
}

#[test]
fn merge_with_push_updates_the_remote_env_branch() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    commit_file(&f.wt("feature-x"), "feat.txt", "1", "feature work");
    sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    let out = sync::merge(&f.hub, &ws, "dev", &ModuleFilter::all(), true, false).unwrap();

    assert!(out.did_push);
    assert_eq!(out.pushed, vec![MODULE]);
    let remote_dev = git(&f.origin, &["rev-parse", "dev"]);
    let remote_main = git(&f.origin, &["rev-parse", "main"]);
    assert_ne!(remote_dev, remote_main, "远程 dev 应已前进");
}

#[test]
fn merge_refuses_when_the_requirement_branch_is_behind_remote() {
    // 前置闸门：其他人已推送新提交时直接合入环境分支
    // 会用旧代码覆盖他的成果。
    let f = Fixture::new();
    let ws = f.create("feature-x");
    commit_file(&f.wt("feature-x"), "a.txt", "1", "mine");
    sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();
    f.colleague_pushes("feature-x", "theirs.txt", "2");

    let err = sync::merge(&f.hub, &ws, "dev", &ModuleFilter::all(), false, false).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("落后于远程"), "{msg}");
    assert!(msg.contains("gws pull"), "应给出正确的下一步: {msg}");
}

#[test]
fn merge_recovers_when_the_env_branch_was_rebuilt_by_force_push() {
    // 远程环境分支被重建（force push）→ 本地副本全部作废，必须重置后重新合并。
    // 而且重置后要继续完成合并，不能只重置——旧版为此专门修过一次。
    let f = Fixture::new();
    let ws = f.create("feature-x");
    commit_file(&f.wt("feature-x"), "feat.txt", "1", "feature work");
    sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    // 先合一次
    sync::merge(&f.hub, &ws, "dev", &ModuleFilter::all(), false, false).unwrap();

    // 模拟远程 dev 被重建：主干前进后再把 dev 指过去。
    // 光指回当前的 main 不够 —— 那样 origin/dev 仍是本地 HEAD 的祖先，
    // 按定义不算"被重建"，会走到正常合并分支上去。
    f.colleague_pushes("main", "trunk.txt", "trunk moved");
    git(&f.origin, &["update-ref", "refs/heads/dev", "refs/heads/main"]);

    let out = sync::merge(&f.hub, &ws, "dev", &ModuleFilter::all(), false, false).unwrap();

    assert_eq!(out.stale, vec![MODULE], "应识别出远程被重建：{out:?}");
    assert_eq!(out.clean, vec![MODULE], "重置后应继续完成合并，而不是等下一次：{out:?}");
    assert!(f.env_wt("dev").join("feat.txt").is_file(), "出问题了：{out:?}");
}

#[test]
fn merge_preserves_unpushed_local_merge_results() {
    // envs 里可能有在 IDE 中解完但还没推的冲突解决成果。默认不推不问，
    // 这些成果必须保住。
    let f = Fixture::new();
    let ws = f.create("feature-x");
    commit_file(&f.wt("feature-x"), "feat.txt", "1", "feature work");
    sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    sync::merge(&f.hub, &ws, "dev", &ModuleFilter::all(), false, false).unwrap();
    // 本地 env 上再打一个未推送的提交，模拟手工收尾
    commit_file(&f.env_wt("dev"), "manual.txt", "手工作品", "manual fix");

    let out = sync::merge(&f.hub, &ws, "dev", &ModuleFilter::all(), false, false).unwrap();

    // 领先 = 首次合并带来的提交 + 手工提交
    assert_eq!(out.pending.len(), 1, "{out:?}");
    assert_eq!(out.pending[0].0, MODULE);
    assert!(out.pending[0].1 > 0);
    assert!(out.clean.is_empty(), "不应重合，否则会丢掉手工作品");
    assert!(f.env_wt("dev").join("manual.txt").is_file(), "手工作品必须还在");
}

#[test]
fn merge_force_discards_unpushed_results_and_remerges() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    commit_file(&f.wt("feature-x"), "feat.txt", "1", "feature work");
    sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    sync::merge(&f.hub, &ws, "dev", &ModuleFilter::all(), false, false).unwrap();
    commit_file(&f.env_wt("dev"), "manual.txt", "手工作品", "manual fix");

    let out = sync::merge(&f.hub, &ws, "dev", &ModuleFilter::all(), false, true).unwrap();

    assert_eq!(out.clean, vec![MODULE]);
    assert!(out.pending.is_empty());
    assert!(!f.env_wt("dev").join("manual.txt").exists(), "--force 应丢弃本地成果");
    assert!(f.env_wt("dev").join("feat.txt").is_file(), "但应重新合入需求内容");
}

#[test]
fn merge_reports_conflict_and_keeps_the_env_scene_for_ide() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    commit_file(&f.wt("feature-x"), "same.txt", "feature\n", "feature");
    sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    // 让 dev 分支上也改同一个文件
    f.colleague_pushes("dev", "same.txt", "dev-side\n");

    let out = sync::merge(&f.hub, &ws, "dev", &ModuleFilter::all(), false, false).unwrap();

    assert!(out.has_conflicts(), "出问题了：{out:?}");
    assert_eq!(out.conflicts, vec![MODULE], "出问题了：{out:?}");
    let dir = out.conflict_dirs.get(MODULE).expect("应给出冲突目录供界面打开");
    let content = std::fs::read_to_string(Path::new(dir).join("same.txt")).unwrap();
    assert!(content.contains("<<<<<<<"), "冲突现场要留给人解决: {content}");
}

#[test]
fn merge_skips_modules_without_the_env_branch() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    // 造一个远程没有的 env 分支名
    git(&f.origin, &["branch", "-D", "dev"]);

    let out = sync::merge(&f.hub, &ws, "dev", &ModuleFilter::all(), false, false).unwrap();
    assert_eq!(out.skipped.len(), 1);
    assert!(out.skipped[0].1.contains("没有 dev 分支"), "{:?}", out.skipped);
}

#[test]
fn merge_respects_env_branch_overrides() {
    // 需求 #13：环境标识与分支名解耦 —— pre 环境在这个仓库上用的是 pre-202609
    let f = Fixture::new();
    git(&f.origin, &["branch", "pre-202609", "main"]);

    let mut hub = f.hub.clone();
    hub.config_mut().envs.push({
        let mut e = EnvDef::new("pre", "预发");
        e.branches.insert(MODULE.into(), "pre-202609".into());
        e
    });
    hub.save_config().unwrap();
    let hub = Hub::open(hub.root()).unwrap();

    let ws = f.create("feature-x");
    commit_file(&f.wt("feature-x"), "feat.txt", "1", "feature work");
    sync::push(&hub, &ws, &ModuleFilter::all()).unwrap();

    let out = sync::merge(&hub, &ws, "pre", &ModuleFilter::all(), false, false).unwrap();

    assert_eq!(out.clean, vec![MODULE]);
    // 目录名是环境标识 pre，实际合入的是 pre-202609 分支
    assert!(hub.env_dir("pre").join(MODULE).join("feat.txt").is_file());
}

// ══ st / done / sync ═══════════════════════════════════════════════════

#[test]
fn status_reports_dirty_ahead_behind_and_missing_modules() {
    let f = Fixture::new();
    let ws = f.create("feature-x");

    // 初始：干净、未推送、领先主干 0
    let rep = sync::status(&f.hub, &ws, &ModuleFilter::all()).unwrap();
    assert_eq!(rep.branch, "feature-x");
    assert_eq!(rep.modules.len(), 1);
    assert_eq!(rep.modules[0].dirty, 0);
    assert!(!rep.modules[0].pushed);
    assert!(rep.modules[0].error.is_none());

    // 提交 + 推送后：干净、已推送、领先主干 1
    commit_file(&f.wt("feature-x"), "a.txt", "1", "work");
    sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();
    let rep = sync::status(&f.hub, &ws, &ModuleFilter::all()).unwrap();
    assert_eq!(rep.modules[0].dirty, 0);
    assert!(rep.modules[0].pushed);
    assert_eq!(rep.modules[0].ahead_of_main, Some(1));

    // 有未提交改动
    std::fs::write(f.wt("feature-x").join("b.txt"), "x").unwrap();
    let rep = sync::status(&f.hub, &ws, &ModuleFilter::all()).unwrap();
    assert_eq!(rep.modules[0].dirty, 1);

    // 目录缺失 → present=false，而不是伪装成干净
    std::fs::remove_dir_all(f.wt("feature-x")).unwrap();
    let mut ws2 = Workspace::open(&f.hub, "feature-x").unwrap();
    ws2.meta_mut().modules = vec![MODULE.into()];
    let rep = sync::status(&f.hub, &ws2, &ModuleFilter::all()).unwrap();
    assert!(!rep.modules[0].present);
}

#[test]
fn done_reports_modules_not_yet_in_trunk() {
    let f = Fixture::new();
    let ws = f.create("feature-x");

    // 刚建出来时没有领先主干的提交 → 视为已并入
    assert!(sync::done(&f.hub, &ws, &ModuleFilter::all(), None).unwrap().is_done());

    commit_file(&f.wt("feature-x"), "a.txt", "1", "work");
    let out = sync::done(&f.hub, &ws, &ModuleFilter::all(), None).unwrap();
    assert_eq!(out.unmerged, vec![(MODULE.to_string(), 1)]);
}

#[test]
fn sync_all_resets_repos_and_envs_to_remote() {
    let f = Fixture::new();
    let ws = f.create("feature-x");
    commit_file(&f.wt("feature-x"), "feat.txt", "1", "work");
    sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();
    sync::merge(&f.hub, &ws, "dev", &ModuleFilter::all(), true, false).unwrap();

    // 把 repos 镜像弄脏，再同步应被重置回主干
    commit_file(&f.hub.repos_dir().join(MODULE), "junk.txt", "x", "junk");

    let out = sync::sync_all(&f.hub, None).unwrap();

    assert!(out.repos_updated.contains(&MODULE.to_string()), "{out:?}");
    assert!(!f.hub.repos_dir().join(MODULE).join("junk.txt").exists(), "repos 应被重置");
    assert!(
        f.env_wt("dev").join("feat.txt").is_file(),
        "envs/dev 应保持远程 dev 的内容"
    );
}

// ══ 模块选择（§6.4）═══════════════════════════════════════════════════

#[test]
fn module_filter_rejects_unknown_module_names() {
    // --only 里写了不存在的模块 → 报错，而不是静默什么都不做
    let f = Fixture::new();
    let ws = f.create("feature-x");

    let filter = ModuleFilter::only(["打错的名字"]);
    let err = sync::push(&f.hub, &ws, &filter).unwrap_err();
    assert!(err.to_string().contains("不在本工作区"), "{err}");
}

#[test]
fn module_filter_excluding_everything_is_a_no_op_not_an_error() {
    let f = Fixture::new();
    let ws = f.create("feature-x");

    let filter = ModuleFilter::excluding([MODULE]);
    let out = sync::push(&f.hub, &ws, &filter).unwrap();
    assert!(out.is_clean(), "{out:?}");
    assert!(out.pushed.is_empty());
}
// ══ sync 补建环境 worktree（旧版 gws sync 的「新增仓库自动补建」）══════════
//
// 回归测试：最初实现里 `sync_all` 对缺失的环境 worktree 直接 `continue`，
// 导致 `gws init --envs dev,pre` + `repo add` + `sync` 之后 envs/ 下什么都没有
// ——整个环境机制形同虚设。端到端演练时才发现。

#[test]
fn sync_creates_missing_env_worktrees() {
    let f = Fixture::new();
    // 新建 hub 时环境目录是空的，worktree 要靠 sync 补建
    assert!(!f.env_wt("dev").exists(), "初始不该有环境 worktree");

    let out = sync::sync_all(&f.hub, None).unwrap();

    assert!(
        out.env_created.iter().any(|(e, m, b)| e == "dev" && m == MODULE && b == "dev"),
        "应补建 envs/dev/{MODULE}: {:?}",
        out.env_created
    );
    assert!(f.env_wt("dev").exists(), "环境 worktree 应已建出");
    assert_eq!(git(&f.env_wt("dev"), &["branch", "--show-current"]), "dev");
}

#[test]
fn sync_creates_env_worktree_on_the_override_branch() {
    // 环境标识 dev 覆盖成 dev-202609 后，补建的 worktree 应检出覆盖后的分支
    let f = Fixture::new();
    git(&f.origin, &["branch", "dev-202609", "main"]);

    let mut hub = f.hub.clone();
    hub.config_mut().env_mut("dev").unwrap().branches.insert(MODULE.into(), "dev-202609".into());
    hub.save_config().unwrap();
    let hub = Hub::open(hub.root()).unwrap();

    let out = sync::sync_all(&hub, None).unwrap();

    assert!(
        out.env_created.iter().any(|(_, _, b)| b == "dev-202609"),
        "应按覆盖后的分支补建: {:?}",
        out.env_created
    );
    // 目录名仍是环境标识 dev，检出的却是 dev-202609
    let wt = hub.env_dir("dev").join(MODULE);
    assert_eq!(git(&wt, &["branch", "--show-current"]), "dev-202609");
}

#[test]
fn sync_is_idempotent_for_env_worktrees() {
    let f = Fixture::new();
    sync::sync_all(&f.hub, None).unwrap();
    let again = sync::sync_all(&f.hub, None).unwrap();

    assert!(again.env_created.is_empty(), "第二次不该重复补建: {:?}", again.env_created);
    assert!(f.env_wt("dev").exists());
}

#[test]
fn sync_resets_dirty_env_worktree_back_to_remote() {
    let f = Fixture::new();
    sync::sync_all(&f.hub, None).unwrap();

    // 把环境 worktree 弄脏（模拟手工在里面改过东西）
    commit_file(&f.env_wt("dev"), "junk.txt", "x", "junk");
    assert!(f.env_wt("dev").join("junk.txt").is_file());

    sync::sync_all(&f.hub, None).unwrap();

    assert!(!f.env_wt("dev").join("junk.txt").exists(), "环境是远程镜像，应被重置");
}

// ══ ensure_env_worktrees：添加仓库 / 新建环境后的自动补齐 ════════════════
//
// 实测反馈：加完仓库去环境页点「文件管理」没反应 —— 环境页列的是配置，
// 目录要手动点「同步此环境」才建。所以加了这两处自动补。
//
// 但**自动补绝不能顺手 reset**：环境里可能躺着还没推送的合并成果。
// 下面第一条钉安全性，第二条钉「它跟 sync_env 真的不一样」。

#[test]
fn ensure_env_worktrees_creates_the_missing_checkout() {
    let f = Fixture::new();
    // 刚加完仓库：repos/ 有了，envs/ 还是空的 —— 正是自动补要处理的场景
    assert!(!f.env_wt("dev").exists(), "初始不该有环境 worktree");

    let out = sync::ensure_env_worktrees(&f.hub, "dev", &ModuleFilter::all(), None).unwrap();

    assert!(
        out.env_created.iter().any(|(e, m, b)| e == "dev" && m == MODULE && b == "dev"),
        "应补建 envs/dev/{MODULE}: {:?}",
        out.env_created
    );
    assert!(f.env_wt("dev").exists());
    assert_eq!(git(&f.env_wt("dev"), &["branch", "--show-current"]), "dev");
    // 只补建，不该报「动了」谁
    assert!(out.env_moved.is_empty(), "只补建时不该有 env_moved: {:?}", out.env_moved);
}

#[test]
fn ensure_env_worktrees_never_resets_an_existing_checkout() {
    let f = Fixture::new();
    sync::ensure_env_worktrees(&f.hub, "dev", &ModuleFilter::all(), None).unwrap();

    // 环境里有一份**还没推送**的合并成果
    commit_file(&f.env_wt("dev"), "merged-result.txt", "还没推", "合进环境的成果");
    let before = git(&f.env_wt("dev"), &["rev-parse", "HEAD"]);

    // 再补一次（比如又加了个仓库、或又新建了别的环境）
    let out = sync::ensure_env_worktrees(&f.hub, "dev", &ModuleFilter::all(), None).unwrap();

    assert!(out.env_created.is_empty(), "已经在了就不该再建: {:?}", out.env_created);
    assert!(out.env_moved.is_empty(), "更不该报「动了它」: {:?}", out.env_moved);
    assert!(
        f.env_wt("dev").join("merged-result.txt").is_file(),
        "自动补齐**抹掉了环境里没推送的成果** —— 这正是不能用 sync_env 的原因"
    );
    assert_eq!(git(&f.env_wt("dev"), &["rev-parse", "HEAD"]), before, "HEAD 不该动");
}

#[test]
fn sync_env_would_have_reset_that_same_checkout() {
    // 上一条测试的另一半：证明两个函数**确实不一样**。
    // 没有这条，`ensure` 存在的理由就只是注释里的一句话。
    let f = Fixture::new();
    sync::sync_env(&f.hub, "dev", &ModuleFilter::all(), None).unwrap();
    commit_file(&f.env_wt("dev"), "merged-result.txt", "还没推", "合进环境的成果");

    sync::sync_env(&f.hub, "dev", &ModuleFilter::all(), None).unwrap();

    assert!(
        !f.env_wt("dev").join("merged-result.txt").exists(),
        "sync_env 是「同步到远程」，会 reset —— 所以自动补齐不能用它"
    );
}

#[test]
fn ensure_env_worktrees_skips_repos_without_the_env_branch() {
    // 不是每个仓库都参与每个环境：没有该环境分支的仓库要跳过，
    // 而不是让整个补齐失败
    let f = Fixture::new();
    // 远程只有 main / dev；新加一个 stag 环境，它没有对应分支
    let mut hub = f.hub.clone();
    hub.config_mut().envs.push(EnvDef::new("stag", "预发"));
    hub.save_config().unwrap();
    let hub = Hub::open(hub.root()).unwrap();

    let out = sync::ensure_env_worktrees(&hub, "stag", &ModuleFilter::all(), None).unwrap();

    assert!(out.env_created.is_empty(), "没有该环境分支就不该建: {:?}", out.env_created);
    assert!(out.env_failures.is_empty(), "跳过不是失败: {:?}", out.env_failures);
}

#[test]
fn ensure_env_worktrees_reports_an_unknown_env_instead_of_guessing() {
    let f = Fixture::new();
    let err = sync::ensure_env_worktrees(&f.hub, "not-an-env", &ModuleFilter::all(), None)
        .unwrap_err();
    assert!(err.to_string().contains("not-an-env"), "{err}");
}

#[test]
fn merge_skips_only_the_modules_missing_the_env_branch() {
    // 真实场景：一个需求横跨多个仓库，但某个仓库没有 dev 分支（不是每个仓库
    // 都参与每个环境）。应跳过它、如实报出，其余照常合。
    let f = Fixture::new();
    add_repo_without_dev(&f, "no-dev-repo");

    let mut opts = f.opts("feature-x");
    opts.modules = vec![MODULE.into(), "no-dev-repo".into()];
    lifecycle::new(&f.hub, &opts).unwrap();
    let ws = Workspace::open(&f.hub, "feature-x").unwrap();

    commit_file(&f.wt("feature-x"), "feat.txt", "1", "work");
    let extra_wt = f.hub.ws_dir().join("feature-x").join("no-dev-repo");
    commit_file(&extra_wt, "other.txt", "1", "work");
    sync::push(&f.hub, &ws, &ModuleFilter::all()).unwrap();

    let out = sync::merge(&f.hub, &ws, "dev", &ModuleFilter::all(), false, false).unwrap();

    assert_eq!(out.clean, vec![MODULE], "有 dev 分支的仓库应正常合入");
    assert_eq!(out.skipped.len(), 1, "{out:?}");
    assert_eq!(out.skipped[0].0, "no-dev-repo");
    assert!(out.skipped[0].1.contains("没有 dev 分支"), "{:?}", out.skipped[0].1);
    // 被跳过的仓库不该被建出环境 worktree
    assert!(!f.hub.env_dir("dev").join("no-dev-repo").exists());
}

// ══ 环境 worktree 的分支校验（实测事故）═════════════════════════════════
//
// 用户选「合并到环境 dev」，东西却落到了 `pre-202610` 分支上 —— 因为
// `envs/dev/<仓库>` 当时检出的不是 dev。根因是这些操作从不校验「实际检出在哪条
// 分支」，往当前检出的分支上合、却按配置里该有的分支上报。
// 分支标签一旦和历史对不上（改过覆盖表、手工动过目录），就会**默默合错分支**。

#[test]
fn merge_refuses_when_the_env_worktree_is_on_the_wrong_branch() {
    let f = Fixture::new();
    let ws = f.create("feature-20261014-wrong-branch");
    sync::sync_all(&f.hub, None).unwrap();

    // 把环境里那条 worktree 切到别的分支上（模拟历史遗留的漂移）
    let env_wt = f.env_wt("dev");
    git(&f.origin, &["branch", "pre-202610", "main"]);
    git(&env_wt, &["fetch", "origin"]);
    git(&env_wt, &["switch", "--quiet", "-c", "pre-202610", "origin/pre-202610"]);

    let err = sync::merge(&f.hub, &ws, "dev", &ModuleFilter::all(), false, false)
        .expect_err("检出分支不对时必须拒绝，而不是合到那条错的分支上");

    let msg = err.to_string();
    assert!(msg.contains("实际检出的分支"), "要说清为什么拒绝: {msg}");
    assert!(msg.contains("pre-202610") && msg.contains("dev"), "两个分支都要点出来: {msg}");
    // 拒绝就要拒绝干净：那条错的分支不该被动过
    assert_eq!(
        git(&env_wt, &["rev-parse", "--short", "pre-202610"]),
        git(&f.origin, &["rev-parse", "--short", "pre-202610"]),
        "被拒时那条分支不该被改动"
    );
}

#[test]
fn sync_switches_the_env_worktree_back_to_its_own_branch() {
    // 「同步此环境」应当把漂移纠正回来 —— 而不是把错分支 reset 到该有的提交上
    // （那会把错的那条分支指针也挪走，是事故的另一半）
    let f = Fixture::new();
    f.create("feature-20261014-switch-back");
    sync::sync_all(&f.hub, None).unwrap();

    let env_wt = f.env_wt("dev");
    git(&f.origin, &["branch", "pre-202610", "main"]);
    git(&env_wt, &["fetch", "origin"]);
    git(&env_wt, &["switch", "--quiet", "-c", "pre-202610", "origin/pre-202610"]);

    sync::sync_env(&f.hub, "dev", &ModuleFilter::all(), None).unwrap();

    assert_eq!(git(&env_wt, &["branch", "--show-current"]), "dev", "应当切回 dev");
    assert_eq!(
        git(&env_wt, &["rev-parse", "--short", "pre-202610"]),
        git(&f.origin, &["rev-parse", "--short", "pre-202610"]),
        "那条错的分支不该被顺手挪走"
    );
}
