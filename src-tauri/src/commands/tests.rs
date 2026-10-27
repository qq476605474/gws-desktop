//! 命令层回归测试。
//!
//! # 这个测试文件存在的唯一理由
//!
//! 曾经有一个 bug 逃过了全部 232 个测试、逃过了 clippy、逃过了手工验收，
//! 因为**它只在 `tauri dev`（debug 构建）+ Tauri 的 async 运行时里才出现**：
//!
//! `reqwest::blocking` 在 debug 下会嵌套新建并立刻销毁一个 tokio runtime，
//! 而 `#[tauri::command(async)]` 的命令体已经在 tokio 运行时里 —— 于是 panic
//! `Cannot drop a runtime in a context where blocking is not allowed`。
//! 命令体炸在 spawned 任务里，前端 Promise 永不 settle，界面**永远转圈**。
//!
//! 这些测试把「联网命令必须能在 async 运行时里跑通」钉住：任何一个命令忘记走
//! `bg()`（回到把 `#[tauri::command(async)]` 直接加在同步函数上的写法），
//! 都会在这里红掉，而不是等到用户在界面上看到一个转不完的圈。

use crate::commands::{bg, gitlab};

/// 能在 `tauri::async_runtime` 里跑通——这正是 `tauri dev` 的运行环境。
///
/// 不可达 GitLab 时也**必须**返回 `Ok`（状态里带 error 字段），
/// 不能 panic、不能挂住：这个命令是设置页加载时就调的。
#[test]
fn gitlab_status_survives_the_async_runtime() {
    // 用「还没打开 hub」这条状态测（`State` 在单测里构造不出来，所以直接调
    // 命令体）：设置页与启动页都会走到这里，而且它同样经过 `bg()` ——
    // 正是本文件要钉的那条路径。
    let out = tauri::async_runtime::block_on(gitlab::status_impl(None));
    // 状态查询本身**总是 Ok**（连不上时把原因放进 error 字段），所以这里可以严格要求
    assert!(out.is_ok(), "命令不该失败：{:?}", out.err().map(|e| e.message));
}

/// 检查更新同理——它也是「打开设置页/点一下」就走的联网命令。
///
/// **断言的是「没 panic」，不是「必须成功」**：这条命令要打真实 GitLab，
/// 网络抖一下就会合理地失败。真正要钉的是「不会因为 panic 而卡死」——
/// 所以失败时只检查**不是**那条内部错误文案（panic 会被 `bg` 转成它）。
#[test]
fn check_update_survives_the_async_runtime() {
    let out = tauri::async_runtime::block_on(crate::commands::deploy::check_update());
    if let Err(e) = out {
        assert!(
            !e.message.contains("内部错误"),
            "不该是 panic 被兜住的那条：{}",
            e.message
        );
        eprintln!("（网络原因失败，不算错）{}", e.message);
    }
}

/// `bg` 里任何一个 panic 都要变成**一条错误**，而不是永久转圈。
///
/// 这是 `bg` 的第二道防线：即使将来冒出别的 panic 源，用户也只会看到
/// 一条报错，而不是对着遮罩点不动界面。
#[test]
fn a_panicking_task_becomes_an_error_not_a_hang() {
    let out = tauri::async_runtime::block_on(bg(|| -> Result<(), _> {
        panic!("模拟命令体里的意外 panic");
    }));
    let err = out.expect_err("panic 应该被转成错误");
    assert!(err.message.contains("内部错误"), "{}", err.message);
}

/// 顺带钉住：Releases 地址必须是**完整的、能直接打开的**地址。
///
/// 之前桌面端自己拼成 `{根地址}/-/releases`（少了项目路径），点开必然 404。
#[test]
fn releases_url_is_a_complete_github_url() {
    let url = gws_core::update::releases_url();
    assert!(url.starts_with("https://github.com/"), "{url}");
    assert!(url.contains(gws_core::update::REPO), "要带上 owner/repo：{url}");
    assert!(url.ends_with("/releases"), "{url}");
    assert!(!url.contains("//releases"), "拼串出错了：{url}");
}

/// **文档的 `rel` 路径必须一律用 `/` 分隔。**
///
/// 实测反馈：「Windows 包里点『需求相关文档』，弹出来的文档没有树形结构」。
/// 真因是 `Path::to_string_lossy()` 在 Windows 上给的是 `原型\index.html`，
/// 而前端按 `/` 切分拼树 —— 切不开，整份文档就平铺成一行、还带着反斜杠。
///
/// 这个用例**在 mac / Linux 上也能红**：喂的是 Windows 形态的字符串，
/// 把 `to_slash` 去掉就会失败 —— 不必等人在 Windows 上再撞一次。
#[test]
fn doc_rel_paths_use_forward_slashes_on_every_platform() {
    use crate::commands::doc::to_slash;
    assert_eq!(to_slash(r"原型\index.html"), "原型/index.html");
    assert_eq!(to_slash(r"sql\2026-10-16-频控参数加列.sql"), "sql/2026-10-16-频控参数加列.sql");
    assert_eq!(to_slash(r"a\b\c.md"), "a/b/c.md");
    // 已经是 `/` 的（mac / Linux）原样不动
    assert_eq!(to_slash("guides/existing.md"), "guides/existing.md");
    // 没有分隔符的也不该变
    assert_eq!(to_slash("meta.json"), "meta.json");
}
