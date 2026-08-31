pub mod gws_runner;
pub mod shell;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例保护：Windows 上无此插件时每次点击图标都会起新进程，
        // 多个实例同开一个 hub 会并行跑 gws 命令且界面互不同步
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
        .invoke_handler(tauri::generate_handler![
            gws_runner::run_gws,
            gws_runner::run_gws_stream,
            gws_runner::respond_confirm,
            gws_runner::replay_output,
            shell::open_in_finder,
            shell::open_in_terminal,
            shell::open_path,
            shell::copy_text,
            shell::terminal_options,
            shell::check_gws_installed,
            shell::latest_gws_version,
            shell::hub_exists,
            shell::read_text_file,
            shell::list_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
