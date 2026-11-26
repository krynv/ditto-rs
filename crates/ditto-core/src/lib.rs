//! Platform-independent core of Ditto-RS, a cross-platform re-creation of the
//! [Ditto](https://github.com/sabrogden/Ditto) clipboard manager.
//!
//! * [`model`]: a captured clip ([`ClipContent`]) and the summaries shown in the UI.
//! * [`store`]: the SQLite history (ordering, sticky clips, groups, retention).
//! * [`search`]: Ditto's search syntax (`AND`/`OR`/`NOT`, quotes, `*`, regex, `/q` and `/f`).
//! * [`transform`]: the "Special Paste" text transforms.
//! * [`imaging`]: thumbnails and combining several images into one.
//! * [`settings`]: user options with Ditto's defaults.
//! * [`ditto_import`]: import of an existing Windows `Ditto.db`.

pub mod error;
pub mod hash;
pub mod imaging;
pub mod model;
pub mod search;
pub mod settings;
pub mod text;
pub mod transform;

#[cfg(feature = "store")]
pub mod ditto_import;
#[cfg(feature = "store")]
pub mod store;

pub use error::{Error, Result};
pub use model::{ClipContent, ClipFormat, ClipKind, ClipSummary, Group, formats};
pub use search::{CompiledSearch, SearchMode, SearchScopes, SearchSpec};
pub use settings::Settings;
pub use transform::TextTransform;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
