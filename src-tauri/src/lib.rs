mod album_art;
mod catalog;
mod commands;
mod download;
mod dta;
mod duplicates;
mod filename_guess;
mod folder_template;
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
        .setup(|app| {
            catalog::start(app.handle().clone());
            #[cfg(windows)]
            if let Some(window) = tauri::Manager::get_webview_window(app, "main") {
                set_native_window_icons(&window);
                if let Ok(hwnd) = window.hwnd() {
                    set_title_bar_colors(hwnd.0 as _, true);
                }
            }
            Ok(())
        })
        .on_window_event(|_window, _event| {
            // A custom caption color stays put when the window loses focus, so
            // dim the title text instead to keep the usual inactive cue.
            #[cfg(windows)]
            if let tauri::WindowEvent::Focused(focused) = _event {
                if let Ok(hwnd) = _window.hwnd() {
                    set_title_bar_colors(hwnd.0 as _, *focused);
                }
            }
        })
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
            commands::path_is_dir,
            commands::batch_decrypt_moggs,
            duplicates::find_duplicates,
            commands::delete_files,
            commands::preview_renames,
            commands::batch_rename,
            commands::batch_get_field,
            commands::preview_organize,
            commands::render_organize_template,
            commands::execute_organize,
            commands::batch_validate,
            commands::get_chart_overview,
            commands::get_chart_notes,
            rhythmverse::rv_browse,
            rhythmverse::rv_download,
            rhythmverse::rv_replace_broken,
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
            catalog::catalog_status,
            catalog::catalog_sync_now,
            catalog::catalog_set_owned,
            catalog::catalog_query,
            catalog::catalog_facets,
            catalog::catalog_suggest,
            updater::check_for_update,
            updater::download_and_apply_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Tauri only sets a window's small icon, from the first `.ico` entry, so the
/// taskbar scales that one image up or down. Instead hand Windows both icons
/// from the exe's embedded `.ico` (resource 32512, see tauri-build) at the
/// window's DPI, so it picks the matching hand-tuned layer itself.
#[cfg(windows)]
fn set_native_window_icons(window: &tauri::WebviewWindow) {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        LoadImageW, SendMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, SM_CXICON,
        SM_CXSMICON, WM_SETICON,
    };

    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = hwnd.0 as windows_sys::Win32::Foundation::HWND;
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        let dpi = GetDpiForWindow(hwnd);
        for (kind, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            let px = GetSystemMetricsForDpi(metric, dpi);
            let icon = LoadImageW(module, 32512 as _, IMAGE_ICON, px, px, LR_DEFAULTCOLOR);
            if !icon.is_null() {
                SendMessageW(hwnd, WM_SETICON, kind as usize, icon as isize);
            }
        }
    }
}

/// Color the native title bar to match the app icon's navy tile (`#0E1626`,
/// also the left panel's `--bg-secondary`) so the icon blends in instead of
/// sitting on Windows' gray caption, with muted title text while inactive.
/// Windows 11 only; Windows 10 rejects these attributes and keeps its default
/// title bar, so errors are ignored.
#[cfg(windows)]
fn set_title_bar_colors(hwnd: windows_sys::Win32::Foundation::HWND, focused: bool) {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
    };

    // COLORREF is 0x00BBGGRR.
    const CAPTION: u32 = 0x0026_160E; // #0E1626
    const TEXT: u32 = 0x00FF_E0C7; // #C7E0FF (--text-header)
    const TEXT_INACTIVE: u32 = 0x0099_7D6B; // #6B7D99
    const BORDER: u32 = 0x0047_2E1F; // #1F2E47 (--border)

    let text = if focused { TEXT } else { TEXT_INACTIVE };
    for (attr, color) in [
        (DWMWA_CAPTION_COLOR, CAPTION),
        (DWMWA_TEXT_COLOR, text),
        (DWMWA_BORDER_COLOR, BORDER),
    ] {
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                attr as u32,
                &color as *const u32 as *const _,
                std::mem::size_of::<u32>() as u32,
            );
        }
    }
}
