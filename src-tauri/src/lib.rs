pub mod gws_runner;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            gws_runner::run_gws,
            gws_runner::run_gws_stream,
            gws_runner::respond_confirm,
            gws_runner::replay_output,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
