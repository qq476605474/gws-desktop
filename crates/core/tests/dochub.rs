//! doc-hub 集成测试。
//!
//! 用**本地裸仓库**冒充 doc-hub：既能完整验证目录契约与分支流程，又不会往
//! 真实的共享文档库写任何东西。真实的 doc-hub 只在用户明确批准时才推。

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use gws_core::dochub::{DocHub, PeriodMeta, META_FILE};
use gws_core::hub::{Hub, InitOptions};
use gws_core::repo;
use gws_core::workspace::lifecycle::{self, NewOptions};
use gws_core::workspace::Workspace;

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git 可执行");
    assert!(
        out.status.success(),
        "git {args:?} 失败:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 造一个本地裸仓库当 doc-hub，带 `main` 分支和一条初始内容。
fn fake_doc_hub(root: &Path) -> PathBuf {
    let bare = root.join("doc-hub.git");
    git(
        root,
        &["init", "--bare", "-b", "main", bare.to_str().unwrap()],
    );

    let seed = root.join("dochub-seed");
    git(root, &["init", "-b", "main", seed.to_str().unwrap()]);
    std::fs::write(seed.join("README.md"), "# doc-hub\n").unwrap();
    std::fs::create_dir_all(seed.join("guides")).unwrap();
    std::fs::write(seed.join("guides/existing.md"), "# 既有产品文档\n").unwrap();
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
        let work = tmp.path().to_path_buf();

        // 一个代码仓库（doc-hub 之外还得有东西可建工作区）
        let code = work.join("demo-api.git");
        git(
            &work,
            &["init", "--bare", "-b", "main", code.to_str().unwrap()],
        );
        let seed = work.join("code-seed");
        git(&work, &["init", "-b", "main", seed.to_str().unwrap()]);
        std::fs::write(seed.join("README.md"), "# code\n").unwrap();
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
        git(&seed, &["remote", "add", "origin", code.to_str().unwrap()]);
        git(&seed, &["push", "-q", "origin", "main"]);

        // hub 的 doc-hub 地址指向本地裸仓库
        let mut hub = Hub::init(&work.join("hub"), InitOptions::default()).unwrap();
        // 单测不联网：直接写 GitLab 绑定（`repo::add` 等操作要求 hub 已绑定）。
        // 测试里的仓库都是本地路径，`check_remote` 一律放行，不会真去连这个地址。
        hub.config_mut().gitlab.base_url = "https://gitlab.example.com".into();
        // 提交身份也在这里定死：**不要指望机器上的 git 全局配置** ——
        // CI 的 Linux / Windows runner 上根本没有，而文档提交（`DocHub::sync`）
        // 由工具直接 `git commit`，会以 `fatal: empty ident name` 全红。
        // （与 CLAUDE.md 里「测试里不要假设环境干净」是同一条。）
        hub.config_mut().git = gws_core::hub::GitIdentity {
            author_name: "测试".into(),
            author_email: "test@example.com".into(),
        };
        hub.save_config().unwrap();
        let doc_url = fake_doc_hub(&work).to_string_lossy().to_string();
        hub.config_mut().dochub.url = doc_url;
        hub.save_config().unwrap();
        let hub = Hub::open(hub.root()).unwrap();

        repo::add(&hub, &[code.to_string_lossy().into()], None).unwrap();
        Self { _tmp: tmp, hub }
    }

    fn create(&self, branch: &str, title: &str) -> Workspace {
        self.create_with(branch, title, None)
    }

    /// 建需求时勾上 doc-hub —— 桌面端默认就是这样，`lifecycle::new` 会预登记
    /// 文档归属（`meta.doc`），之后的 attach 就按它走。
    fn create_with_docs(&self, branch: &str, title: &str) -> Workspace {
        self.create_with(branch, title, Some("doc-hub".into()))
    }

    fn create_with(&self, branch: &str, title: &str, docs: Option<String>) -> Workspace {
        let mut modules = vec!["demo-api".into()];
        modules.extend(docs);
        let opts = NewOptions {
            branch: branch.to_string(),
            title: title.to_string(),
            prefix: "feature".into(),
            base: vec!["主干".into()],
            modules,
            doc_family: None,
            notes: String::new(),
            links: Vec::new(),
        };
        lifecycle::new(&self.hub, &opts).unwrap();
        Workspace::open(&self.hub, branch).unwrap()
    }

    fn dochub(&self) -> DocHub {
        DocHub::ensure(&self.hub).unwrap()
    }
}

#[test]
fn ensure_clones_the_doc_hub_into_the_hub() {
    let f = Fixture::new();
    assert!(!f.hub.dochub_dir().exists(), "初始不该有 doc-hub");

    let dh = f.dochub();

    assert!(dh.root().join(".git").is_dir(), "应克隆出 doc-hub");
    assert!(dh.root().join("README.md").is_file(), "应是真实的 checkout");
    assert!(
        dh.root().join("guides/existing.md").is_file(),
        "既有产品文档应在"
    );
    // 既有的治理体系不受影响
    assert!(
        !dh.root().join("workspaces").exists(),
        "workspaces/ 由我们新建"
    );
}

#[test]
fn ensure_is_idempotent_and_does_not_reclone() {
    let f = Fixture::new();
    let first = f.dochub();
    std::fs::write(first.root().join("本地标记.txt"), "x").unwrap();

    let second = f.dochub();

    assert!(
        second.root().join("本地标记.txt").is_file(),
        "不该重新克隆把本地改动冲掉"
    );
}

// ══ 已归档文档：同步之后要能真的看到 ═══════════════════════════════════
//
// 实测反馈：「点了同步最新文档，同步下来了但我这里看不到」。
//
// 根因：已归档（已并入主干）的期次只存在于 `main` 上，界面是**按工作树路径**
// 去读它们的（`repos/doc-hub/workspaces/<族>/<分支>/`），而 `ensure_with`
// 只 `git fetch` —— ref 更新了、列表里能看见那一期（那条走 git 对象），
// 工作树却还停在旧提交上，于是点开是空的。

/// 以「其他人」的身份往文档库的 `main` 推一期文档（模拟别人合进去了）。
///
/// fixture 里那个 seed 仓库的 origin 就是本地裸仓库，直接拿它当第二个人。
fn colleague_pushes_to_main(hub_root: &Path, family: &str, branch: &str) {
    let seed = hub_root.parent().expect("hub 的上一级").join("dochub-seed");
    let dir = seed.join("workspaces").join(family).join(branch);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("需求说明.md"), "# 需求说明\n").unwrap();
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
            "docs",
        ],
    );
    git(&seed, &["push", "-q", "origin", "main"]);
}

#[test]
fn ensure_brings_the_archived_docs_into_the_main_clone_working_tree() {
    let f = Fixture::new();
    let dh = f.dochub(); // 先克隆：主 clone 停在此刻的 main

    colleague_pushes_to_main(f.hub.root(), "示例需求", "feature-20260101-archived");
    let archived = dh
        .root()
        .join("workspaces/示例需求/feature-20260101-archived");
    assert!(
        !archived.exists(),
        "克隆之后别人推的内容，本地工作树里本来是没有的（这正是当初看不到的原因）"
    );

    // 界面上的「同步最新文档」
    DocHub::ensure(&f.hub).unwrap();

    assert!(
        archived.join("需求说明.md").is_file(),
        "同步之后已归档文档要能在工作树里看到：{}",
        archived.display()
    );
}

#[test]
fn ensure_does_not_touch_a_dirty_main_clone() {
    // 快进是真会动文件的。这个 clone 只是「一份给人看的镜像」，
    // 本地有改动时宁可不同步，也不能把人的东西冲掉。
    let f = Fixture::new();
    let dh = f.dochub();
    let mine = dh.root().join("我手写的东西.txt");
    std::fs::write(&mine, "别动我").unwrap();

    colleague_pushes_to_main(f.hub.root(), "示例需求", "feature-20260101-archived");
    DocHub::ensure(&f.hub).unwrap();

    assert!(mine.is_file(), "本地改动不能被冲掉");
    assert_eq!(std::fs::read_to_string(&mine).unwrap(), "别动我");
    assert!(
        !dh.root()
            .join("workspaces/示例需求/feature-20260101-archived")
            .exists(),
        "脏工作树就不该快进（宁可不同步）"
    );
}

#[test]
fn ensure_does_not_touch_a_diverged_main_clone() {
    // 本地 main 有自己的提交 → 分叉，`--ff-only` 会失败；
    // 我们要的是**根本不尝试**，而不是失败后留下半个状态。
    let f = Fixture::new();
    let dh = f.dochub();
    let root = dh.root().to_path_buf();
    std::fs::write(root.join("本地提交.md"), "# 我在这边改过\n").unwrap();
    git(&root, &["add", "-A"]);
    git(
        &root,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "local",
        ],
    );

    colleague_pushes_to_main(f.hub.root(), "示例需求", "feature-20260101-archived");
    DocHub::ensure(&f.hub).unwrap();

    assert!(root.join("本地提交.md").is_file(), "分叉时不碰它");
    assert!(!root.join("MERGE_HEAD").exists(), "不该留下合并中途的状态");
}

#[test]
fn attach_creates_branch_worktree_and_period_dir() {
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-20261010-smart-reach", "示例需求");

    // 显式给分类：这个用例验的是"目录与元数据的契约"，不该顺带依赖族名兜底规则
    let out = dh.attach(&mut ws, Some("示例需求")).unwrap();

    assert!(out.created);
    assert_eq!(out.family, "示例需求");
    assert_eq!(
        out.relative_dir,
        "workspaces/示例需求/feature-20261010-smart-reach"
    );

    // 文档 worktree 挂在 ws/<分支>/docs
    let docs = dh.docs_dir(&ws);
    assert!(docs.join(".git").exists(), "docs 应是 doc-hub 的 worktree");
    assert_eq!(
        git(&docs, &["branch", "--show-current"]),
        "feature-20261010-smart-reach",
        "文档分支应与代码分支同名"
    );

    // 期次目录 + 元数据 + AI 约定
    let period = dh.period_dir(&ws);
    assert!(period.join(META_FILE).is_file());
    assert!(
        period.join("CLAUDE.md").is_file(),
        "需求 #2：要有 CLAUDE.md 说明文档去处"
    );
    // 同时也写 AGENTS.md（同内容）——也有人用这个文件名
    assert!(
        period.join("AGENTS.md").is_file(),
        "期次目录也要有 AGENTS.md"
    );

    let meta = PeriodMeta::load(&period.join(META_FILE)).unwrap();
    assert_eq!(meta.branch, "feature-20261010-smart-reach");
    assert_eq!(meta.family, "示例需求");
    assert_eq!(meta.label, "V1", "首期应是 V1");
    assert_eq!(meta.repos, vec!["demo-api"]);

    // 工作区元数据回写了 doc 归属
    let reopened = Workspace::open(&f.hub, "feature-20261010-smart-reach").unwrap();
    let doc = reopened.meta().doc.as_ref().expect("应登记 doc 归属");
    assert_eq!(doc.family, "示例需求");
    assert_eq!(doc.period, "feature-20261010-smart-reach");
}

#[test]
fn attach_is_idempotent() {
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-x", "示例需求");

    dh.attach(&mut ws, Some("示例需求")).unwrap();
    let again = dh.attach(&mut ws, Some("示例需求")).unwrap();

    assert!(!again.created, "第二次不该重复建");
    assert_eq!(again.family, "示例需求");
}

#[test]
fn a_second_period_joins_the_family_when_the_category_is_given() {
    // 同一需求做 V2：填了同一个分类，就落在同一个需求分类目录下
    let f = Fixture::new();
    let dh = f.dochub();

    let mut v1 = f.create("feature-20260901-smart-reach", "示例需求");
    dh.attach(&mut v1, Some("示例需求")).unwrap();
    std::fs::write(dh.period_dir(&v1).join("需求文档.md"), "# V1\n").unwrap();

    // 第二期换了措辞的中文名，但分类填的是同一个
    let mut v2 = f.create("feature-20261010-smart-reach-v2", "示例需求V2期");
    let out = dh.attach(&mut v2, Some("示例需求")).unwrap();

    assert_eq!(out.family, "示例需求");
    assert_eq!(out.label, "V2", "第二期应编号 V2");

    let families = dh.families().unwrap();
    assert_eq!(families.len(), 1, "只应有一个族目录");
    assert_eq!(families[0].periods.len(), 2);
    // 按期次日期排序
    assert_eq!(families[0].periods[0].meta.label, "V1");
    assert_eq!(families[0].periods[1].meta.label, "V2");
}

#[test]
fn a_blank_category_never_borrows_a_related_family() {
    // 实测反馈：新建 `feature-20261021-test`、没填分类，却被归进了已有的
    // `test` 族。现在没填分类**只能是年月**，哪怕族名/标题看起来相关。
    let f = Fixture::new();
    let dh = f.dochub();

    let mut old = f.create("feature-20260901-test", "test");
    dh.attach(&mut old, Some("test")).unwrap();
    std::fs::write(dh.period_dir(&old).join("需求文档.md"), "# old\n").unwrap();

    let mut ws = f.create_with_docs("feature-20261021-test", "test");
    let out = dh.attach(&mut ws, None).unwrap();

    assert_eq!(out.family, &ws.meta().created[..7], "没填分类应按年月归组");
    assert_ne!(out.family, "test", "不该被同名/同标题的旧族吸走");
}

#[test]
fn a_period_with_only_scaffolding_has_no_content() {
    // 界面上不该出现「只有工具生成的三个文件」的空期次。
    // meta.json 是元数据，CLAUDE.md/AGENTS.md 是说明，都不算需求正文。
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-20261010-empty-doc", "空文档需求");
    dh.attach(&mut ws, None).unwrap();

    let period = dh.period_dir(&ws);
    assert!(period.join("meta.json").is_file());
    assert!(period.join("AGENTS.md").is_file());
    assert!(!gws_core::dochub::period_has_content(&period));

    let fams = dh.families().unwrap();
    let p = fams[0]
        .periods
        .iter()
        .find(|p| p.meta.branch == ws.branch())
        .unwrap();
    assert!(!p.has_content, "只有脚手架时应判为空");
    assert!(p.available, "工作区里有这一期目录，文档页打得开");

    // 写了一篇真正的需求文档 → 有内容
    std::fs::write(period.join("需求文档.md"), "# 需求\n").unwrap();
    assert!(gws_core::dochub::period_has_content(&period));
    let fams = dh.families().unwrap();
    assert!(
        fams[0]
            .periods
            .iter()
            .find(|p| p.meta.branch == ws.branch())
            .unwrap()
            .has_content
    );
}

#[test]
fn content_written_only_to_the_doc_branch_is_still_seen() {
    // worktree 被摘掉后，families 只能从分支树判断；正文在那儿也要算「有内容」。
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-20261011-branch-only", "分支上的文档");
    dh.attach(&mut ws, None).unwrap();

    let period = dh.period_dir(&ws);
    std::fs::write(period.join("需求文档.md"), "# 只有分支上有\n").unwrap();
    dh.sync(&ws, "docs: add", false).unwrap();
    dh.detach(&mut ws).unwrap();

    let fams = dh.families().unwrap();
    let p = fams[0]
        .periods
        .iter()
        .find(|p| p.meta.branch == "feature-20261011-branch-only")
        .expect("摘掉 worktree 后期次仍应从分支树读出来");
    assert!(p.has_content, "分支树里有正文，has_content 必须为 true");
    assert!(
        !p.available,
        "工作区已摘掉、主 clone 也没有这一期 → 文档页打不开，不该列出来"
    );
}

#[test]
fn hotfix_branch_gets_bugfix_label() {
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("hotfix-20261020-smart-reach", "示例需求");
    let out = dh.attach(&mut ws, None).unwrap();
    assert_eq!(out.label, "BUGFIX");
}

#[test]
fn new_doc_creates_each_builtin_type() {
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-x", "示例需求");
    dh.attach(&mut ws, None).unwrap();

    for key in ["需求文档", "技术方案", "提测单", "测试用例", "发布步骤"] {
        let path = dh
            .new_doc(&ws, key)
            .unwrap_or_else(|e| panic!("{key} 应能创建: {e}"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(key), "模板应带标题: {text}");
        assert!(text.contains("feature-x"), "模板应带需求上下文: {text}");
    }

    // sql 是多文件类型，给的是目录里的 README
    let sql = dh.new_doc(&ws, "sql").unwrap();
    assert!(sql.to_string_lossy().contains("sql"), "{}", sql.display());

    // 重复创建要报错而不是静默覆盖
    let err = dh.new_doc(&ws, "需求文档").unwrap_err();
    assert!(err.to_string().contains("已存在"), "{err}");

    // 未知类型要给出可用列表
    let err = dh.new_doc(&ws, "不存在的类型").unwrap_err();
    assert!(err.to_string().contains("可用类型"), "{err}");
}

#[test]
fn status_reports_pending_docs_and_sync_commits_them() {
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-x", "示例需求");
    dh.attach(&mut ws, None).unwrap();

    // attach 自己就已产生待提交内容（meta.json / CLAUDE.md / sql/）
    let pending = dh.status(&ws).unwrap();
    assert!(!pending.is_empty(), "attach 之后应有待提交内容");

    let out = dh.sync(&ws, "docs: 初始化需求文档", false).unwrap();
    assert!(out.committed);
    assert!(!out.pushed, "这里不推远程");
    assert!(
        out.changed.iter().any(|p| p.contains("meta.json")),
        "{:?}",
        out.changed
    );

    // 提交后应干净
    assert!(
        dh.status(&ws).unwrap().is_empty(),
        "提交后不该还有待提交内容"
    );

    // 再 sync 一次不该产生空提交
    let again = dh.sync(&ws, "docs: 无变化", false).unwrap();
    assert!(!again.committed, "没有变更时不该提交");
}

#[test]
fn docs_are_readable_from_the_main_clone_without_switching_branches() {
    // AI 读历史文档的场景：主 clone 不动分支就能看到内容（worktree 共享对象库）
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-x", "示例需求");
    dh.attach(&mut ws, Some("示例需求")).unwrap();
    dh.new_doc(&ws, "技术方案").unwrap();
    dh.sync(&ws, "docs: 技术方案", false).unwrap();

    // 主 clone 仍在 main 上，但对象库里已有该分支
    assert_eq!(git(dh.root(), &["branch", "--show-current"]), "main");
    let show = git(
        dh.root(),
        &[
            "show",
            "feature-x:workspaces/示例需求/feature-x/技术方案.md",
        ],
    );
    assert!(show.contains("技术方案"), "{show}");
}

#[test]
fn removing_the_workspace_leaves_docs_worktree_registered() {
    // 删除工作区时若不清 docs worktree，doc-hub 里会留下悬空引用。
    // 这条测试记录当前行为，防止以后改坏。
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-x", "示例需求");
    dh.attach(&mut ws, None).unwrap();
    dh.sync(&ws, "docs: init", false).unwrap();

    let docs = dh.docs_dir(&ws);
    assert!(docs.exists());

    // 直接删目录（模拟未清理的情况）
    std::fs::remove_dir_all(&docs).unwrap();

    // 主 clone 里 prune 之后不该再有悬空 worktree
    git(dh.root(), &["worktree", "prune"]);
    let list = git(dh.root(), &["worktree", "list"]);
    assert_eq!(list.lines().count(), 1, "prune 后只应剩主仓库:\n{list}");
}

// ══ 读操作不联网 ═══════════════════════════════════════════════════════

#[test]
fn ensure_local_does_not_clone() {
    // 读操作（列需求分类、读某期文档）走 ensure_local：**不联网、不克隆**。
    // 原来它们也走 ensure_with，于是每次切到文档页都跑一次 git fetch，
    // 界面卡在那转圈——而用户只是要看本地已有的文档（实测反馈）。
    let f = Fixture::new();
    assert!(!f.hub.dochub_dir().exists());

    let err = DocHub::ensure_local(&f.hub).err().expect("应报错");
    assert!(err.to_string().contains("还没拉到本地"), "{err}");
    assert!(!f.hub.dochub_dir().exists(), "不该顺手克隆");
}

#[test]
fn ensure_local_opens_an_existing_clone() {
    let f = Fixture::new();
    f.dochub(); // 先克隆

    let dh = DocHub::ensure_local(&f.hub).unwrap();
    assert!(dh.root().join("README.md").is_file());
    // 读操作能直接跑，不需要网络
    assert!(dh.families().unwrap().is_empty());
}

// ══ 期次标注「是否已并入主干」═══════════════════════════════════════════

#[test]
fn periods_are_marked_merged_or_not() {
    // 界面据此区分「已归档的历史文档」与「还在开发的文档」——
    // AI 不该改已归档的那些。
    let f = Fixture::new();
    let dh = f.dochub();

    let mut v1 = f.create("feature-20260901-a", "示例需求");
    dh.attach(&mut v1, Some("示例需求")).unwrap();
    // 有正文才算"真的族"
    std::fs::write(dh.period_dir(&v1).join("需求文档.md"), "# V1\n").unwrap();
    dh.sync(&v1, "docs: v1", false).unwrap();

    let mut v2 = f.create("feature-20261010-b", "示例需求");
    dh.attach(&mut v2, Some("示例需求")).unwrap();
    dh.sync(&v2, "docs: v2", false).unwrap();

    // 两个都还没并入 main
    let fams = dh.families().unwrap();
    assert_eq!(fams[0].periods.len(), 2);
    assert!(
        fams[0].periods.iter().all(|p| !p.merged),
        "还没合并的应标为开发中"
    );

    // 把 V1 合进 main
    git(dh.root(), &["checkout", "-q", "main"]);
    // 合完就留在 main —— 那个分支正被它自己的 docs worktree 占着，切不回去
    git(
        dh.root(),
        &["merge", "-q", "--no-edit", "feature-20260901-a"],
    );

    let fams = dh.families().unwrap();
    let by_branch = |b: &str| {
        fams[0]
            .periods
            .iter()
            .find(|p| p.meta.branch == b)
            .unwrap()
            .merged
    };
    assert!(
        by_branch("feature-20260901-a"),
        "已并入 main 的应标为已归档"
    );
    assert!(!by_branch("feature-20261010-b"), "没并入的仍应是开发中");
}

#[test]
fn a_period_with_no_commits_of_its_own_is_not_archived() {
    // 实测踩到的坑：一个刚建出来、文档还没提交的期次分支，恰好与主干**同一个
    // 提交**，于是被判成「已归档」——用户看到一份正在写的文档被列进了
    // 「已并入主干、只读、不要改」那一栏。
    //
    // 原来的判据是 `git branch --merged <主干>`（该分支的提交是否都在主干里），
    // 而它对「自己没有任何提交」的分支**平凡成立**：空集是任何集合的子集。
    let f = Fixture::new();
    let dh = f.dochub();

    let mut v1 = f.create("feature-260917-still-writing", "示例需求");
    dh.attach(&mut v1, None).unwrap();
    // 故意**不** sync —— 文档还是未提交状态，分支与主干同提交

    let fams = dh.families().unwrap();
    let p = fams[0]
        .periods
        .iter()
        .find(|p| p.meta.branch == "feature-260917-still-writing")
        .expect("未提交的期次也该被扫到（走 worktree）");
    assert!(
        !p.merged,
        "没有自己提交的分支不该算「已归档」，它什么都还没合进主干"
    );

    // 提交之后仍然是开发中（合进主干才算归档）
    dh.sync(&v1, "docs: wip", false).unwrap();
    let fams = dh.families().unwrap();
    assert!(
        !fams[0]
            .periods
            .iter()
            .find(|p| p.meta.branch == "feature-260917-still-writing")
            .unwrap()
            .merged,
        "只在分支上提交、没合进主干，仍是开发中"
    );
}

#[test]
fn archived_detection_survives_chinese_family_names() {
    // 族目录名是中文，而 git 默认把非 ASCII 路径转义成八进制
    // （`"workspaces/\347\247\201..."`）。判据要从 ls-tree 的输出里切出
    // 「族/分支」，不显式关掉 quotePath 就会切出一串八进制、永远匹配不上——
    // 而且是**静默**全错（所有期次都被当成开发中）。
    let f = Fixture::new();
    let dh = f.dochub();

    let mut v1 = f.create("feature-20260901-中文族", "示例需求");
    // 这个用例要的正是**中文族名**这个条件，所以显式给
    dh.attach(&mut v1, Some("示例需求")).unwrap();
    dh.sync(&v1, "docs: v1", false).unwrap();

    git(dh.root(), &["checkout", "-q", "main"]);
    git(
        dh.root(),
        &["merge", "-q", "--no-edit", "feature-20260901-中文族"],
    );

    let fams = dh.families().unwrap();
    assert_eq!(fams[0].name, "示例需求");
    assert!(
        fams[0]
            .periods
            .iter()
            .find(|p| p.meta.branch == "feature-20260901-中文族")
            .unwrap()
            .merged,
        "需求分类目录下已合并的期次必须能被认出来"
    );
}

// ══ 布局迁移（实测反馈 #9）═════════════════════════════════════════════
    /// **文档库目录名跟着配置走。**
    ///
    /// 早期版本把它写死成 `doc-hub`：用户填的明明是 `x-doc-hub`，hub 里却建出个
    /// `doc-hub` 目录 —— 名字对不上，界面看着就像「文档没拉下来」（实测反馈）。
    /// 这里造一个「目录名与配置不符」的 hub，确认打开时会被改回配置里那个名字。
    #[test]
    fn dochub_dir_is_renamed_to_the_configured_name() {
        let f = Fixture::new();
        let dh = f.dochub();
        let mut ws = f.create("feature-260917-naming", "示例需求");
        dh.attach(&mut ws, None).unwrap();

        let want = f.hub.dochub_name();
        let good = f.hub.dochub_new_dir();
        assert!(good.join(".git").is_dir(), "先确认文档库拉下来了");

        // 制造「目录名与配置不符」：早期版本会写死成 `doc-hub`，
        // 而配置推导出来的是另一个名字（测试里是 doc-hub，所以换个历史名来造）
        let legacy = if want == "x-doc-hub" { "doc-hub" } else { "x-doc-hub" };
        let wrong = f.hub.repos_dir().join(legacy);
        std::fs::rename(&good, &wrong).unwrap();
        let ws_good = ws.dir().join(&want);
        if ws_good.exists() {
            std::fs::rename(&ws_good, ws.dir().join(legacy)).unwrap();
        }

        // 重新打开 hub → 应把名字对齐回配置里那个
        let hub = Hub::open(f.hub.root()).unwrap();
        assert!(
            hub.migration().failed.is_empty(),
            "对齐不该失败：{:?}",
            hub.migration().failed
        );
        assert!(!wrong.exists(), "旧名字的目录不该还在");
        assert_eq!(hub.dochub_dir(), hub.dochub_new_dir(), "应回到配置里的名字下");
        assert!(hub.dochub_dir().join(".git").is_dir(), "文档库应仍可用");
    }

#[test]
fn migration_is_skipped_when_already_new_layout() {
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-260917-fresh", "示例需求");
    dh.attach(&mut ws, None).unwrap();

    // 全新 hub 本来就是新布局，打开时不该动任何东西
    let hub = Hub::open(f.hub.root()).unwrap();
    assert!(hub.migration().did_nothing(), "{:?}", hub.migration());
    assert!(hub.dochub_dir().join(".git").is_dir());
}

// ══ 文档库要能参与「模块级操作」════════════════════════════════════════
//
// 实测反馈：合代码 /「合并到环境并推送」时**选不到 doc-hub**。
//
// 根因是两层：文档库不在 `meta.modules` 里（由 `meta.doc` 单独登记），
// 所以后端的候选里没有它；而因为保留名不算「未匹配的模块」，选中了也是
// **静默跳过**、连报错都没有。前端另有一层过滤把 doc-hub 从列表里去掉。

#[test]
fn the_doc_worktree_participates_in_module_level_operations() {
    use gws_core::workspace::{sync, ModuleFilter};

    let f = Fixture::new();
    let mut ws = f.create("feature-20261013-doc-merge", "文档也要跟需求一起合");
    f.dochub().attach(&mut ws, None).unwrap();

    // ① 候选模块要含文档库（而且代码模块一个不能少）
    let cands = sync::op_candidates(&ws, "doc-hub");
    assert!(
        cands.iter().any(|m| m == "doc-hub"),
        "候选里应有 doc-hub: {cands:?}"
    );
    assert!(
        cands.iter().any(|m| m == "demo-api"),
        "代码模块不能丢: {cands:?}"
    );

    // ② 状态里也要有一行 —— 界面靠它把文档库列进模块勾选
    let st = sync::status(&f.hub, &ws, &ModuleFilter::all()).unwrap();
    let doc = st
        .modules
        .iter()
        .find(|m| m.module == "doc-hub")
        .expect("状态里应有 doc-hub 那条");
    assert!(doc.present, "文档 worktree 就在磁盘上，应当 present");

    // ③ 只勾它一个时也要真的选中，而不是被静默丢掉
    let st2 = sync::status(&f.hub, &ws, &ModuleFilter::only(["doc-hub"])).unwrap();
    assert_eq!(
        st2.modules
            .iter()
            .map(|m| m.module.as_str())
            .collect::<Vec<_>>(),
        vec!["doc-hub"],
        "只勾文档库时应当就它一个"
    );
}

#[test]
fn no_doc_attached_means_no_doc_candidate() {
    // 别凭空多出一个不存在的模块：没挂文档就没有 doc-hub
    use gws_core::workspace::{sync, ModuleFilter};

    let f = Fixture::new();
    let ws = f.create("feature-20261013-no-doc", "没挂文档");

    assert!(!sync::op_candidates(&ws, "doc-hub").iter().any(|m| m == "doc-hub"));
    let st = sync::status(&f.hub, &ws, &ModuleFilter::all()).unwrap();
    assert!(st.modules.iter().all(|m| m.module != "doc-hub"));
}

#[test]
fn the_doc_worktree_participates_in_merge_requests() {
    // 实测反馈：**提 MR 时选不了 doc-hub**，于是少提了一个。
    //
    // 推送 / 合并那边修过（用 `op_candidates`），MR 这条路漏了 —— 它还在用
    // `present_modules()`，而文档库不在 `meta.modules` 里，于是被**静默丢掉**：
    // 勾了它一个 MR 都不提，连报错都没有（保留名不算「未匹配模块」）。
    use gws_core::workspace::{mr, ModuleFilter};

    let f = Fixture::new();
    let mut ws = f.create("feature-20261013-doc-mr", "文档也要提 MR");
    f.dochub().attach(&mut ws, None).unwrap();

    // ① 默认范围里要有文档库
    let all = mr::selected_modules(&ws, &ModuleFilter::all(), "doc-hub").unwrap();
    assert!(
        all.contains(&"doc-hub".to_string()),
        "MR 范围里应有 doc-hub: {all:?}"
    );
    assert!(
        all.contains(&"demo-api".to_string()),
        "代码模块不能丢: {all:?}"
    );

    // ② 只勾它一个时**就它一个**，而不是静默变成 0 个（这才是当初的 bug 形状）
    let only = mr::selected_modules(&ws, &ModuleFilter::only(["doc-hub"]), "doc-hub").unwrap();
    assert_eq!(only, vec!["doc-hub"], "只勾文档库时应当就它一个");

    // ③ 关 MR 走的是同一条路（`close` 也用 selected_modules）
    let exclude = mr::selected_modules(&ws, &ModuleFilter::excluding(["doc-hub"]), "doc-hub").unwrap();
    assert!(
        !exclude.contains(&"doc-hub".to_string()),
        "排除生效: {exclude:?}"
    );
}

// ══ 改中文名 / 改分类 时文档要跟着动 ═══════════════════════════════════
//
// 实测反馈：需求的中文名与「需求分类」都能改，但**文档那边什么都不会变**。
// 分类名就是目录名，不挪目录 = 元数据指向新分类、内容还在旧分类 →
// 文档页按分类读，等于**文档丢了**。

#[test]
fn renaming_the_family_moves_the_whole_period_dir() {
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-20261013-rename", "改分类测试");
    dh.attach(&mut ws, None).unwrap();

    let old = dh.period_dir(&ws);
    assert!(old.is_dir(), "先有旧分类下的期次目录");
    // 放一份内容进去，验证挪的是**整棵子树**而不是只挪元数据
    std::fs::write(old.join("需求文档.md"), "内容").unwrap();

    let note = dh
        .rename_family(&ws, "新分类")
        .unwrap()
        .expect("应当真的挪了");

    let new = dh
        .docs_dir(&ws)
        .join("workspaces")
        .join("新分类")
        .join(ws.branch());
    assert!(new.is_dir(), "新分类下应当有期次目录：{}", new.display());
    assert!(!old.exists(), "旧分类下不该还留着");
    assert_eq!(
        std::fs::read_to_string(new.join("需求文档.md")).unwrap(),
        "内容",
        "内容要跟着走"
    );
    assert!(new.join("meta.json").is_file(), "元数据也要在");
    assert!(note.contains("新分类"), "说明里要写清挪到哪了：{note}");

    // 旧分类空了就该收掉，否则文档页会多出一个空分类
    let old_family_dir = dh.docs_dir(&ws).join("workspaces").join("示例需求");
    assert!(!old_family_dir.exists(), "空的旧分类目录应收掉");
}

#[test]
fn renaming_the_family_refuses_to_overwrite_an_existing_period() {
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-20261013-clash", "撞名测试");
    dh.attach(&mut ws, None).unwrap();

    // 目标位置先造一个同名期次出来
    let target = dh
        .docs_dir(&ws)
        .join("workspaces")
        .join("已占用")
        .join(ws.branch());
    std::fs::create_dir_all(&target).unwrap();

    let err = dh.rename_family(&ws, "已占用").unwrap_err();
    assert!(err.to_string().contains("已经有同名期次"), "{err}");
    // 源目录**必须原封不动**（拒绝就要拒绝干净，不能挪一半）
    assert!(dh.period_dir(&ws).is_dir(), "被拒时源目录不能被动过");
}

#[test]
fn renaming_to_a_bad_directory_name_is_rejected() {
    // 分类名会被当成目录名，带空格之类一律拒（否则路径会变得莫名其妙）
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-20261013-badname", "命名校验");
    dh.attach(&mut ws, None).unwrap();

    assert!(dh.rename_family(&ws, "带 空格").is_err());
    assert!(dh.rename_family(&ws, "a/b").is_err());
    assert!(dh.period_dir(&ws).is_dir(), "被拒时不该动源目录");
}

#[test]
fn renaming_the_family_is_a_no_op_when_nothing_changes() {
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-20261013-noop", "不变");
    dh.attach(&mut ws, None).unwrap();

    // attach 时没指定分类名 → 用需求标题当分类名，所以这里取实际的
    let cur = ws.meta().doc.as_ref().unwrap().family.clone();
    assert!(dh.rename_family(&ws, &cur).unwrap().is_none(), "同名不该动");
    assert!(
        dh.rename_family(&ws, "   ").unwrap().is_none(),
        "空名不该动"
    );
}

#[test]
fn changing_the_title_updates_the_period_metadata() {
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-20261013-title", "老名字");
    dh.attach(&mut ws, None).unwrap();

    let period = dh.period_dir(&ws);
    let name = |p: &std::path::Path| PeriodMeta::load(&p.join(META_FILE)).unwrap().name;
    assert_eq!(name(&period), "老名字", "挂载时把需求名写进了期次元数据");

    // 改需求中文名 → 期次元数据里的 name 要跟着改（文档页左列表显示的就是它）
    ws.meta_mut().title = "新名字".into();
    ws.save_meta().unwrap();
    assert!(dh.sync_title(&ws).unwrap(), "应当真的改了");

    let period = dh.period_dir(&ws); // 名字没换分类，路径不变
    assert_eq!(name(&period), "新名字");
    // 没变时不该反复写
    assert!(!dh.sync_title(&ws).unwrap(), "没变就不动");
}

#[test]
fn adding_or_dropping_a_module_updates_the_period_repos() {
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-20261013-repos", "涉及模块");
    dh.attach(&mut ws, None).unwrap();

    let period = dh.period_dir(&ws);
    let repos = |p: &std::path::Path| PeriodMeta::load(&p.join(META_FILE)).unwrap().repos;
    assert_eq!(repos(&period), vec!["demo-api".to_string()]);

    // 后来加了模块 → 文档页的「涉及模块」要跟着变（实测反馈：一直不变）
    ws.meta_mut().modules.push("demo-web".into());
    ws.save_meta().unwrap();
    assert!(dh.sync_repos(&ws).unwrap());
    assert_eq!(
        repos(&period),
        vec!["demo-api".to_string(), "demo-web".to_string()]
    );
    assert!(!dh.sync_repos(&ws).unwrap(), "没变就不写");

    // 移除模块同样要跟上
    ws.meta_mut().modules.retain(|m| m != "demo-api");
    ws.save_meta().unwrap();
    assert!(dh.sync_repos(&ws).unwrap());
    assert_eq!(repos(&period), vec!["demo-web".to_string()]);
}

#[test]
fn the_period_docs_are_regenerated_with_the_new_title() {
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-20261013-regen", "旧名");
    dh.attach(&mut ws, None).unwrap();

    ws.meta_mut().title = "改过的名字".into();
    ws.save_meta().unwrap();
    dh.sync_title(&ws).unwrap();

    let period = dh.period_dir(&ws);
    // 全文只有 AGENTS.md 一份；CLAUDE.md 只是指向它的指针
    let agents = std::fs::read_to_string(period.join("AGENTS.md")).unwrap();
    assert!(agents.contains("改过的名字"), "AGENTS.md 里应当是新名字");
    let claude = std::fs::read_to_string(period.join("CLAUDE.md")).unwrap();
    assert!(
        claude.contains("AGENTS.md"),
        "CLAUDE.md 应当是指针：{claude}"
    );
}

#[test]
fn the_doc_branch_does_not_track_trunk() {
    // 实测错误：文档分支的上游被设成了 `main`，于是 IDE 里点 Push 推成
    // `feature-xxx:main`，被 GitLab 按「受保护分支」拒掉：
    //   You are not allowed to push code to protected branches
    //
    // 原因是 `worktree add -b <分支> <路径> origin/main` 会让 git 把上游设成 main
    // 。需求工作区那边修过，文档这条漏了。
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create("feature-20261015-upstream", "上游检查");
    dh.attach(&mut ws, None).unwrap();

    let docs = dh.docs_dir(&ws);
    let out = std::process::Command::new("git")
        .current_dir(&docs)
        .args(["rev-parse", "--abbrev-ref", "@{u}"])
        .output()
        .unwrap();
    let up = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(
        !up.contains("main"),
        "文档分支不该把主干当上游（否则 IDE 推送会推成 xxx:main 被拒）：{up}"
    );
}

/// **没填需求分类 → 族名用年月，不是需求名。**
///
/// 实测反馈：新建需求时没填分类、需求名叫 `test`，文档族就被起成了 `test` ——
/// 那既不是分类、也认不出是哪段时间做的。与需求列表里"没填分类按年月归组"
/// 是同一套规则。
#[test]
fn a_new_family_falls_back_to_year_month_not_the_requirement_name() {
    let f = Fixture::new();
    let dh = f.dochub();
    let mut ws = f.create_with_docs("feature-20261016-my-test", "test");

    let out = dh.attach(&mut ws, None).unwrap();

    // 年月取自**工作区的创建日期**（不是"今天"）
    let expect = &ws.meta().created[..7];
    assert_eq!(out.family, expect, "没填分类时族名应当是年月");
    assert_ne!(out.family, "test", "绝不能拿需求名当族名");
    // 目录也真的落在那个年月下
    let dir = dh.period_dir(&ws);
    assert!(dir.to_string_lossy().contains(expect), "{}", dir.display());
}

/// **没填分类时，只有脚手架的旧族也不会把新需求"吸"过去。**
///
/// 实测反馈：没填分类、标题叫 `test`，结果被一个早先测试留下的空 `test` 族
/// 接走了，而不是按年月归组。
#[test]
fn an_empty_family_is_not_reused_for_a_new_requirement() {
    let f = Fixture::new();
    let dh = f.dochub();

    // 早先留下的族：只有脚手架，没有任何正文
    let mut old = f.create("test22221", "test");
    dh.attach(&mut old, Some("test")).unwrap();

    // 新需求同样没填分类、中文名也叫 test
    let mut ws = f.create_with_docs("feature-20261021-test-new", "test");
    let out = dh.attach(&mut ws, None).unwrap();

    let expect = &ws.meta().created[..7];
    assert_eq!(out.family, expect, "没填分类应按年月归组");
    assert_ne!(out.family, "test");
}

/// **导入别人推上来的那一期：正文要拉下来，而且不许凭空多出"未提交改动"。**
///
/// 实测反馈：另一台电脑拉了这个需求，文档没下来（只有刚生成的几个文件），
/// 而且「什么都没改却提示 3 个改动」。
///
/// 根因：文档 worktree 原来写死从 `origin/main` 建 —— 而那一期文档的正文
/// （含它自己的 CLAUDE.md / meta.json）就在 `origin/<分支>` 上。于是：
/// ① 拉下来的是空目录；② attach 随后又生成了那三个文件 → 3 个未提交改动。
#[test]
fn importing_is_attached_from_the_remote_branch_not_from_main() {
    let f = Fixture::new();
    f.dochub(); // 先把主 clone 建出来（模拟"这台电脑已有文档库"）

    let branch = "feature-20261019-imported";
    // 别人（或另一台机器上的你）已经把**这一期**推上去了：正文就在他那条分支上
    let seed = f.hub.root().parent().unwrap().join("dochub-seed");
    let period = seed.join("workspaces/示例需求").join(branch);
    std::fs::create_dir_all(&period).unwrap();
    std::fs::write(period.join("技术方案.md"), "# 我写的方案\n").unwrap();
    // 真实情况下期次目录里一定有 meta.json（工具建的）—— 有它才走"复用"那条路。
    // 用真的 `PeriodMeta` 写，别手搓 JSON（手搓的字段不全，测的就不是真场景了）
    let pushed = PeriodMeta::new(branch, "示例需求", "导入的那期");
    pushed.save(&period.join(META_FILE)).unwrap();
    git(&seed, &["checkout", "-q", "-b", branch]);
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
            "docs",
        ],
    );
    git(&seed, &["push", "-q", "origin", branch]);
    git(&seed, &["checkout", "-q", "main"]); // 回到 main，免得影响后面的用法

    // 这台电脑上导入这个需求
    let mut ws = f.create(branch, "示例需求");
    let dh = f.dochub();
    let out = dh.attach(&mut ws, Some("示例需求")).unwrap();

    // ① 正文**在**
    let docs = dh.docs_dir(&ws);
    // 这一期**是远程那份**（worktree 拉下来就带着它），所以不是"新建"
    assert!(!out.created, "远程已经有这一期了，不该当成新建");
    assert_eq!(out.family, "示例需求");
    let mine = docs
        .join("workspaces/示例需求")
        .join(branch)
        .join("技术方案.md");
    assert!(
        mine.is_file(),
        "别人推上来的正文要拉下来：{}",
        mine.display()
    );
    // 换行要归一化再比：Windows 上 git 默认 `core.autocrlf=true`，
    // checkout 出来的是 CRLF —— 那是 git 的行为，不是我们写坏了文件
    assert_eq!(
        std::fs::read_to_string(&mine).unwrap().replace("\r\n", "\n"),
        "# 我写的方案\n"
    );

    // ② **不许**多出未提交改动：拉下来就该是干净的
    let g = gws_core::git::Git::new(&docs);
    let lines = g.status_lines(true).unwrap();
    assert_eq!(
        lines.len(),
        0,
        "什么都没改，不该有未提交改动（实测反馈就是这条）：{lines:?}"
    );
}

/// **导入需求时，远程没有这一期就什么都不许生成。**
///
/// 实测反馈：导入的需求在 doc-hub 上多出 3 个改动（`meta.json` /
/// `CLAUDE.md` / `AGENTS.md`），**而且因为它们在，`git pull` 被挡住**
/// （`untracked working tree files would be overwritten by merge`）。
///
/// 那几个文件名字是确定性的 —— 只要本地的分支名和分类跟对方一样，
/// 生成出来的路径就跟他将来推上来的**完全重合**。所以"猜"不准就不该写。
#[test]
fn reusing_never_fabricates_files_when_the_remote_has_no_period() {
    let f = Fixture::new();
    f.dochub(); // 这台电脑已经有文档库了

    let branch = "feature-20261019-no-docs-yet";
    // 代码分支在远程（所以"导入"是成立的），但**文档分支根本没推过**
    let mut ws = f.create(branch, "示例需求");
    let dh = f.dochub();

    let out = dh
        .attach_with(
            &mut ws,
            Some("示例需求"),
            gws_core::dochub::AttachMode::ReuseOnly,
        )
        .unwrap();

    assert!(!out.attached, "远程没有这一期 → 这次不挂，别谎报挂上了");
    assert!(!out.created);
    // 也不算挂在需求上（`meta.doc` 留空 → 界面上是「未挂载」+「挂上文档」）
    assert!(ws.meta().doc.is_none(), "没真挂上就不该登记");

    let docs = dh.docs_dir(&ws);
    let period = docs.join("workspaces/示例需求").join(branch);
    assert!(!period.join("meta.json").exists(), "不该凭空造 meta.json");
    assert!(!period.join("CLAUDE.md").exists(), "不该凭空造 CLAUDE.md");
    assert!(!period.join("AGENTS.md").exists(), "不该凭空造 AGENTS.md");

    // 关键：**零未提交改动**（那 3 个文件就是这么来的）
    let g = gws_core::git::Git::new(&docs);
    let lines = g.status_lines(true).unwrap();
    assert_eq!(
        lines.len(),
        0,
        "导入后必须是干净的，否则 pull 就会被挡住：{lines:?}"
    );
}

/// **那一期挂在别的分类下时，要复用，而不是按猜出来的分类新建一份。**
///
/// 分类（`workspaces/<分类>/<分支>/`）一般是「指定 / 已登记 / 按年月」。
/// 猜错的时候按猜出来的路径一看"没有这一期"，
/// 于是又生成一份 —— 远程那份正文其实一直在，只是躺在另一个分类目录下。
/// 实测反馈的「导入后文档没下来 + 平白 3 个改动」就是这么来的。
#[test]
fn a_period_under_another_family_is_reused_instead_of_recreated() {
    let f = Fixture::new();
    f.dochub();

    let branch = "feature-20261019-other-family";
    let pushed_family = "增长手机号加密";
    // 对方把这一期推在「增长手机号加密」下
    let seed = f.hub.root().parent().unwrap().join("dochub-seed");
    let period = seed.join("workspaces").join(pushed_family).join(branch);
    std::fs::create_dir_all(&period).unwrap();
    std::fs::write(period.join("技术方案.md"), "# 他的方案\n").unwrap();
    PeriodMeta::new(branch, pushed_family, "另一分类下的一期")
        .save(&period.join(META_FILE))
        .unwrap();
    git(&seed, &["checkout", "-q", "-b", branch]);
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
            "docs",
        ],
    );
    git(&seed, &["push", "-q", "origin", branch]);
    git(&seed, &["checkout", "-q", "main"]);

    // 导入时**给了一个不一样的分类提示**（比如其他人在配置里写的分类跟远程不一致）
    let mut ws = f.create(branch, "示例需求");
    let dh = f.dochub();
    let out = dh
        .attach_with(
            &mut ws,
            Some("示例需求"),
            gws_core::dochub::AttachMode::ReuseOnly,
        )
        .unwrap();

    assert!(out.attached, "远程有这一期，就该挂上");
    assert_eq!(
        out.family, pushed_family,
        "分类要以**远程那份**为准，不能按提示新建"
    );
    let docs = dh.docs_dir(&ws);
    assert!(
        docs.join("workspaces")
            .join(pushed_family)
            .join(branch)
            .join("技术方案.md")
            .is_file(),
        "远程那份正文要能看到"
    );
    // 既没有新建到猜出来的分类下，也没有多出未提交改动
    assert!(
        !docs.join("workspaces/示例需求").join(branch).exists(),
        "不该另起一份"
    );
    let lines = gws_core::git::Git::new(&docs)
        .status_lines(true)
        .unwrap();
    assert_eq!(lines.len(), 0, "复用必须是干净的：{lines:?}");
}

/// **本地 clone 还没 fetch 到那条文档分支时，也要能找到它。**
///
/// `origin/<分支>` 是判"从哪建 worktree"的依据。本地那份远程跟踪 ref 是旧的话，
/// 会误判成"远程没有" → 从 `origin/main` 建一个空目录 → 接着凭空生成那 3 个文件。
#[test]
fn a_pushed_doc_branch_is_found_even_before_a_fetch() {
    let f = Fixture::new();
    let dh = f.dochub(); // 先克隆出主 clone（此刻它还不知道后面那条分支）

    let branch = "feature-20261019-pushed-later";
    let seed = f.hub.root().parent().unwrap().join("dochub-seed");
    let period = seed.join("workspaces/示例需求").join(branch);
    std::fs::create_dir_all(&period).unwrap();
    std::fs::write(period.join("提测单.md"), "# 提测\n").unwrap();
    PeriodMeta::new(branch, "示例需求", "推上来的那期")
        .save(&period.join(META_FILE))
        .unwrap();
    git(&seed, &["checkout", "-q", "-b", branch]);
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
            "docs",
        ],
    );
    git(&seed, &["push", "-q", "origin", branch]);
    git(&seed, &["checkout", "-q", "main"]);

    // 主 clone 此刻**还没 fetch**，`origin/<分支>` 在它眼里不存在
    let main_clone = f.hub.dochub_dir();
    assert!(
        !gws_core::git::Git::new(&main_clone).ref_exists(&format!("origin/{branch}")),
        "前提：本地确实还不知道这条分支"
    );

    let mut ws = f.create(branch, "示例需求");
    // 用的还是刚才那个 dh（没有先 ensure/fetch 一遍）
    let out = dh
        .attach_with(
            &mut ws,
            Some("示例需求"),
            gws_core::dochub::AttachMode::ReuseOnly,
        )
        .unwrap();

    assert!(
        out.attached,
        "fetch 一次就该发现这条分支，从而挂上远程那一期"
    );
    let docs = dh.docs_dir(&ws);
    assert!(
        docs.join("workspaces/示例需求")
            .join(branch)
            .join("提测单.md")
            .is_file(),
        "正文要拉下来"
    );
    let lines = gws_core::git::Git::new(&docs)
        .status_lines(true)
        .unwrap();
    assert_eq!(lines.len(), 0, "干净：{lines:?}");
}
