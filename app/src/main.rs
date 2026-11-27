// Release builds on Windows should not open a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod cli;
mod commands;
mod hotkeys;
mod paste;
mod paths;
mod popup;
mod state;
mod tray;

use ditto_core::settings::Settings;
use ditto_core::store::Store;
use tauri::Manager;
use tauri_plugin_autostart::MacosLauncher;

use crate::cli::Command;
use crate::paths::Paths;
use crate::state::AppState;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths = Paths::resolve();
    match cli::parse(&args) {
        // Always start through Tauri: if Ditto-RS is already running, the single-instance
        // plugin forwards these flags to it and this process exits.
        Ok(Command::Gui { show, options, background, quit }) => run_gui(paths, show || !background, options, quit),
        Ok(cmd) => std::process::exit(cli::run(cmd, &paths)),
        Err(e) => {
            ditto_platform::attach_parent_console();
            eprintln!("{e}");
            std::process::exit(2);
        }
    }
}

fn load_settings(paths: &Paths) -> Settings {
    let first_run = !paths.settings.exists();
    let mut settings = Settings::load(&paths.settings);
    if first_run {
        if paths.portable {
            // Ditto's portable build keeps fewer clips by default.
            settings.max_entries = 100;
        }
        if let Err(e) = settings.save(&paths.settings) {
            log::warn!("could not write {}: {e}", paths.settings.display());
        }
    }
    settings
}

fn run_gui(paths: Paths, show_on_start: bool, open_options: bool, quit: bool) {
    log::info!("data folder: {}", paths.data_dir.display());
    let settings = load_settings(&paths);
    let store = match Store::open(&paths.db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot open the clip history {}: {e}", paths.db.display());
            std::process::exit(1);
        }
    };
    let state = AppState::new(paths, store, settings);

    tauri::Builder::default()
        // Must be the first plugin: a second launch forwards its arguments and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| cli::handle_forwarded(app, &argv)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--background"])))
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::list_clips,
            commands::clip_details,
            commands::clip_thumbnail,
            commands::paste_clips,
            commands::copy_clips,
            commands::text_transforms,
            commands::delete_clips,
            commands::update_clip_text,
            commands::new_clip,
            commands::update_clip_properties,
            commands::set_sticky,
            commands::move_clip,
            commands::list_groups,
            commands::create_group,
            commands::rename_group,
            commands::delete_group,
            commands::move_to_group,
            commands::get_settings,
            commands::save_settings,
            commands::reset_settings,
            commands::app_info,
            commands::stats,
            commands::compact_database,
            commands::backup_database,
            commands::clear_history,
            commands::import_ditto_database,
            commands::save_clip_to_file,
            commands::hide_popup,
            commands::open_settings,
            commands::context_menu_opening,
            commands::set_capture_paused,
            commands::toggle_keep_open,
            commands::save_current_clipboard,
            commands::request_paste_permission,
            commands::quit,
        ])
        .setup(move |app| {
            if quit {
                // `--quit` with no instance running: nothing to do.
                app.handle().exit(0);
                return Ok(());
            }
            // A tray utility: no Dock icon, no app menu bar on macOS.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            popup::create_popup(&handle)?;
            tray::create(&handle)?;
            if let Err(e) = hotkeys::init(&handle) {
                log::error!("global hotkeys are unavailable: {e}");
            }
            hotkeys::register_all(&handle);
            if let Err(e) = capture::start(&handle) {
                log::error!("clipboard monitoring is unavailable: {e}");
            }
            let settings = handle.state::<AppState>().settings();
            if let Err(e) = commands::apply_autostart(&handle, settings.start_on_login) {
                log::warn!("start on login: {e}");
            }
            if open_options {
                popup::open_settings(&handle);
            } else if show_on_start {
                popup::show(&handle);
            }
            Ok(())
        })
        .on_window_event(popup::on_window_event)
        .build(tauri::generate_context!())
        .expect("error while building the Ditto-RS application")
        .run(|_app, event| {
            // Stay in the tray when the options window closes; exit only via "Exit".
            if let tauri::RunEvent::ExitRequested { api, code: None, .. } = event {
                api.prevent_exit();
            }
        });
}
