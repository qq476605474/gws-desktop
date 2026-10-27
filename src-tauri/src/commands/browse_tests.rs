//! 文件浏览 / 代码对比的**后端自测**。
//!
//! # 为什么专门写这一组
//!
//! 这块功能反复出问题，而且每次都是「编译过、类型检查过、跑起来才发现」：
//! - diff 的两个参数被拼成一个字符串（`"origin/dev HEAD"`）→ git 报用法说明
//! - 三点 diff 在本地**落后**时结果为空 → 界面说「没有变化」，实际差几千个提交
//! - 基准 ref 不存在时把 git 的报错原文丢给用户
//!
//! 这些都是**能在真实仓库上跑出来的**，所以就在这里钉住 —— 而不是等人在界面上撞。

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use gws_core::hub::{Hub, InitOptions};

use crate::commands::danger::{
    browse_changes_in, browse_commit_files_in, browse_commits_in, browse_diff_in, browse_read_in,
    resolved_inside,
};

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

/// 造一个 hub，里面一个仓库 `demo`：
/// - `main`：主干
/// - `origin/dev`：比主干多一个提交（模拟"环境分支在远程走到了前面"）
/// - 本地 `main` 落后 `origin/dev`（就是用户遇到的"本地落后"场景）
fn fixture() -> (TempDir, Hub, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();

    // 裸仓库当 origin
    let bare = root.join("demo.git");
    git(
        &root,
        &["init", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    let seed = root.join("seed");
    git(&root, &["init", "-b", "main", seed.to_str().unwrap()]);
    std::fs::write(seed.join("README.md"), "v1\n").unwrap();
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

    // dev 分支：多一个提交（改 README + 加一个文件），推到远程
    git(&seed, &["checkout", "-q", "-b", "dev"]);
    std::fs::write(seed.join("README.md"), "v1\ndev only\n").unwrap();
    std::fs::write(seed.join("dev.txt"), "dev\n").unwrap();
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
            "feat: dev 上的改动",
        ],
    );
    git(&seed, &["push", "-q", "origin", "dev"]);
    git(&seed, &["checkout", "-q", "main"]);

    let mut hub = Hub::init(&root.join("hub"), InitOptions::default()).unwrap();
    // 单测不联网：直接写 GitLab 绑定（`repo::add` 要求 hub 已绑定）。
    // 这里的仓库是本地路径，`check_remote` 一律放行，不会真去连这个地址。
    hub.config_mut().gitlab.base_url = "https://gitlab.example.com".into();
    hub.save_config().unwrap();
    gws_core::repo::add(&hub, &[bare.to_string_lossy().to_string()], None).unwrap();
    let repo_dir = hub.repos_dir().join("demo");
    // 让本地 clone 拿到 dev 的远程跟踪分支
    git(&repo_dir, &["fetch", "-q", "origin"]);
    (tmp, hub, repo_dir)
}

fn dir_str(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

/// **核心场景**：本地落后于基准时，`与远程对比` 必须仍能列出差异。
///
/// 用户实测：`demo-app` 里本地落后 origin/dev 七千多个提交，
/// 界面却说「没有变化」—— 因为三点 diff 取的是"合并基之后的改动"，
/// 落后时合并基就是 HEAD 自己，必然为空。
#[test]
fn diff_against_a_branch_that_is_ahead_of_us_still_lists_files() {
    let (_t, hub, repo) = fixture();
    let base = "origin/dev";

    // 先确认前提：本地确实**落后**（而不是"两边一样"或"branch 不存在"另一种情况）。
    // `rev-list --left-right --count A...B` 输出 `左 右`：左 = A 有而 B 没有的提交数，
    // 也就是「落后」数；右 = 领先数。
    let counts = git(
        &repo,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{base}...HEAD"),
        ],
    );
    let left: u64 = counts.split_whitespace().next().unwrap().parse().unwrap();
    let right: u64 = counts.split_whitespace().nth(1).unwrap().parse().unwrap();
    assert!(
        left > 0 && right == 0,
        "前提：本地应落后于 {base}，实测 {counts}"
    );

    let out = browse_changes_in(&hub, &dir_str(&repo), "demo", Some(base.to_string()))
        .expect("落后时也应该能列出差异，而不是报错或空");
    let paths: Vec<&str> = out.iter().map(|c| c.path.as_str()).collect();
    assert!(
        paths.contains(&"dev.txt"),
        "应列出 dev 上的新文件：{paths:?}"
    );
    assert!(paths.contains(&"README.md"), "应列出被改的文件：{paths:?}");
    // 状态字母是「**从基准到本地**」的变化：
    //   A = 本地新增（基准没有） / D = 本地没有而基准有（别人加了，我这边还没有）
    let dev_txt = out.iter().find(|c| c.path == "dev.txt").unwrap();
    assert_eq!(
        dev_txt.status, "D",
        "dev.txt 在基准有、本地没有 → 本地视角是「删除」（别人加了）"
    );
    let readme = out.iter().find(|c| c.path == "README.md").unwrap();
    assert_eq!(readme.status, "M", "两边都有但内容不同 → 修改");
}

/// **本次实测反馈**：本地改了一堆文件（都还没提交），「与主干」却显示 0 个。
///
/// 根因：原来比的是 `base HEAD` —— 只比**已提交的树**。改动全在工作区里时，
/// 提交层面 HEAD 与 origin/main 一模一样，于是「没有任何变化」。
/// 「本地代码跟主干代码的差异」必须把工作区算进来。
#[test]
fn uncommitted_changes_show_up_against_the_trunk() {
    let (_t, hub, repo) = fixture();
    // 在主干镜像上直接改两个文件（一个改、一个新加），**都不提交**
    std::fs::write(repo.join("README.md"), "v1\n本地改的\n").unwrap();
    std::fs::write(repo.join("brand-new.txt"), "新写的\n").unwrap();
    git(&repo, &["add", "README.md"]); // 一个 add 了、一个没 add —— 两种都要算

    let out = browse_changes_in(&hub, &dir_str(&repo), "demo", None).expect("要能列出改动");
    let paths: Vec<&str> = out.iter().map(|c| c.path.as_str()).collect();
    assert!(
        paths.contains(&"README.md"),
        "改过的文件要列出来：{paths:?}"
    );
    assert!(
        paths.contains(&"brand-new.txt"),
        "没 add 的新文件也要列出来：{paths:?}"
    );
    assert_eq!(
        out.iter()
            .find(|c| c.path == "brand-new.txt")
            .unwrap()
            .status,
        "A",
        "新文件算「新增」"
    );

    // 单文件 diff 也要给出内容，而不是空白
    let d = browse_diff_in(&hub, &dir_str(&repo), "demo", "README.md", None).unwrap();
    assert!(
        d.diff.contains("本地改的"),
        "diff 里要有本地改的那行：{}",
        d.diff
    );
    let new_file = browse_diff_in(&hub, &dir_str(&repo), "demo", "brand-new.txt", None).unwrap();
    assert!(
        new_file.diff.contains("+新写的"),
        "没跟踪的新文件要合成一份「整文件新增」的 diff：{}",
        new_file.diff
    );
}

/// **实测反馈**：需求分支还没推送时，「与远程」的 `origin/<分支>` 不存在，
/// 界面上只剩一句报错 —— 本地明明改了东西却什么都看不到。
///
/// 默认目标「与本地」拿 `HEAD` 当基准：不依赖任何远程分支，未提交的改动
/// （改过的、没 add 的新文件）都要列出来。这里造一条**从未推送**的本地分支来钉住。
#[test]
fn head_is_a_valid_base_even_when_the_branch_was_never_pushed() {
    let (_t, hub, repo) = fixture();
    git(&repo, &["checkout", "-q", "-b", "feature-local-only"]);
    // 前提：远程确实没有这条分支（否则这条用例就没测到点子上）
    let exists = Command::new("git")
        .current_dir(&repo)
        .args(["rev-parse", "--verify", "-q", "origin/feature-local-only"])
        .output()
        .unwrap()
        .status
        .success();
    assert!(!exists, "前提：这条分支不该在远程上");

    std::fs::write(repo.join("README.md"), "v1\n本地改的\n").unwrap();
    std::fs::write(repo.join("brand-new.txt"), "新写的\n").unwrap();
    git(&repo, &["add", "README.md"]); // 一个 add 了、一个没 add —— 两种都要算

    let out = browse_changes_in(&hub, &dir_str(&repo), "demo", Some("HEAD".into()))
        .expect("「与本地」不该依赖远程分支");
    let paths: Vec<&str> = out.iter().map(|c| c.path.as_str()).collect();
    assert!(
        paths.contains(&"README.md"),
        "改过的文件要列出来：{paths:?}"
    );
    assert!(
        paths.contains(&"brand-new.txt"),
        "没 add 的新文件也要列出来：{paths:?}"
    );

    let d = browse_diff_in(
        &hub,
        &dir_str(&repo),
        "demo",
        "README.md",
        Some("HEAD".into()),
    )
    .expect("单文件 diff 也要能跑通");
    assert!(
        d.diff.contains("本地改的"),
        "diff 里要有本地改的那行：{}",
        d.diff
    );
}

/// 基准 ref 不存在 → 给一句能看懂的话，**不是** git 的用法说明。
#[test]
fn a_missing_base_gives_an_actionable_message() {
    let (_t, hub, repo) = fixture();
    let err = browse_changes_in(&hub, &dir_str(&repo), "demo", Some("origin/nope".into()))
        .expect_err("不存在的基准必须报错，而不是悄悄返回空");
    assert!(
        err.message.contains("nope"),
        "要点名是哪个分支：{}",
        err.message
    );
    assert!(
        !err.message.contains("git <命令>") && !err.message.contains("--version"),
        "不能把 git 的用法说明原文丢出来：{}",
        err.message
    );
}

/// 不传基准 → 探测主干，且必须真的能跑通（不再是拼错的单个参数）。
#[test]
fn default_base_is_detected_and_works() {
    let (_t, hub, repo) = fixture();
    let out = browse_changes_in(&hub, &dir_str(&repo), "demo", None).expect("按主干比对要能跑通");
    // 本地 main 与 origin/main 一致 → 没有差异
    assert!(out.is_empty(), "主干镜像应无差异，实测 {out:?}");
}

/// 单个文件的 diff 也要能跑通（同样不能被拼成一个参数）。
#[test]
fn per_file_diff_works() {
    let (_t, hub, repo) = fixture();
    let d = browse_diff_in(
        &hub,
        &dir_str(&repo),
        "demo",
        "dev.txt",
        Some("origin/dev".into()),
    )
        .expect("单文件 diff 要能跑通");
    assert!(
        d.diff.contains("dev.txt"),
        "diff 里应有这个文件：{}",
        d.diff
    );
    assert!(!d.diff.contains("usage:"), "不该是 git 的用法输出");
}

/// 提交历史：时间 / 作者 / 说明都要有（界面就显示这三样）。
#[test]
fn history_carries_author_date_and_subject() {
    let (_t, hub, repo) = fixture();
    // 镜像默认停在主干，而那条提交在 dev 上 —— 切过去才看得到
    git(&repo, &["checkout", "-q", "dev"]);
    let log = browse_commits_in(&hub, &dir_str(&repo), Some(5)).expect("列提交不该失败");
    assert!(!log.is_empty());
    let c = &log[0];
    assert_eq!(
        c.subject, "feat: dev 上的改动",
        "按时间倒序，最新那条在最前"
    );
    assert_eq!(c.author, "t", "要有作者");
    assert!(
        c.date.contains('-') && c.date.contains(':'),
        "日期要可读：{}",
        c.date
    );
}

/// 点一条提交 → 看它的文件变动。
#[test]
fn a_commit_lists_its_own_files() {
    let (_t, hub, repo) = fixture();
    git(&repo, &["checkout", "-q", "dev"]);
    let log = browse_commits_in(&hub, &dir_str(&repo), Some(5)).unwrap();
    let sha = log[0].sha.clone();
    let files =
        browse_commit_files_in(&hub, &dir_str(&repo), &sha).expect("列某提交的文件不该失败");
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"dev.txt"), "{paths:?}");
    assert!(paths.contains(&"README.md"), "{paths:?}");
}

/// 目录不在 hub 内 → 拒绝（路径穿越防护）。
#[test]
fn directories_outside_the_hub_are_rejected() {
    let (_t, hub, _repo) = fixture();
    let err = browse_changes_in(&hub, "/etc", "demo", None).expect_err("hub 之外的目录要拒绝");
    assert!(err.message.contains("不在 hub"), "{}", err.message);
}

/// **Windows 的逐字路径必须仍被判为「在目录内」。**
///
/// 实测反馈：对比代码里点任何一项都报「路径越界」。真因是 `browse_read` 内部
/// 那次 `canonicalize` 忘了剥 `\\?\` —— Windows 上它返回逐字路径（`\\?\D:\…`），
/// 与剥过的 `dir` 一比 `starts_with` 必然为假，**只要文件真的存在就必然报错**。
///
/// 这个用例**在 mac / Linux 上也能红**：喂进去的是一对纯字符串路径，
/// 剥前缀这一步去掉了它就会失败 —— 不必等人在 Windows 上再撞第三次。
#[test]
fn verbatim_windows_paths_are_still_inside() {
    // 用**本平台形态**的路径喂进去：判据是「剥掉 `\\?\` 之后前缀仍然成立」，
    // 与平台无关；用 Unix 形态就能在 mac / Linux 上把这个坑钉住。
    let (dir, resolved) = if cfg!(windows) {
        (
            std::path::PathBuf::from(r"D:\gws\hub\ws\feature-x\doc-hub"),
            std::path::PathBuf::from(r"\\?\D:\gws\hub\ws\feature-x\doc-hub\a.md"),
        )
    } else {
        (
            std::path::PathBuf::from("/tmp/gws/hub/ws/feature-x/doc-hub"),
            std::path::PathBuf::from(r"\\?\/tmp/gws/hub/ws/feature-x/doc-hub/a.md"),
        )
    };
    assert!(
        resolved_inside(&dir, &resolved),
        "剥掉 `\\\\?\\` 之后必须判为在内，否则 Windows 上每次读文件都报路径越界"
    );

    // 反向：真的在外的仍然要在外（别为了修这个把防护拆了）
    let outside = if cfg!(windows) {
        std::path::PathBuf::from(r"\\?\D:\gws\other\a.md")
    } else {
        std::path::PathBuf::from(r"\\?\/tmp/gws/other/a.md")
    };
    let parent = dir.parent().unwrap().to_path_buf();
    assert!(!resolved_inside(&parent, &outside));
}

/// **未跟踪的目录要展开成里面的文件，而不是把目录本身当成一条「新增」。**
///
/// 实测反馈：对比代码里点某一条「新增」报错。那条其实是 `?? workspaces/` ——
/// 默认的 `git status` 会把整个未跟踪目录折成**一行目录名**，点开既没有内容、
/// 也不是文件。改成 `-uall` 展开之后，列表里只剩真实文件。
#[test]
fn untracked_directories_are_expanded_into_files() {
    let (_t, hub, repo) = fixture();
    // 造一个未跟踪的目录，里面两个文件（模拟"新写的一期文档"）
    let sub = repo.join("workspaces").join("示例").join("feature-x");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("meta.json"), "{}\n").unwrap();
    std::fs::write(sub.join("说明.md"), "内容\n").unwrap();

    let dir = dir_str(&repo);
    let out = browse_changes_in(&hub, &dir, "demo", Some("origin/main".into())).expect("列改动");

    let paths: Vec<&str> = out.iter().map(|f| f.path.as_str()).collect();
    assert!(
        paths.iter().all(|p| !p.ends_with('/')),
        "列表里不该出现目录条目：{paths:?}"
    );
    assert_eq!(
        paths,
        vec![
            "workspaces/示例/feature-x/meta.json",
            "workspaces/示例/feature-x/说明.md"
        ],
        "未跟踪目录里的文件要逐条列出来"
    );

    // 文件读得出来；而**直接点目录**要给一句人话，不是「路径越界」
    let body = browse_read_in(&hub, &dir, "workspaces/示例/feature-x/meta.json").expect("读新文件");
    assert_eq!(body, "{}\n");
    let err = browse_read_in(&hub, &dir, "workspaces/示例/feature-x").expect_err("目录读不出内容");
    assert!(err.message.contains("是个目录"), "{}", err.message);
    assert!(
        !err.message.contains("越界"),
        "不该报路径越界：{}",
        err.message
    );
}

/// 真穿越仍然要挡（改动列表与读文件两条路）。
#[test]
fn traversal_is_still_rejected_on_read() {
    let (_t, hub, repo) = fixture();
    let dir = dir_str(&repo);
    let err = browse_read_in(&hub, &dir, "../../../etc/hosts").expect_err("穿越要拒绝");
    assert!(err.message.contains("越界"), "{}", err.message);
}
/// **拿真实仓库跑一遍**（默认跳过：要联网）。
///
/// 用你自己指定的真实仓库，只读、不推不改分支。
/// 合成夹具能验语义，但真实仓库才会暴露「分支名奇怪 / 主干不是 main /
/// 远程跟踪分支不齐」这类问题 —— 而这些正是这块功能此前翻车的地方。
///
/// 跑法：`cargo test -p gws-desktop --lib real_repo -- --ignored --nocapture`
#[test]
#[ignore = "需要真实 GitLab"]
fn real_repo_browse_end_to_end() {
    const URL: &str = "https://gitlab.example.com/demo-team/demo-log-tools.git";
    let tmp = TempDir::new().unwrap();
    let hub = Hub::init(&tmp.path().join("hub"), InitOptions::default()).unwrap();
    gws_core::repo::add(&hub, &[URL.to_string()], None).expect("clone 真实仓库");
    let repo = hub.repos_dir().join("demo-log-tools");

    // 主干探测：不能写死 main
    let main = gws_core::git::detect_main_branch(&repo).expect("探测主干");
    println!("真实仓库主干 = {main}");
    let dir = dir_str(&repo);

    // ① 按主干比：主干镜像停在主干上，应是空的（证明参数没拼错、命令合法）
    let same = browse_changes_in(
        &hub,
        &dir,
        "demo-log-tools",
        Some(format!("origin/{main}")),
    )
        .expect("按主干比对要能跑通");
    assert!(same.is_empty(), "主干镜像 vs 主干不该有差异：{same:?}");

    // ② 换一条**真实存在的**分支比：应列出文件，且带正确的状态字母
    git(&repo, &["checkout", "-q", "20260410-init"]);
    let out = browse_changes_in(
        &hub,
        &dir,
        "demo-log-tools",
        Some(format!("origin/{main}")),
    )
        .expect("与主干对比要能跑通");
    assert!(!out.is_empty(), "这条分支领先主干 26 个提交，必然有差异");
    println!(
        "与主干差异 {} 个文件，前 3 个：{:?}",
        out.len(),
        &out[..3.min(out.len())]
    );

    // ③ 提交历史：真实提交要有作者/时间/说明
    let log = browse_commits_in(&hub, &dir, Some(5)).expect("列提交");
    assert!(!log.is_empty());
    for c in &log {
        assert!(
            !c.sha.is_empty()
                && !c.author.is_empty()
                && !c.date.is_empty()
                && !c.subject.is_empty(),
            "提交的四要素都不能空：{c:?}"
        );
    }
    println!(
        "最近提交：{} · {} · {}",
        log[0].sha, log[0].date, log[0].subject
    );

    // ④ 点第一条提交看它的文件变动
    let files = browse_commit_files_in(&hub, &dir, &log[0].sha).expect("列某提交的文件");
    println!("最新提交改了 {} 个文件", files.len());

    // ⑤ 不存在的分支 → 给人话
    let err = browse_changes_in(
        &hub,
        &dir,
        "demo-log-tools",
        Some("origin/根本没有这条".into()),
    )
        .expect_err("不存在的分支要报错");
    assert!(err.message.contains("根本没有这条"), "{}", err.message);
    assert!(
        !err.message.contains("git <命令>"),
        "不该是 git 的用法说明：{}",
        err.message
    );
}
