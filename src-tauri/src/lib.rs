//! GWS 桌面端。
//!
//! **直接调用 `gws-core`**，不 spawn 任何外部程序——这是相对开源版最重要
//! 的变化：旧版需要一个装在 PATH 里的 bash 脚本，Windows 上还得靠 Git Bash
//! 解释执行；本版把这些整类问题从根上消除（设计方案 §1.3）。

pub mod apps;
pub mod commands;
pub mod platform;

use tauri::Manager;

use commands::App;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例保护：Windows 上无此插件时每次点图标都会起新进程，
        // 多个实例同时操作一个 hub 会互相打架
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(App::default())
        .setup(|app| {
            #[cfg(debug_assertions)]
            let _ = app;
            // 装完 / 升级完桌面端，把同版本的 CLI 与 skills 一起带到本机（#7）。
            // 只在 release 构建里自动做：dev 下的 CLI 资源不存在，而且仓库里的
            // skill 可能正在改，不该在每次 `tauri dev` 时覆盖 ~/.claude。
            #[cfg(not(debug_assertions))]
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let out = commands::tools::sync_bundled(&handle);
                    if out.changed {
                        eprintln!("[gws] {}；{}", out.cli, out.skills);
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // hub
            commands::hub::locate_hub,
            commands::hub::open_hub,
            commands::hub::init_hub,
            commands::hub::hub_info_of,
            commands::hub::close_hub,
            commands::hub::hub_rename,
            commands::hub::hub_set_quick_links,
            commands::hub::hub_set_identity,
            commands::hub::global_git_identity,
            // 仓库
            commands::repo::repo_list,
            commands::repo::repo_add,
            commands::repo::repo_remove,
            // 环境
            commands::repo::env_list,
            commands::repo::env_add,
            commands::repo::env_update,
            commands::repo::env_remove,
            commands::repo::env_set_overrides,
            commands::repo::env_check_branches,
            // 工作区
            commands::ws::ws_list,
            commands::ws::ws_baseline,
            commands::ws::ws_new,
            commands::ws::ws_get,
            commands::ws::ws_shell,
            commands::ws::ws_detail,
            commands::ws::ws_add_modules,
            commands::ws::ws_drop_module,
            commands::ws::ws_drop_missing,
            commands::ws::ws_remove,
            commands::ws::ws_clean,
            commands::ws::ws_patch_meta,
            commands::ws::ws_share,
            commands::ws::ws_share_parse,
            commands::ws::ws_share_import,
            commands::deploy::update_install,
            commands::deploy::restart_app,
            commands::deploy::repo_share,
            commands::deploy::repo_share_parse,
            commands::deploy::repo_share_import,
            // 同步与合并
            commands::sync::ws_push,
            commands::sync::ws_pull,
            commands::sync::ws_merge,
            commands::sync::ws_sync_main,
            commands::sync::env_push,
            commands::sync::env_merge_check,
            commands::sync::ws_done,
            commands::sync::sync_all,
            commands::sync::sync_repos,
            commands::sync::sync_env,
            commands::sync::env_branch_of,
            commands::sync::ws_probe_remote,
            // MR
            commands::mr::mr_defaults,
            commands::mr::mr_plan,
            commands::mr::mr_create,
            commands::mr::mr_close,
            commands::mr::mr_reviewers,
            commands::mr::mr_save_reviewers,
            // Paseo（在 hub 根目录开 AI 会话）
            commands::paseo::paseo_available,
            commands::paseo::paseo_providers,
            commands::paseo::paseo_open_session,
            commands::paseo::paseo_open_free_session,
            // 文档
            commands::doc::doc_view,
            commands::doc::doc_attach,
            commands::doc::doc_detach,
            commands::doc::doc_types,
            commands::doc::doc_new,
            commands::doc::doc_status,
            commands::doc::doc_sync,
            commands::doc::doc_ahead_of_main,
            commands::doc::doc_set_label,
            commands::doc::doc_families,
            commands::doc::dochub_ensure,
            // 危险操作（都是 plan + apply 两步）
            commands::danger::ws_revert_plan,
            commands::danger::ws_revert_apply,
            commands::danger::ws_reset_plan,
            commands::danger::ws_reset_apply,
            commands::danger::branch_prune_plan,
            commands::danger::branch_prune_apply,
            commands::danger::branch_rename_plan,
            commands::danger::branch_rename_apply,
            // 文件浏览与对比
            commands::danger::browse_changes,
            commands::danger::browse_read,
            commands::danger::browse_ahead_behind,
            commands::danger::browse_commits,
            commands::danger::browse_commit_files,
            commands::danger::browse_commit_diff,
            commands::danger::browse_diff,
            // 发布地址
            commands::deploy::deploy_matrix,
            commands::deploy::deploy_set_address,
            commands::deploy::deploy_set_gitlab,
            commands::deploy::deploy_set_repo_info,
            commands::deploy::deploy_set_many,
            commands::deploy::check_update,
            commands::deploy::open_releases,
            commands::tools::tool_versions,
            commands::tools::sync_tools,
            // GitLab
            commands::gitlab::gitlab_status,
            commands::gitlab::gitlab_save_token,
            commands::gitlab::gitlab_login,
            commands::gitlab::gitlab_logout,
            commands::gitlab::gitlab_search_projects,
            commands::gitlab::gitlab_search_users,
            commands::gitlab::gitlab_clone_projects,
            commands::gitlab::gitlab_probe,
            commands::gitlab::hub_bind_gitlab,
            // 平台能力
            platform::open_in_finder,
            platform::open_path,
            platform::open_url,
            platform::copy_text,
            platform::open_in_terminal,
            platform::terminal_options,
            platform::open_in_editor,
            platform::editor_options,
            platform::editor_programs_missing,
            platform::program_picker_default,
            apps::installed_apps,
            platform::read_text_file,
            platform::list_dir,
            platform::list_entries,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
