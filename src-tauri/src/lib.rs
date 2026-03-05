mod album_art;
mod commands;
mod dta;
mod stfs;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::open_folder,
            commands::get_song_details,
            commands::save_song,
            commands::get_thumbnail,
            commands::get_album_art,
            commands::search_album_art,
            commands::download_album_art,
            commands::get_yarg_score_info,
            commands::sync_yarg_scores,
            commands::get_song_scores,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
