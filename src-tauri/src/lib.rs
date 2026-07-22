mod album_art;
mod commands;
mod dta;
mod local_db;
mod midi;
mod rhythmverse;
mod scan_cache;
mod song_ini;
mod stfs;
mod updater;

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
            commands::reveal_in_explorer,
            commands::batch_decrypt_moggs,
            commands::find_duplicates,
            commands::delete_files,
            commands::preview_renames,
            commands::batch_rename,
            commands::batch_get_field,
            commands::preview_organize,
            commands::execute_organize,
            commands::batch_validate,
            commands::get_chart_overview,
            commands::get_chart_notes,
            rhythmverse::rv_browse,
            rhythmverse::rv_download,
            rhythmverse::rv_download_records,
            rhythmverse::rv_open_external,
            rhythmverse::rv_opened_ids,
            rhythmverse::rv_mark_opened,
            rhythmverse::rv_mark_downloaded,
            rhythmverse::rv_unmark_downloaded,
            rhythmverse::rv_link_song,
            rhythmverse::rv_unlink_song,
            rhythmverse::rv_linked_file_id,
            rhythmverse::rv_touch_downloaded,
            rhythmverse::rv_set_upload_baseline,
            updater::check_for_update,
            updater::download_and_apply_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
