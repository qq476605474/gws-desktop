//! GitLab 客户端的**只读**冒烟测试。
//!
//! 打**你自己**的 GitLab，但**只调读取接口**（`/user`、`/projects`、`/users`），
//! 不创建、不修改、不删除任何东西——可以随时安全重跑。
//!
//! # 怎么跑
//!
//! 需要三个环境变量；**缺任何一个，整组测试自动跳过**（不算失败），
//! 所以在没配这些的机器上跑 `cargo test` 也是安全的：
//!
//! ```bash
//! export GWS_TEST_GITLAB_URL=https://gitlab.example.com
//! export GWS_TEST_GITLAB_TOKEN=<一个有 api 权限的 PAT>
//! export GWS_TEST_PROJECT=group/repo   # 你有权限的任意仓库，`group/name` 形式
//! cargo test -p gws-core --test gitlab_live -- --ignored --test-threads=1
//! ```
//!
//! # 为什么这些测试默认不跑（`#[ignore]`）
//!
//! 它们依赖**外部服务**。放进默认的 `cargo test` 会有两个后果：
//!
//! 1. **卡住**：网络慢或不可达时，每个请求要等满 30s 超时，一次
//!    `cargo test` 能挂十几分钟——开发时这直接让人放弃跑测试。
//! 2. **假红**：网络抖动会被当成代码错误。
//!
//! 单测/集成测试的职责是「离线也能确定地验证逻辑」，外部依赖测试是另一回事。

use gws_core::gitlab::Client;

/// 读测试配置。缺任一环境变量就返回 `None`，调用方直接跳过。
fn setup() -> Option<(Client, String)> {
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let (Some(url), Some(token), Some(project)) = (
        get("GWS_TEST_GITLAB_URL"),
        get("GWS_TEST_GITLAB_TOKEN"),
        get("GWS_TEST_PROJECT"),
    ) else {
        eprintln!(
            "⚠ 未配置 GWS_TEST_GITLAB_URL / GWS_TEST_GITLAB_TOKEN / GWS_TEST_PROJECT，\
             跳过只读冒烟测试"
        );
        return None;
    };
    Some((
        Client::new(url, token).expect("有地址和 token 时应能建出客户端"),
        project,
    ))
}

/// 只要客户端（不需要指定仓库的测试用）。
fn client() -> Option<Client> {
    setup().map(|(c, _)| c)
}

#[test]
#[ignore = "依赖真实 GitLab；用 cargo test -- --ignored 单独跑"]
fn connects_and_identifies_the_current_user() {
    let Some(c) = client() else { return };

    let me = c.current_user().expect("GET /user 应成功");
    assert!(!me.username.is_empty(), "应返回当前用户: {me:?}");
    assert!(!me.name.is_empty(), "应返回显示名（用于 @ 提醒）: {me:?}");
    eprintln!("✓ 连接成功: {} ({})", me.name, me.username);
}

#[test]
#[ignore = "依赖真实 GitLab；用 cargo test -- --ignored 单独跑"]
fn searches_projects_the_user_can_access() {
    // 「从 GitLab 检索仓库」的基础：能列出「我有权限的仓库」。
    let Some(c) = client() else { return };

    let projects = c.search_projects("", 20).expect("GET /projects 应成功");
    assert!(!projects.is_empty(), "当前用户应至少能访问一个项目");
    for p in &projects {
        assert!(!p.path_with_namespace.is_empty(), "项目必须有路径: {p:?}");
        assert!(!p.clone_url().is_empty(), "项目必须有可 clone 的地址: {p:?}");
    }
    eprintln!("✓ 取到 {} 个项目，例如: {}", projects.len(), projects[0].path_with_namespace);
}

#[test]
#[ignore = "依赖真实 GitLab；用 cargo test -- --ignored 单独跑"]
fn reads_a_project_by_full_path() {
    // 路径必须整段百分号编码，否则会被路由成别的端点、返回 404
    let Some((c, project)) = setup() else { return };

    let p = c.project(&project).expect("按完整路径取项目应成功");
    assert_eq!(p.path_with_namespace, project);
    assert_eq!(p.name_tail(), project.rsplit('/').next().unwrap());
    eprintln!("✓ {} 主干={:?} web={}", p.path_with_namespace, p.default_branch, p.web_url);
}

#[test]
#[ignore = "依赖真实 GitLab；用 cargo test -- --ignored 单独跑"]
fn detached_branch_list_is_readable() {
    // 批量删远程分支 / 批量改分支名都依赖这个
    let Some((c, project)) = setup() else { return };

    let branches = c.branches(&project).expect("列分支应成功");
    eprintln!(
        "✓ {} 取到 {} 个分支，例如: {:?}",
        project,
        branches.len(),
        &branches[..branches.len().min(5)]
    );
}

#[test]
#[ignore = "依赖真实 GitLab；用 cargo test -- --ignored 单独跑"]
fn searches_users_by_keyword() {
    // 评审人下拉框的候选来源
    let Some(c) = client() else { return };

    let users = c.search_users("a", 10).expect("GET /users 应成功");
    eprintln!("✓ 关键词 a 命中 {} 个用户", users.len());
    for u in users.iter().take(3) {
        eprintln!("    {} ({})", u.name, u.username);
    }
}

#[test]
#[ignore = "依赖真实 GitLab；用 cargo test -- --ignored 单独跑"]
fn missing_user_reports_clearly_instead_of_silently() {
    let Some(c) = client() else { return };

    // 指定的 username 不存在时必须报错中止，绝不能静默用错人当评审人
    let err = c.user_by_username("这个用户肯定不存在-zzz").unwrap_err();
    assert!(err.to_string().contains("不存在"), "{err}");
}