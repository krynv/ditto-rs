//! Text recognition (OCR) for image clips, so an image can be found by the words in it.
//! Ditto has no equivalent; this is a Ditto-RS extension. Everything runs on this computer.
//!
//! | Platform  | Engine                                                                |
//! |-----------|-----------------------------------------------------------------------|
//! | macOS     | Apple Vision (`VNRecognizeTextRequest`, the engine behind Live Text)  |
//! | Windows   | `Windows.Media.Ocr`; needs an OCR language, which most display languages install |
//! | Linux/BSD | the `tesseract` command, when it is installed                         |

use crate::Result;

#[cfg(any(test, all(unix, not(target_os = "macos"))))]
mod tesseract;
#[cfg(target_os = "macos")]
mod vision;
#[cfg(windows)]
mod winrt;

/// What recognizes text on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// "Apple Vision", "Windows OCR" or "Tesseract".
    pub engine: &'static str,
    pub available: bool,
    /// The languages it reads, or what to do when it is unavailable. Shown in the options window.
    pub detail: String,
}

/// Which engine would be used and whether it can run. Cheap enough to call before each image.
pub fn status() -> Status {
    #[cfg(target_os = "macos")]
    {
        vision::status()
    }
    #[cfg(windows)]
    {
        winrt::status()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        tesseract::status()
    }
    #[cfg(not(any(unix, windows)))]
    {
        Status { engine: "none", available: false, detail: "Text recognition is not supported on this system.".into() }
    }
}

/// Recognize the text in a PNG image: one line of text per line, in reading order, or an
/// empty string when the image has no text. Slow (tenths of a second to seconds): call it
/// from a background thread.
pub fn recognize_png(png: &[u8]) -> Result<String> {
    #[cfg(target_os = "macos")]
    let text = vision::recognize(png)?;
    #[cfg(windows)]
    let text = winrt::recognize(png)?;
    #[cfg(all(unix, not(target_os = "macos")))]
    let text = tesseract::recognize(png)?;
    #[cfg(not(any(unix, windows)))]
    let text = {
        let _ = png;
        return Err(crate::PlatformError::Unsupported("text recognition is not supported on this system"));
    };
    Ok(tidy(&text))
}

/// Trim every line and drop the empty ones.
fn tidy(text: &str) -> String {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

/// Windows OCR reads text best when it is about this tall, in pixels.
#[cfg(any(test, windows))]
const IDEAL_TEXT_HEIGHT: f64 = 36.0;

/// The scale for a second recognition pass on Windows, if one is worthwhile.
///
/// `Windows.Media.Ocr` misreads small text (a 100 % screenshot has 11–13 px text), so after a
/// first pass the image is enlarged until its text is about [`IDEAL_TEXT_HEIGHT`] tall, the
/// approach PowerToys Text Extractor uses. `word_height` is the median height of the words the
/// first pass found, in the pixels of that pass; `current` is the scale that pass used.
#[cfg(any(test, windows))]
fn second_pass_scale(word_height: Option<f64>, width: u32, height: u32, current: f64, max_dimension: u32) -> Option<f64> {
    const MAX_PIXELS: f64 = 16_000_000.0;
    let longest = f64::from(width.max(height).max(1));
    let wanted = match word_height {
        // Already large enough to read well.
        Some(h) if h >= 24.0 => return None,
        Some(h) => current * (IDEAL_TEXT_HEIGHT / h.max(4.0)),
        // Nothing found: small text can be missed entirely, so try once at double size.
        None if longest * current <= 2000.0 => current * 2.0,
        None => return None,
    };
    let pixels = f64::from(width.max(1)) * f64::from(height.max(1));
    let scale = wanted.min(current * 4.0).min(f64::from(max_dimension) / longest).min((MAX_PIXELS / pixels).sqrt());
    (scale >= current * 1.3).then_some(scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 800 × 300, rendered by a browser at 100 %: dark-on-light and light-on-dark text.
    pub(super) const SAMPLE: &[u8] = include_bytes!("../../tests/fixtures/ocr-sample.png");

    #[test]
    fn tidies_recognized_text() {
        assert_eq!(tidy("  Hello \r\n\n\t\nworld  \n"), "Hello\nworld");
        assert_eq!(tidy(" \n\x0c"), "");
    }

    #[test]
    fn second_pass_enlarges_small_text_within_limits() {
        // 12 px text in a 1920 × 1080 screenshot: enlarge so it is about 36 px, capped by pixels.
        let s = second_pass_scale(Some(12.0), 1920, 1080, 1.0, 10_000).unwrap();
        assert!((2.7..=3.0).contains(&s), "{s}");
        // A small snippet can take the full factor.
        assert_eq!(second_pass_scale(Some(12.0), 400, 100, 1.0, 10_000), Some(3.0));
        // Never more than 4× and never past the engine's largest image.
        assert_eq!(second_pass_scale(Some(5.0), 400, 100, 1.0, 10_000), Some(4.0));
        assert_eq!(second_pass_scale(Some(12.0), 2000, 100, 1.0, 5000), Some(2.5));
        // Large text, or too little to gain, is left alone.
        assert_eq!(second_pass_scale(Some(30.0), 400, 100, 1.0, 10_000), None);
        assert_eq!(second_pass_scale(Some(12.0), 4000, 3000, 1.0, 10_000), None);
        // No text found: one try at double size, for small images only.
        assert_eq!(second_pass_scale(None, 800, 300, 1.0, 10_000), Some(2.0));
        assert_eq!(second_pass_scale(None, 3000, 2000, 1.0, 10_000), None);
        // An image that had to be shrunk to fit is not enlarged again.
        assert_eq!(second_pass_scale(Some(12.0), 20_000, 500, 0.5, 10_000), None);
    }

    /// Runs the real engine. Where there is none (no OCR language on Windows, no Tesseract on
    /// Linux, or a CI machine whose engine cannot run) the test reports that and passes.
    #[test]
    fn recognizes_the_sample_image() {
        let status = status();
        eprintln!("OCR engine: {} (available: {}; {})", status.engine, status.available, status.detail);
        if !status.available {
            eprintln!("skipped: no text recognition engine on this machine");
            return;
        }
        let text = match recognize_png(SAMPLE) {
            Ok(text) => text,
            Err(e) if std::env::var_os("CI").is_some() => {
                eprintln!("skipped: the engine failed on this CI machine: {e}");
                return;
            }
            Err(e) => panic!("recognition failed: {e}"),
        };
        eprintln!("recognized:\n{text}");
        let lower = text.to_lowercase();
        for expected in ["hello from ditto", "invoice", "48213", "quarterly report", "light on dark"] {
            assert!(lower.contains(expected), "missing {expected:?} in:\n{text}");
        }
        assert!(recognize_png(b"not a png").is_err() || recognize_png(b"not a png").is_ok_and(|t| t.is_empty()));
    }
}
