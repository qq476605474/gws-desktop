//! 工作区生命周期的集成测试。
//!
//! 全程使用**真实的 git 裸仓库与 worktree**，不 mock。这里验证的每一条都对应
//! 设计方案 §8 里旧版踩过的坑，或本次要修的 bug。

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use gws_core::hub::{Hub, InitOptions};
use gws_core::repo::{self, Outcome};
use gws_core::workspace::lifecycle::{self, NewOptions};
use gws_core::workspace::{self, Workspace, META_FILE};

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

/// 执行 git 并返回是否成功（用于探测类断言）。
fn git_ok(cwd: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn seeded_origin(root: &Path, name: &str, branch: &str) -> PathBuf {
    let bare = root.join(format!("{name}.git"));
    git(
        root,
        &["init", "--bare", "-b", branch, bare.to_str().unwrap()],
    );

    let seed = root.join(format!("{name}-seed"));
    git(root, &["init", "-b", branch, seed.to_str().unwrap()]);
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
    git(&seed, &["push", "-q", "origin", branch]);
    bare
}

/// 在裸仓库里直接造一个分支（模拟「他人已推送同名分支」）。
fn create_remote_branch(bare: &Path, branch: &str, from: &str) {
    git(bare, &["branch", branch, from]);
}

struct Fixture {
    _tmp: TempDir,
    hub: Hub,
    a: PathBuf,
    /// 第二个 origin（主干名为 master）。当前测试只经 `hub` 使用它，
    /// 保留字段是为了后续要造远程分支时方便。
    _b: PathBuf,
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
        // 一个 main 主干、一个 master 主干——真实仓库里两者混用
        let a = seeded_origin(tmp.path(), "demo-web", "main");
        let b = seeded_origin(tmp.path(), "demo-api", "master");
        repo::add(
            &hub,
            &[a.to_string_lossy().into(), b.to_string_lossy().into()],
            None,
        )
        .unwrap();
        Self {
            _tmp: tmp,
            hub,
            a,
            _b: b,
        }
    }

    fn opts(&self, branch: &str, modules: &[&str]) -> NewOptions {
        NewOptions {
            branch: branch.to_string(),
            title: "示例需求".to_string(),
            prefix: "feature".to_string(),
            base: vec!["主干".to_string()],
            modules: modules.iter().map(|s| s.to_string()).collect(),
            doc_family: None,
            notes: String::new(),
            links: Vec::new(),
        }
    }
}

#[test]
fn new_creates_a_worktree_per_module_on_the_same_branch() {
    let f = Fixture::new();
    let opts = f.opts(
        "feature-20261010-demo",
        &["demo-web", "demo-api"],
    );
    let out = lifecycle::new(&f.hub, &opts).unwrap();

    assert_eq!(out.created, vec!["demo-web", "demo-api"]);

    for m in ["demo-web", "demo-api"] {
        let wt = f.hub.ws_dir().join("feature-20261010-demo").join(m);
        assert!(wt.join("README.md").is_file(), "{m} 应有检出内容");
        // 每个模块都固定在同一个需求分支上
        assert_eq!(
            git(&wt, &["branch", "--show-current"]),
            "feature-20261010-demo"
        );
    }
}

#[test]
fn new_writes_metadata_only() {
    // **需求目录下不再生成 CLAUDE.md**：约束与说明统一放在 hub 根那一份
    //（内容对每个需求都一样，逐份生成只是复制同一段话 N 遍）。
    let f = Fixture::new();
    let opts = f.opts("feature-20261010-demo", &["demo-web"]);
    lifecycle::new(&f.hub, &opts).unwrap();

    let ws = Workspace::open(&f.hub, "feature-20261010-demo").unwrap();
    assert_eq!(ws.branch(), "feature-20261010-demo");
    assert_eq!(ws.title(), "示例需求");
    assert!(ws.meta().base.is_empty(), "「主干」不该被存成字面值");
    assert!(!ws.claude_path().exists(), "需求目录下不该有 CLAUDE.md");

    // 而 hub 根那一份必须有，且带上提交约定（那是硬约束，不能丢）
    let hub_md = std::fs::read_to_string(f.hub.root().join("CLAUDE.md")).unwrap();
    let agents_md = std::fs::read_to_string(f.hub.root().join("AGENTS.md")).unwrap();
    assert!(
        agents_md.contains("只能 `git add`"),
        "hub 的 AGENTS.md 要带上提交约定"
    );
    // AGENTS.md 是全文（Codex / Cursor 等也读它）；CLAUDE.md 只是指向它的指针
    assert!(
        hub_md.contains("请先阅读同目录的 AGENTS.md"),
        "CLAUDE.md 应只是指针：{hub_md}"
    );
    assert!(agents_md.contains("ws/<分支名>/"), "要讲清目录语义");
    assert!(
        !hub_md.contains("只改 `ws/<当前分支>/`"),
        "指针里不该再复制全文"
    );
}

#[test]
fn hub_claude_md_allows_fixing_a_bad_merge_in_the_env() {
    // 实测反馈：环境默认「完全不让 AI 动」，但**合错代码**时需要在环境分支上
    // 就地改并提交 —— 所以开了这一个例外。
    //
    // 这里钉住的是**例外必须带着边界**：一段只说「环境也能改」的文案，
    // 会被读成「环境随便改」，那正是这条规则原本要防的事。
    let f = Fixture::new();
    let md = std::fs::read_to_string(f.hub.root().join("AGENTS.md")).unwrap();

    assert!(md.contains("合错代码"), "要说明这个例外的场景：{md}");
    // 允许：在环境分支上改 + 提交
    assert!(md.contains("环境分支"), "要讲清是在环境分支上改");
    // 边界：只处理合错本身，不是需求开发
    assert!(
        md.contains("不是需求开发"),
        "必须写死边界，否则会被读成「环境随便改」"
    );
    // 推送仍归人 —— 推上去就等于改了测试环境
    assert!(md.contains("推送由人执行"), "推送仍要人来做");
    // 而「只改 ws/」这条主规则不能被这次调整冲掉
    assert!(md.contains("只改 `ws/<当前分支>/`"), "主规则要还在");
}

#[test]
fn hub_claude_md_says_env_is_read_only_by_default() {
    // 例外不能把默认值改掉：默认仍是只读，例外的措辞必须显式说出「临时」
    let f = Fixture::new();
    let md = std::fs::read_to_string(f.hub.root().join("AGENTS.md")).unwrap();
    assert!(md.contains("默认只读"), "默认仍是只读：{md}");
    assert!(md.contains("临时"), "例外要限定为「临时」");
}

#[test]
fn new_unsets_upstream_so_ide_push_does_not_target_trunk() {
    // 旧版踩过的坑：从 origin/<主干> 建分支时 git 会把上游设成主干
    // 结果 IDE 里点 Push 直接推向 origin/main。必须解除。
    let f = Fixture::new();
    let opts = f.opts("feature-20261010-demo", &["demo-web"]);
    lifecycle::new(&f.hub, &opts).unwrap();

    let wt = f
        .hub
        .ws_dir()
        .join("feature-20261010-demo")
        .join("demo-web");
    assert!(
        !git_ok(
            &wt,
            &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]
        ),
        "不应有上游跟踪（否则 IDE 的 Push 会推向主干）"
    );
}

#[test]
fn new_precheck_rejects_missing_repo_without_creating_anything() {
    let f = Fixture::new();
    let opts = f.opts("feature-x", &["demo-web", "ghost-repo"]);
    let err = lifecycle::new(&f.hub, &opts).unwrap_err();

    assert!(err.to_string().contains("ghost-repo"), "{err}");
    // 全有或全无：一个模块不过，什么都不该建
    assert!(
        !f.hub.ws_dir().join("feature-x").exists(),
        "不应留下半成品工作区"
    );
}

#[test]
fn new_precheck_rejects_branch_already_on_remote() {
    // 同名分支已在远程 → 多半是别人的需求，应该用 gws get
    let f = Fixture::new();
    create_remote_branch(&f.a, "feature-20261010-demo", "main");

    let opts = f.opts("feature-20261010-demo", &["demo-web"]);
    let err = lifecycle::new(&f.hub, &opts).unwrap_err();

    let msg = err.to_string();
    assert!(msg.contains("已存在于远程"), "{msg}");
    assert!(msg.contains("gws get"), "应给出正确的下一步建议: {msg}");
    assert!(!f.hub.ws_dir().join("feature-20261010-demo").exists());
}

#[test]
fn new_precheck_rejects_branch_already_local() {
    let f = Fixture::new();
    git(
        &f.hub.repos_dir().join("demo-web"),
        &["branch", "feature-x", "main"],
    );

    let opts = f.opts("feature-x", &["demo-web"]);
    let err = lifecycle::new(&f.hub, &opts).unwrap_err();
    assert!(err.to_string().contains("已存在于本地"), "{err}");
}

#[test]
fn new_rolls_back_completely_when_a_later_module_fails() {
    // 第一个模块能建、第二个失败 → 必须整体回滚，不留半残工作区
    let f = Fixture::new();
    // 给第二个仓库预先造一个本地同名分支，让它在创建阶段失败
    // （预检会拦住它，所以这里绕过预检直接构造：先把分支建在预检之后）
    git(
        &f.hub.repos_dir().join("demo-api"),
        &["branch", "feature-x", "master"],
    );

    let opts = f.opts("feature-x", &["demo-web", "demo-api"]);
    assert!(lifecycle::new(&f.hub, &opts).is_err());

    assert!(!f.hub.ws_dir().join("feature-x").exists());
    // 第一个模块的分支也必须被回滚掉，否则下次建同名需求会撞「分支已存在于本地」
    assert!(
        !git_ok(
            &f.hub.repos_dir().join("demo-web"),
            &["show-ref", "--verify", "-q", "refs/heads/feature-x"]
        ),
        "回滚应删掉已创建的分支"
    );
    // 不能留下悬空 worktree 元数据
    let list = git(
        &f.hub.repos_dir().join("demo-web"),
        &["worktree", "list"],
    );
    assert_eq!(list.lines().count(), 1, "只应剩主仓库本身:\n{list}");
}

#[test]
fn new_mixed_main_and_master_trunks_both_work() {
    // 一批仓库里 main/master 混用，写死任何一个都会在另一批上失败
    let f = Fixture::new();
    let opts = f.opts("feature-mixed", &["demo-web", "demo-api"]);
    lifecycle::new(&f.hub, &opts).unwrap();

    for m in ["demo-web", "demo-api"] {
        let wt = f.hub.ws_dir().join("feature-mixed").join(m);
        assert_eq!(git(&wt, &["branch", "--show-current"]), "feature-mixed");
    }
    let ws = Workspace::open(&f.hub, "feature-mixed").unwrap();
    assert_eq!(ws.meta().base_resolved["demo-web"], "origin/main");
    assert_eq!(ws.meta().base_resolved["demo-api"], "origin/master");
}

#[test]
fn base_chain_resolves_upstream_workspace_branch() {
    // 阶段依赖：V2 基于 V1（V1 是另一个工作区）
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-v1", &["demo-web"])).unwrap();

    // 把 V1 的分支推到远程 —— 基线一律从远程分支取
    let v1_wt = f.hub.ws_dir().join("feature-v1").join("demo-web");
    git(&v1_wt, &["push", "-q", "-u", "origin", "feature-v1"]);

    let mut opts = f.opts("feature-v2", &["demo-web"]);
    opts.base = vec!["ws:feature-v1".to_string()];
    let out = lifecycle::new(&f.hub, &opts).unwrap();

    assert_eq!(out.base_resolved["demo-web"], "origin/feature-v1");
    let v2_wt = f.hub.ws_dir().join("feature-v2").join("demo-web");
    assert!(v2_wt.join("README.md").is_file());
}

#[test]
fn base_chain_falls_back_to_trunk_when_upstream_lacks_the_branch() {
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-v1", &["demo-web"])).unwrap();

    // 基线一律从远程分支取，所以 V1 的分支必须先推上去
    let v1_wt = f.hub.ws_dir().join("feature-v1").join("demo-web");
    git(&v1_wt, &["push", "-q", "-u", "origin", "feature-v1"]);

    // V2 引入了一个 V1 没参与的模块 → 该模块应降级主干，而不是整体失败
    let mut opts = f.opts("feature-v2", &["demo-web", "demo-api"]);
    opts.base = vec!["ws:feature-v1".to_string()];
    let out = lifecycle::new(&f.hub, &opts).unwrap();

    assert_eq!(out.base_resolved["demo-web"], "origin/feature-v1");
    assert_eq!(
        out.base_resolved["demo-api"], "origin/master",
        "应降级主干"
    );
    assert!(out.base_notes["demo-api"].contains("降级主干"));
}

#[test]
fn new_rejects_duplicate_and_illegal_branch_names() {
    let f = Fixture::new();
    assert!(lifecycle::new(&f.hub, &f.opts("a/b", &["demo-web"])).is_err());
    assert!(lifecycle::new(&f.hub, &f.opts("..", &["demo-web"])).is_err());
}

#[test]
fn add_modules_attaches_later_without_disturbing_existing_ones() {
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-x", &["demo-web"])).unwrap();

    let mut ws = Workspace::open(&f.hub, "feature-x").unwrap();
    let results = lifecycle::add_modules(&f.hub, &mut ws, &["demo-api".to_string()]).unwrap();

    assert_eq!(results[0].outcome, Outcome::Ok);
    let wt = f.hub.ws_dir().join("feature-x").join("demo-api");
    assert_eq!(git(&wt, &["branch", "--show-current"]), "feature-x");

    let ws = Workspace::open(&f.hub, "feature-x").unwrap();
    // 模块登记是**按名字排序**的，不是加入顺序 —— 这里先建 demo-web 再加 demo-api，
    // 最终列表里 demo-api 在前
    assert_eq!(ws.meta().modules, vec!["demo-api", "demo-web"]);
}

#[test]
fn add_modules_skips_ones_already_present() {
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-x", &["demo-web"])).unwrap();

    let mut ws = Workspace::open(&f.hub, "feature-x").unwrap();
    let results =
        lifecycle::add_modules(&f.hub, &mut ws, &["demo-web".to_string()]).unwrap();
    assert_eq!(results[0].outcome, Outcome::Skipped("已在工作区中".into()));
}

#[test]
fn drop_module_removes_worktree_and_branch() {
    let f = Fixture::new();
    lifecycle::new(
        &f.hub,
        &f.opts("feature-x", &["demo-web", "demo-api"]),
    )
    .unwrap();

    let mut ws = Workspace::open(&f.hub, "feature-x").unwrap();
    let outcome = lifecycle::drop_module(&f.hub, &mut ws, "demo-api", false).unwrap();

    assert_eq!(outcome, Outcome::Ok);
    assert!(!f
        .hub
        .ws_dir()
        .join("feature-x")
        .join("demo-api")
        .exists());
    assert!(!git_ok(
        &f.hub.repos_dir().join("demo-api"),
        &["show-ref", "--verify", "-q", "refs/heads/feature-x"]
    ));

    let ws = Workspace::open(&f.hub, "feature-x").unwrap();
    assert_eq!(ws.meta().modules, vec!["demo-web"]);
}

#[test]
fn drop_module_refuses_when_there_are_uncommitted_changes() {
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-x", &["demo-web"])).unwrap();

    let wt = f.hub.ws_dir().join("feature-x").join("demo-web");
    std::fs::write(wt.join("README.md"), "改了点东西").unwrap();

    let mut ws = Workspace::open(&f.hub, "feature-x").unwrap();
    let outcome = lifecycle::drop_module(&f.hub, &mut ws, "demo-web", false).unwrap();
    assert!(outcome.is_failed(), "有未提交改动时应拒绝");
    assert!(wt.exists());
}

#[test]
fn drop_missing_modules_cleans_up_after_manual_rm() {
    let f = Fixture::new();
    lifecycle::new(
        &f.hub,
        &f.opts("feature-x", &["demo-web", "demo-api"]),
    )
    .unwrap();

    // 用户直接 rm -rf 掉了第二个模块目录（旧版需要 gws drop 清理这种残留）
    std::fs::remove_dir_all(f.hub.ws_dir().join("feature-x").join("demo-api")).unwrap();

    let mut ws = Workspace::open(&f.hub, "feature-x").unwrap();
    let outcome = lifecycle::drop_missing_modules(&f.hub, &mut ws).unwrap();

    assert_eq!(outcome.cleaned, vec!["demo-api"]);
    assert!(outcome.kept.is_empty());
    let ws = Workspace::open(&f.hub, "feature-x").unwrap();
    assert_eq!(ws.meta().modules, vec!["demo-web"]);
}

#[test]
fn drop_missing_modules_keeps_branch_with_unpushed_commits() {
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-x", &["demo-web"])).unwrap();

    let wt = f.hub.ws_dir().join("feature-x").join("demo-web");
    std::fs::write(wt.join("new.txt"), "x").unwrap();
    git(&wt, &["add", "-A"]);
    git(
        &wt,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "work",
        ],
    );

    // 分支从未推送且领先主干 → 删目录后必须保留分支，否则丢代码
    std::fs::remove_dir_all(&wt).unwrap();

    let mut ws = Workspace::open(&f.hub, "feature-x").unwrap();
    let outcome = lifecycle::drop_missing_modules(&f.hub, &mut ws).unwrap();

    assert!(
        outcome.cleaned.is_empty(),
        "不该清理，会丢提交: {outcome:?}"
    );
    assert_eq!(outcome.kept.len(), 1);
    assert!(outcome.kept[0].1.contains("未推送"), "{:?}", outcome.kept);
    assert!(git_ok(
        &f.hub.repos_dir().join("demo-web"),
        &["show-ref", "--verify", "-q", "refs/heads/feature-x"]
    ));
}

#[test]
fn remove_refuses_when_there_is_unsaved_work() {
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-x", &["demo-web"])).unwrap();

    // 有提交但没推送、也没进主干 → 属于风险项
    let wt = f.hub.ws_dir().join("feature-x").join("demo-web");
    std::fs::write(wt.join("new.txt"), "x").unwrap();
    git(&wt, &["add", "-A"]);
    git(
        &wt,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "本地的活",
        ],
    );

    let outcome = lifecycle::remove(&f.hub, "feature-x", false).unwrap();
    assert!(!outcome.risks.is_empty());
    assert!(
        outcome.risks[0].reason.contains("未推送也未并入主干"),
        "原因要说清是提交会丢: {:?}",
        outcome.risks
    );
    assert!(
        f.hub.ws_dir().join("feature-x").exists(),
        "有风险时不应删除"
    );
}

/// 合完主干、远程源分支被删后再删需求，不该再弹「未保存」（实测反馈）。
///
/// 本地分支的提交已经全部在主干里，删掉不会丢任何东西；原来的逻辑看到
/// `origin/<分支>` 不存在就一律报「从未推送到远程」。
#[test]
fn remove_after_merge_to_trunk_does_not_warn() {
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-x", &["demo-web"])).unwrap();
    let wt = f.hub.ws_dir().join("feature-x").join("demo-web");
    std::fs::write(wt.join("new.txt"), "x").unwrap();
    git(&wt, &["add", "-A"]);
    git(
        &wt,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "work",
        ],
    );
    git(&wt, &["push", "-q", "-u", "origin", "feature-x"]);

    // 模拟 MR 合并：主干指到需求分支，源分支在远程被删，本地 prune 掉引用
    git(&f.a, &["update-ref", "refs/heads/main", "refs/heads/feature-x"]);
    git(&f.a, &["branch", "-D", "feature-x"]);
    git(&wt, &["fetch", "--prune", "-q", "origin"]);

    let out = lifecycle::remove(&f.hub, "feature-x", false).unwrap();
    assert!(
        out.risks.is_empty(),
        "已经并入主干，不该再报未保存: {:?}",
        out.risks
    );
    assert!(!f.hub.ws_dir().join("feature-x").exists());
}

/// squash 合并后提交 SHA 变了，祖先关系不成立；`git cherry` 仍能看出改动已在主干。
#[test]
fn remove_after_squash_merge_does_not_warn() {
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-x", &["demo-web"])).unwrap();
    let wt = f.hub.ws_dir().join("feature-x").join("demo-web");
    std::fs::write(wt.join("new.txt"), "x").unwrap();
    git(&wt, &["add", "-A"]);
    git(
        &wt,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "work",
        ],
    );
    git(&wt, &["push", "-q", "-u", "origin", "feature-x"]);

    // 在另一个 clone 里 squash 合并：主干上是一个**不同 SHA、相同改动**的提交
    let trunk = f._tmp.path().join("trunk-work");
    git(
        f._tmp.path(),
        &["clone", "-q", f.a.to_str().unwrap(), trunk.to_str().unwrap()],
    );
    git(&trunk, &["merge", "--squash", "-q", "origin/feature-x"]);
    git(
        &trunk,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "squash merge",
        ],
    );
    git(&trunk, &["push", "-q", "origin", "HEAD:main"]);

    git(&f.a, &["branch", "-D", "feature-x"]);
    git(&wt, &["fetch", "--prune", "-q", "origin"]);

    let out = lifecycle::remove(&f.hub, "feature-x", false).unwrap();
    assert!(
        out.risks.is_empty(),
        "squash 合并同样不该报未保存: {:?}",
        out.risks
    );
}

/// 其他人合了 MR，这边本地引用还是旧的：判风险前要先 fetch 复核，不能拿旧
/// `origin/main` 拦人（实测反馈担心的情况）。
#[test]
fn remove_rechecks_stale_refs_before_blocking() {
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-x", &["demo-web"])).unwrap();
    let wt = f.hub.ws_dir().join("feature-x").join("demo-web");
    std::fs::write(wt.join("new.txt"), "x").unwrap();
    git(&wt, &["add", "-A"]);
    git(
        &wt,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "work",
        ],
    );
    git(&wt, &["push", "-q", "-u", "origin", "feature-x"]);

    // 远端：别人合了 MR（主干前进、源分支删掉）
    git(&f.a, &["update-ref", "refs/heads/main", "refs/heads/feature-x"]);
    git(&f.a, &["branch", "-D", "feature-x"]);
    // 本地：只模拟「源分支引用被清掉」，主干引用仍是旧的（远端已前进）
    git(&wt, &["update-ref", "-d", "refs/remotes/origin/feature-x"]);
    // 前提成立：本地 origin/main 确实还没看到远端这次合并
    assert_ne!(
        git(&wt, &["rev-parse", "origin/main"]),
        git(&f.a, &["rev-parse", "main"])
    );

    let out = lifecycle::remove(&f.hub, "feature-x", false).unwrap();
    assert!(
        out.risks.is_empty(),
        "旧引用不该拦人，应先 fetch 复核: {:?}",
        out.risks
    );
}

/// 刚建出来、一个提交都没有的需求：没有东西可丢，删掉不该拦。
#[test]
fn remove_allows_a_fresh_workspace_with_nothing_to_lose() {
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-x", &["demo-web"])).unwrap();

    let out = lifecycle::remove(&f.hub, "feature-x", false).unwrap();
    assert!(out.risks.is_empty(), "{:?}", out.risks);
    assert!(!f.hub.ws_dir().join("feature-x").exists());
}

#[test]
fn remove_force_deletes_everything_and_verifies_it() {
    // 这条直接对应需求 #6：旧版删完还会留下空文件夹。
    let f = Fixture::new();
    lifecycle::new(
        &f.hub,
        &f.opts("feature-x", &["demo-web", "demo-api"]),
    )
    .unwrap();

    let out = lifecycle::remove(&f.hub, "feature-x", true).unwrap();
    assert_eq!(out.removed_modules.len(), 2);

    let dir = f.hub.ws_dir().join("feature-x");
    assert!(!dir.exists(), "工作区目录必须彻底消失");
    assert!(
        std::fs::read_dir(f.hub.ws_dir()).unwrap().next().is_none(),
        "ws/ 下不该有任何残留"
    );

    // 本地分支也要清掉，否则下次建同名需求会撞「分支已存在于本地」
    for m in ["demo-web", "demo-api"] {
        assert!(!git_ok(
            &f.hub.repos_dir().join(m),
            &["show-ref", "--verify", "-q", "refs/heads/feature-x"]
        ));
    }
    // 不留悬空 worktree 元数据
    let list = git(
        &f.hub.repos_dir().join("demo-web"),
        &["worktree", "list"],
    );
    assert_eq!(list.lines().count(), 1, "只应剩主仓库本身:\n{list}");
}

#[test]
fn remove_cleans_up_readonly_files() {
    // rm -rf 碰到只读文件会删一部分卡住——旧版正是在这里静默失败还报成功
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-x", &["demo-web"])).unwrap();

    let wt = f.hub.ws_dir().join("feature-x").join("demo-web");
    let ro = wt.join("readonly.txt");
    std::fs::write(&ro, "x").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o444)).unwrap();
    }

    lifecycle::remove(&f.hub, "feature-x", true).unwrap();
    assert!(!f.hub.ws_dir().join("feature-x").exists());
}

#[test]
fn list_after_remove_shows_nothing() {
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-x", &["demo-web"])).unwrap();
    assert_eq!(workspace::list(&f.hub).unwrap().len(), 1);

    lifecycle::remove(&f.hub, "feature-x", true).unwrap();
    assert!(workspace::list(&f.hub).unwrap().is_empty());
}

#[test]
fn meta_file_is_written_at_workspace_root_not_inside_modules() {
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-x", &["demo-web"])).unwrap();

    let root = f.hub.ws_dir().join("feature-x");
    assert!(root.join(META_FILE).is_file());
    assert!(!root.join("demo-web").join(META_FILE).exists());
}
#[test]
fn remove_reports_forced_so_callers_do_not_claim_it_was_kept() {
    // 回归：`risks` 非空既可能意味着"因此没删"，也可能意味着"带着风险删了"。
    // 只看 risks 渲染会把后者报成"未删除"，而东西其实已经没了。
    let f = Fixture::new();
    lifecycle::new(&f.hub, &f.opts("feature-x", &["demo-web"])).unwrap();
    // 先造一个「删了会丢」的本地提交，才有风险项可言
    let wt = f.hub.ws_dir().join("feature-x").join("demo-web");
    std::fs::write(wt.join("new.txt"), "x").unwrap();
    git(&wt, &["add", "-A"]);
    git(
        &wt,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "work",
        ],
    );

    let kept = lifecycle::remove(&f.hub, "feature-x", false).unwrap();
    assert!(!kept.risks.is_empty());
    assert!(!kept.forced, "未加 --force，应标记为未强制");
    assert!(f.hub.ws_dir().join("feature-x").exists());

    let forced = lifecycle::remove(&f.hub, "feature-x", true).unwrap();
    assert!(!forced.risks.is_empty(), "风险项仍应如实报出");
    assert!(forced.forced, "带 --force 执行，必须标记为已强制");
    assert!(!f.hub.ws_dir().join("feature-x").exists());
}

/// **半截状态的 worktree 要能自愈。**
///
/// 上一次操作被打断（被杀 / 断电 / 手工删目录）会留下「指向已消失目录的登记」，
/// 于是分支被"占着"，本次 `worktree add` 必失败 —— 报错还说不出该怎么办
/// （实测反馈：其他人在 Windows 上撞的 `Could not reset index file to revision 'HEAD'`
/// 就是这一类）。
///
/// 现在的实现：失败 → `worktree prune` + 清掉目标目录残留 → 重试一次。
/// 这两步只清理**本来就不该存在**的东西，不会碰任何有用的内容。
#[test]
fn a_stale_worktree_registration_is_cleaned_up_and_the_add_retried() {
    use gws_core::git::Git;

    let f = Fixture::new();
    let repo = f.hub.repos_dir().join("demo-web");
    let git = Git::new(&repo);

    // 造出那个半截状态：建一个 worktree、记下分支，然后把**目录**删掉（登记还在）
    let branch = "feature-20261010-stale";
    let ghost = repo.join("ghost-wt");
    git.worktree_add(&ghost, branch, true, Some("origin/main"))
        .unwrap();
    std::fs::remove_dir_all(&ghost).unwrap();

    // 不做任何清理，直接对**同一个分支**再建一个 worktree —— 原样会失败
    // （`'<分支>' is already used by worktree at <已不存在的路径>`）
    let target = repo.join("real-wt");
    git.worktree_add(&target, branch, false, None)
        .expect("应当自动清理陈旧登记后重试成功");

    assert!(target.join("README.md").is_file(), "worktree 要真的建出来");
    // 目标目录与分支都对
    assert_eq!(git_of(&target, &["branch", "--show-current"]), branch);
}

fn git_of(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git 可执行");
    assert!(
        out.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 「换个建法」那步的**安全闸**：本地分支只要有远程没有的提交，就绝不能删了重建。
///
/// 实测背景：Windows 上 `worktree add <路径> <分支>`（复用本地分支那条路）
/// 会以 `Could not reset index file to revision 'HEAD'` 失败，而
/// `-b <分支> <路径> origin/<分支>`（从远程新建）能过。两边 sha 一致时，
/// 我们就把本地分支删了按远程重建 —— 但这条**必须**先确认没丢代码的风险。
#[test]
fn recreating_a_local_branch_from_remote_is_only_allowed_when_nothing_is_unpushed() {
    use gws_core::git::Git;

    let f = Fixture::new();
    let repo = f.hub.repos_dir().join("demo-web");
    let git = Git::new(&repo);

    // 与远程一致的本地分支 → 允许重建
    let branch = "feature-20261010-same-as-remote";
    let wt = repo.join("wt-same");
    git.worktree_add(&wt, branch, true, Some("origin/main"))
        .unwrap();
    git.push(branch, true).unwrap();
    assert!(
        git.can_recreate_from_remote(branch),
        "本地与 origin 一致 → 可以删了重建（那条路在 Windows 上才走得通）"
    );

    // 本地多了一个提交 → **不许**重建（删了就是丢代码）
    std::fs::write(wt.join("new.txt"), "x").unwrap();
    git_of(&wt, &["add", "-A"]);
    git_of(
        &wt,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "还没推的东西",
        ],
    );
    assert!(
        !git.can_recreate_from_remote(branch),
        "本地有远程没有的提交 → 宁可失败也不能删分支"
    );

    // 远程根本没有这条分支 → 保守，不许
    assert!(!git.can_recreate_from_remote("feature-不存在-la"));
}

#[test]
fn new_with_only_the_doc_hub_pulls_no_code_modules() {
    // 与界面默认一致：只勾文档库时，建出来的工作区里一个代码模块都不该有，
    // 但仍要登记文档归属；没填分类时按年月（如 2026-10）归组。
    let f = Fixture::new();
    let branch = "feature-20261021-docs-only";
    let opts = NewOptions {
        branch: branch.into(),
        title: "只写文档的需求".into(),
        prefix: "feature".into(),
        base: vec!["主干".into()],
        modules: vec!["doc-hub".into()],
        doc_family: None,
        notes: String::new(),
        links: Vec::new(),
    };

    let out = lifecycle::new(&f.hub, &opts).unwrap();
    assert!(
        out.created.is_empty(),
        "不该拉任何代码模块：{:?}",
        out.created
    );
    assert!(out.wants_docs, "勾了文档库就要报「要挂文档」");

    let ws = Workspace::open(&f.hub, branch).unwrap();
    let doc = ws.meta().doc.as_ref().expect("应登记文档归属");
    assert_eq!(
        doc.family,
        gws_core::dochub::DocHub::month_family_name(&ws.meta().created),
        "没填分类时默认按年月"
    );
    assert!(!ws.dir().join("demo-api").exists(), "不能有代码模块目录");
    assert!(ws.dir().join(META_FILE).is_file());
}
