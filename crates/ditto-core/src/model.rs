//! The clip model.
//!
//! Ditto stores one row per copy in `Main` and one row per clipboard format in `Data`
//! (`CF_UNICODETEXT`, `HTML Format`, `Rich Text Format`, `CF_DIB`, `CF_HDROP`, ...).
//! We keep the same shape but with portable, canonical format names (see [`formats`]):
//! platform code converts to and from the native clipboard representations.

use serde::{Deserialize, Serialize};

use crate::hash::Fnv64;
use crate::text;

/// Canonical, platform-neutral format names.
pub mod formats {
    /// UTF-8 plain text (Ditto: `CF_UNICODETEXT` / `CF_TEXT`).
    pub const TEXT: &str = "text";
    /// UTF-8 HTML fragment (Ditto: `HTML Format`, without the CF_HTML header).
    pub const HTML: &str = "html";
    /// RTF document (Ditto: `Rich Text Format`).
    pub const RTF: &str = "rtf";
    /// PNG-encoded image (Ditto: `PNG` / `CF_DIB`).
    pub const PNG: &str = "png";
    /// Newline-separated absolute file paths (Ditto: `CF_HDROP`).
    pub const FILES: &str = "files";

    pub const ALL: [&str; 5] = [TEXT, HTML, RTF, PNG, FILES];
}

/// One clipboard format of a clip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipFormat {
    pub name: String,
    pub data: Vec<u8>,
}

/// Everything captured from one copy. Formats are kept sorted by name so that equal
/// content always compares and hashes equal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipContent {
    formats: Vec<ClipFormat>,
}

/// What a clip primarily is (drives how it is drawn in the list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipKind {
    Text,
    Image,
    Files,
    Other,
}

impl ClipKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ClipKind::Text => "text",
            ClipKind::Image => "image",
            ClipKind::Files => "files",
            ClipKind::Other => "other",
        }
    }

    pub fn parse(s: &str) -> ClipKind {
        match s {
            "text" => ClipKind::Text,
            "image" => ClipKind::Image,
            "files" => ClipKind::Files,
            _ => ClipKind::Other,
        }
    }
}

impl ClipContent {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_text(text: impl Into<String>) -> Self {
        let mut c = Self::new();
        c.set(formats::TEXT, text.into().into_bytes());
        c
    }

    pub fn from_png(png: Vec<u8>) -> Self {
        let mut c = Self::new();
        c.set(formats::PNG, png);
        c
    }

    pub fn from_files(files: &[String]) -> Self {
        let mut c = Self::new();
        c.set_files(files);
        c
    }

    /// Insert or replace a format.
    pub fn set(&mut self, name: &str, data: Vec<u8>) {
        match self.formats.binary_search_by(|f| f.name.as_str().cmp(name)) {
            Ok(i) => self.formats[i].data = data,
            Err(i) => self.formats.insert(i, ClipFormat { name: name.to_string(), data }),
        }
    }

    pub fn with(mut self, name: &str, data: Vec<u8>) -> Self {
        self.set(name, data);
        self
    }

    pub fn remove(&mut self, name: &str) -> Option<Vec<u8>> {
        let i = self.formats.binary_search_by(|f| f.name.as_str().cmp(name)).ok()?;
        Some(self.formats.remove(i).data)
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.formats
            .binary_search_by(|f| f.name.as_str().cmp(name))
            .ok()
            .map(|i| self.formats[i].data.as_slice())
    }

    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn formats(&self) -> &[ClipFormat] {
        &self.formats
    }

    pub fn format_names(&self) -> Vec<String> {
        self.formats.iter().map(|f| f.name.clone()).collect()
    }

    fn get_str(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(|b| std::str::from_utf8(b).ok())
    }

    pub fn text(&self) -> Option<&str> {
        self.get_str(formats::TEXT)
    }

    pub fn html(&self) -> Option<&str> {
        self.get_str(formats::HTML)
    }

    pub fn rtf(&self) -> Option<&str> {
        self.get_str(formats::RTF)
    }

    pub fn png(&self) -> Option<&[u8]> {
        self.get(formats::PNG)
    }

    pub fn files(&self) -> Vec<String> {
        self.get_str(formats::FILES)
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect())
            .unwrap_or_default()
    }

    pub fn set_files(&mut self, files: &[String]) {
        let joined = files.iter().map(|f| f.trim()).filter(|f| !f.is_empty()).collect::<Vec<_>>().join("\n");
        if joined.is_empty() {
            self.remove(formats::FILES);
        } else {
            self.set(formats::FILES, joined.into_bytes());
        }
    }

    /// True when there is nothing worth saving.
    pub fn is_empty(&self) -> bool {
        self.formats.iter().all(|f| f.data.is_empty())
    }

    pub fn total_size(&self) -> usize {
        self.formats.iter().map(|f| f.data.len()).sum()
    }

    pub fn kind(&self) -> ClipKind {
        if !self.files().is_empty() {
            ClipKind::Files
        } else if self.text().is_some_and(|t| !t.trim().is_empty()) {
            ClipKind::Text
        } else if self.png().is_some_and(|p| !p.is_empty()) {
            ClipKind::Image
        } else if self.html().is_some() || self.rtf().is_some() || self.text().is_some() {
            ClipKind::Text
        } else {
            ClipKind::Other
        }
    }

    /// Stable content hash used for de-duplication (Ditto: `CClip::GenerateCRC`).
    pub fn content_hash(&self) -> i64 {
        let mut h = Fnv64::new();
        for f in &self.formats {
            h.write(f.name.as_bytes());
            h.write(&[0]);
            h.write(&(f.data.len() as u64).to_le_bytes());
            h.write(&f.data);
        }
        h.finish() as i64
    }

    /// The best plain-text rendering of this clip, if it has any textual meaning.
    pub fn plain_text(&self) -> Option<String> {
        if let Some(t) = self.text() {
            return Some(t.to_string());
        }
        let files = self.files();
        if !files.is_empty() {
            return Some(files.join("\n"));
        }
        if let Some(h) = self.html() {
            return Some(text::html_to_text(h));
        }
        if let Some(r) = self.rtf() {
            return Some(text::rtf_to_text(r));
        }
        None
    }

    /// A copy that only carries plain text ("Paste Plain Text Only").
    pub fn text_only(&self) -> Option<ClipContent> {
        self.plain_text().map(ClipContent::from_text)
    }

    /// The text shown in the list (Ditto: `Main.mText`, `DescTextSize` = 500 by default).
    pub fn description(&self, max_chars: usize) -> String {
        let max_chars = max_chars.max(1);
        match self.kind() {
            ClipKind::Files => text::truncate_chars(&self.files().join("\n"), max_chars).to_string(),
            ClipKind::Image => match self.png().and_then(png_dimensions) {
                Some((w, h)) => format!("Image {w} × {h}"),
                None => "Image".to_string(),
            },
            ClipKind::Text => {
                let t = self.plain_text().unwrap_or_default();
                text::truncate_chars(&t, max_chars).to_string()
            }
            ClipKind::Other => self.format_names().join(", "),
        }
    }
}

/// Read width/height from a PNG's IHDR chunk without decoding the image.
pub fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if png.len() < 24 || png[..8] != SIG || &png[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(png[20..24].try_into().ok()?);
    Some((w, h))
}

/// One row of the list (no clip data).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClipSummary {
    pub id: i64,
    pub description: String,
    pub kind: ClipKind,
    /// Milliseconds since the Unix epoch.
    pub created_at: i64,
    /// Last time it was pasted (initially the creation time), ms since the epoch.
    pub last_used_at: i64,
    pub group_id: Option<i64>,
    /// Sticky in the main list (Ditto: `stickyClipOrder`).
    pub sticky: bool,
    /// Sticky inside its group (Ditto: `stickyClipGroupOrder`).
    pub group_sticky: bool,
    pub never_auto_delete: bool,
    pub quick_paste_text: Option<String>,
    /// Global hotkey that pastes this clip, e.g. `Ctrl+Shift+1`.
    pub hotkey: Option<String>,
    pub source_app: Option<String>,
    pub size_bytes: i64,
    pub formats: Vec<String>,
    pub has_thumbnail: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub clip_count: i64,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn formats_are_sorted_and_replaced() {
        let mut c = ClipContent::new();
        c.set(formats::TEXT, b"b".to_vec());
        c.set(formats::HTML, b"<b>b</b>".to_vec());
        c.set(formats::TEXT, b"a".to_vec());
        assert_eq!(c.format_names(), vec!["html", "text"]);
        assert_eq!(c.text(), Some("a"));
    }

    #[test]
    fn hash_is_order_independent_and_content_sensitive() {
        let a = ClipContent::new().with(formats::TEXT, b"x".to_vec()).with(formats::HTML, b"y".to_vec());
        let b = ClipContent::new().with(formats::HTML, b"y".to_vec()).with(formats::TEXT, b"x".to_vec());
        assert_eq!(a.content_hash(), b.content_hash());
        let c = ClipContent::from_text("x");
        assert_ne!(a.content_hash(), c.content_hash());
        // Moving bytes between formats must change the hash.
        let d = ClipContent::new().with(formats::TEXT, b"xy".to_vec()).with(formats::HTML, b"".to_vec());
        assert_ne!(a.content_hash(), d.content_hash());
    }

    #[test]
    fn kinds_and_descriptions() {
        assert_eq!(ClipContent::from_text("hello").kind(), ClipKind::Text);
        let files = ClipContent::from_files(&["/tmp/a.txt".into(), "/tmp/b.txt".into()]);
        assert_eq!(files.kind(), ClipKind::Files);
        assert_eq!(files.description(500), "/tmp/a.txt\n/tmp/b.txt");
        let html_only = ClipContent::new().with(formats::HTML, b"<p>Hi <i>there</i></p>".to_vec());
        assert_eq!(html_only.kind(), ClipKind::Text);
        assert_eq!(html_only.description(500), "Hi there");
        assert_eq!(ClipContent::from_text("abcdef").description(3), "abc");
        // An image that also has an HTML <img> fragment is still an image.
        let img = ClipContent::from_png(tiny_png()).with(formats::HTML, b"<img src=x>".to_vec());
        assert_eq!(img.kind(), ClipKind::Image);
        assert_eq!(img.description(500), "Image 2 × 3");
    }

    pub(crate) fn tiny_png() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(2, 3, image::Rgba([255, 0, 0, 255]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }
}
