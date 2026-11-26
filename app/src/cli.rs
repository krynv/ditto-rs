//! Command-line interface. Without arguments the GUI starts (or, if Ditto-RS is already
//! running, the existing instance opens its quick-paste window).

use std::path::PathBuf;

use ditto_core::ditto_import::{self, ImportOptions};
use ditto_core::store::{ListQuery, Store};
use tauri::AppHandle;

use crate::paths::Paths;
use crate::popup;

pub const USAGE: &str = "\
Ditto-RS — a cross-platform clipboard manager re-creating Ditto

USAGE:
    ditto-rs [OPTIONS]

GUI OPTIONS:
    --show              Open the quick-paste window (also forwarded to a running instance)
    --options           Open the options window
    --background        Start hidden in the tray (used when starting on login)
    --quit              Ask a running instance to exit

COMMANDS (no GUI):
    --list [N] [TEXT]   Print the N most recent clips (default 20), optionally filtered
    --import <FILE>     Import a Ditto database (Ditto.db) into the history
    --paths             Print where the history and settings are stored
    -V, --version       Print the version
    -h, --help          Print this help

ENVIRONMENT:
    DITTO_RS_DATA_DIR   Store data in this folder instead of the default location
    RUST_LOG            Log filter, e.g. RUST_LOG=debug

Under Wayland, global hotkeys cannot be registered by apps: bind `ditto-rs --show` to a
shortcut in your desktop's keyboard settings instead.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Gui { show: bool, options: bool, background: bool, quit: bool },
    Help,
    Version,
    List { limit: usize, query: Option<String> },
    Import(PathBuf),
    Paths,
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    let (mut show, mut options, mut background, mut quit) = (false, false, false, false);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--paths" => return Ok(Command::Paths),
            "--import" => {
                let path = it.next().ok_or("--import needs the path of a Ditto.db file")?;
                return Ok(Command::Import(PathBuf::from(path)));
            }
            "--list" => {
                let mut limit = 20;
                let mut query: Option<String> = None;
                for rest in it.by_ref() {
                    match rest.parse::<usize>() {
                        Ok(n) if query.is_none() => limit = n,
                        _ => query = Some(query.map_or_else(|| rest.clone(), |q| format!("{q} {rest}"))),
                    }
                }
                return Ok(Command::List { limit, query });
            }
            "--show" => show = true,
            "--options" => options = true,
            "--background" | "--minimized" => background = true,
            "--quit" => quit = true,
            // macOS adds a process serial number when launched from Finder.
            other if other.starts_with("-psn_") => {}
            other => return Err(format!("unknown argument: {other}\n\n{USAGE}")),
        }
    }
    Ok(Command::Gui { show, options, background, quit })
}

/// Run a non-GUI command; returns the process exit code.
pub fn run(cmd: Command, paths: &Paths) -> i32 {
    ditto_platform::attach_parent_console();
    match cmd {
        Command::Help => {
            println!("{USAGE}");
            0
        }
        Command::Version => {
            println!("ditto-rs {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Command::Paths => {
            println!("data folder: {}", paths.data_dir.display());
            println!("history:     {}", paths.db.display());
            println!("settings:    {}", paths.settings.display());
            println!("portable:    {}", paths.portable);
            0
        }
        Command::List { limit, query } => match list(paths, limit, query.as_deref()) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        Command::Import(file) => {
            let result = Store::open(&paths.db).map_err(|e| e.to_string()).and_then(|mut store| {
                ditto_import::import_ditto_db(&file, &mut store, &ImportOptions::default()).map_err(|e| e.to_string())
            });
            match result {
                Ok(r) => {
                    println!(
                        "imported {} clip(s) and {} group(s); skipped {} duplicate(s) and {} unsupported clip(s)",
                        r.clips_imported, r.groups_imported, r.duplicates_skipped, r.unsupported_skipped
                    );
                    for e in &r.errors {
                        eprintln!("warning: {e}");
                    }
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
        Command::Gui { .. } => 0,
    }
}

fn list(paths: &Paths, limit: usize, query: Option<&str>) -> Result<(), String> {
    let store = Store::open(&paths.db).map_err(|e| e.to_string())?;
    let settings = ditto_core::Settings::load(&paths.settings);
    let spec = ditto_core::SearchSpec {
        query: query.unwrap_or_default().to_string(),
        mode: settings.search_mode,
        scopes: settings.search_scopes,
    };
    let search = ditto_core::CompiledSearch::compile(&spec).map_err(|e| e.to_string())?;
    let page = store
        .list(&ListQuery { include_grouped: true, search: search.as_ref(), limit: limit.max(1), ..Default::default() })
        .map_err(|e| e.to_string())?;
    for (i, clip) in page.items.iter().enumerate() {
        let first_line = clip.description.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        let short = ditto_core::text::truncate_chars(first_line, 90);
        let marker = if clip.sticky { "*" } else { " " };
        println!("{:>3}{marker} [{}] {short}", i + 1, clip.kind.as_str());
    }
    if page.items.is_empty() {
        println!("(no clips)");
    }
    Ok(())
}

/// Arguments forwarded by a second launch (single-instance plugin). The plugin calls this
/// from a background thread, so the work is moved to the main thread.
pub fn handle_forwarded(app: &AppHandle, argv: &[String]) {
    let command = parse(argv.get(1..).unwrap_or_default());
    let handle = app.clone();
    let result = app.run_on_main_thread(move || match command {
        Ok(Command::Gui { quit: true, .. }) => handle.exit(0),
        Ok(Command::Gui { options: true, .. }) => popup::open_settings(&handle),
        // Launching Ditto again (or `--show`) opens the quick-paste window.
        Ok(Command::Gui { background: false, .. }) => popup::show(&handle),
        _ => {}
    });
    if let Err(e) = result {
        log::warn!("could not handle a second launch: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Command, String> {
        parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn parses_arguments() {
        assert_eq!(p(&[]).unwrap(), Command::Gui { show: false, options: false, background: false, quit: false });
        assert_eq!(p(&["--show", "-psn_0_123"]).unwrap(), Command::Gui { show: true, options: false, background: false, quit: false });
        assert_eq!(p(&["--list"]).unwrap(), Command::List { limit: 20, query: None });
        assert_eq!(p(&["--list", "5", "foo", "bar"]).unwrap(), Command::List { limit: 5, query: Some("foo bar".into()) });
        assert_eq!(p(&["--import", "/x/Ditto.db"]).unwrap(), Command::Import("/x/Ditto.db".into()));
        assert!(p(&["--import"]).is_err());
        assert!(p(&["--bogus"]).is_err());
        assert_eq!(p(&["--help", "--bogus"]).unwrap(), Command::Help);
    }
}
