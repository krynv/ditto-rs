//! Where the history database and settings live.
//!
//! Like Ditto, a file named `portable` next to the executable switches to portable mode
//! (Ditto's portable zip ships exactly such an empty marker file): data is then kept in a
//! `Ditto-RS Data` folder beside the executable. `DITTO_RS_DATA_DIR` overrides everything.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub db: PathBuf,
    pub settings: PathBuf,
    pub portable: bool,
}

impl Paths {
    pub fn resolve() -> Paths {
        if let Some(dir) = std::env::var_os("DITTO_RS_DATA_DIR").filter(|d| !d.is_empty()) {
            return Paths::in_dir(PathBuf::from(dir), false);
        }
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        if let Some(dir) = exe_dir.filter(|d| d.join("portable").is_file()) {
            return Paths::in_dir(dir.join("Ditto-RS Data"), true);
        }
        let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("Ditto-RS");
        Paths::in_dir(base, false)
    }

    fn in_dir(dir: PathBuf, portable: bool) -> Paths {
        Paths { db: dir.join("history.db"), settings: dir.join("settings.json"), data_dir: dir, portable }
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }
}
