//! Linux and the BSDs: the `tesseract` command, when it is installed. It is run as a program
//! instead of linked as a library so Ditto-RS works without it.
#![cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ditto_core::imaging;
use ditto_core::model::png_dimensions;

use super::Status;
use crate::{PlatformError, Result};

const PROGRAM: &str = "tesseract";
const NOT_INSTALLED: &str = "Install Tesseract to search the text in images: the tesseract-ocr package on Debian and \
                             Ubuntu, tesseract on Fedora and Arch.";
const NO_LANGUAGES: &str = "Tesseract is installed without language data. Install a language, such as the \
                            tesseract-ocr-eng package.";
/// Give up on one image after this long.
const TIMEOUT: Duration = Duration::from_secs(90);
/// Images up to this size (a 1920 × 1200 screen) are probably at 100 % scale, where text is
/// too small for Tesseract; they are read at twice the size.
const ENLARGE_UP_TO_PIXELS: u64 = 2_500_000;

/// The languages to read. Cached once found, so installing Tesseract later needs no restart.
static LANGUAGES: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// `None`: Tesseract is not installed. Empty: it has no language data.
fn languages() -> Option<Vec<String>> {
    let mut cached = LANGUAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(languages) = cached.as_ref() {
        return Some(languages.clone());
    }
    let output = Command::new(PROGRAM).arg("--list-langs").stdin(Stdio::null()).output().ok()?;
    // Tesseract 3 prints the list on stderr, later versions on stdout.
    let listing = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let chosen = choose_languages(&parse_languages(&listing), locale().as_deref());
    if !chosen.is_empty() {
        *cached = Some(chosen.clone());
    }
    Some(chosen)
}

/// The language names in `tesseract --list-langs` output. Its heading and any warnings contain
/// spaces; `osd` and `equ` are helper models; `script/…` models cover whole writing systems.
fn parse_languages(listing: &str) -> Vec<String> {
    listing
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains(char::is_whitespace) && !l.contains('/') && !matches!(*l, "osd" | "equ"))
        .map(str::to_string)
        .collect()
}

/// English plus the language of the user's locale. Every extra language makes Tesseract
/// slower and less accurate, so the other installed ones are left out.
fn choose_languages(installed: &[String], locale: Option<&str>) -> Vec<String> {
    let has = |code: &str| installed.iter().any(|l| l == code);
    let mut chosen = Vec::new();
    if has("eng") {
        chosen.push("eng".to_string());
    }
    if let Some(code) = locale.and_then(tesseract_code) {
        if has(code) && !chosen.iter().any(|c| c == code) {
            chosen.push(code.to_string());
        }
    }
    if chosen.is_empty() {
        chosen.extend(installed.first().cloned());
    }
    chosen
}

fn locale() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
}

/// Tesseract's name for the language of a locale such as `de_DE.UTF-8`.
fn tesseract_code(locale: &str) -> Option<&'static str> {
    let locale = locale.split(['.', '@']).next().unwrap_or_default();
    let (language, region) = locale.split_once(['_', '-']).unwrap_or((locale, ""));
    Some(match language.to_ascii_lowercase().as_str() {
        "en" => "eng",
        "de" => "deu",
        "fr" => "fra",
        "es" => "spa",
        "it" => "ita",
        "pt" => "por",
        "nl" => "nld",
        "sv" => "swe",
        "da" => "dan",
        "nb" | "nn" | "no" => "nor",
        "fi" => "fin",
        "pl" => "pol",
        "cs" => "ces",
        "sk" => "slk",
        "hu" => "hun",
        "ro" => "ron",
        "bg" => "bul",
        "el" => "ell",
        "tr" => "tur",
        "ru" => "rus",
        "uk" => "ukr",
        "he" => "heb",
        "ar" => "ara",
        "hi" => "hin",
        "th" => "tha",
        "vi" => "vie",
        "id" => "ind",
        "ja" => "jpn",
        "ko" => "kor",
        "zh" if matches!(region.to_ascii_uppercase().as_str(), "TW" | "HK" | "MO") => "chi_tra",
        "zh" => "chi_sim",
        _ => return None,
    })
}

pub fn status() -> Status {
    let (available, detail) = match languages() {
        None => (false, NOT_INSTALLED.to_string()),
        Some(languages) if languages.is_empty() => (false, NO_LANGUAGES.to_string()),
        Some(languages) => (true, format!("Reads {}. Other languages: install their tesseract-ocr packages.", languages.join(" + "))),
    };
    Status { engine: "Tesseract", available, detail }
}

pub fn recognize(png: &[u8]) -> Result<String> {
    let languages = languages().filter(|l| !l.is_empty()).ok_or(PlatformError::Unsupported("Tesseract is not installed"))?;
    let languages = languages.join("+");
    let scale = match png_dimensions(png) {
        Some((w, h)) if u64::from(w) * u64::from(h) <= ENLARGE_UP_TO_PIXELS => 2,
        _ => 1,
    };
    let gray = imaging::gray_for_ocr(png, scale).map_err(|e| PlatformError::Ocr(e.to_string()))?;

    // Tesseract expects dark text on a light background. Read dark-theme images with light
    // and dark swapped, and images with both kinds of area (a dark terminal on a light
    // desktop) both ways.
    let dark = gray.dark_fraction();
    let passes: &[bool] = if dark < 0.15 {
        &[false]
    } else if dark > 0.85 {
        &[true]
    } else {
        &[false, true]
    };
    let mut lines: Vec<String> = Vec::new();
    for &inverted in passes {
        for line in run(&languages, &gray.to_pgm(inverted))?.lines().map(str::trim) {
            if !line.is_empty() && !lines.iter().any(|l| l == line) {
                lines.push(line.to_string());
            }
        }
    }
    Ok(lines.join("\n"))
}

/// `tesseract stdin stdout`: the image goes in on stdin, the text comes back on stdout.
fn run(languages: &str, image: &[u8]) -> Result<String> {
    let mut child = Command::new(PROGRAM)
        // Page segmentation mode 11, "sparse text": find as much text as possible in no
        // particular order, which suits screenshots better than the document layout default.
        .args(["stdin", "stdout", "-l", languages, "--psm", "11"])
        // One thread: OpenMP gains little here and would occupy every core in the background.
        .env("OMP_THREAD_LIMIT", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| PlatformError::Ocr(format!("could not start tesseract: {e}")))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");

    let (status, out, err) = std::thread::scope(|scope| {
        // Dropping stdin at the end of the closure closes the pipe, which starts recognition.
        scope.spawn(move || {
            let _ = stdin.write_all(image);
        });
        let read = |mut pipe: Box<dyn Read + Send>| {
            move || {
                let mut bytes = Vec::new();
                let _ = pipe.read_to_end(&mut bytes);
                bytes
            }
        };
        let out = scope.spawn(read(Box::new(&mut stdout)));
        let err = scope.spawn(read(Box::new(&mut stderr)));
        let deadline = Instant::now() + TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(PlatformError::Ocr(format!("tesseract took longer than {} s", TIMEOUT.as_secs())));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => break Err(PlatformError::Ocr(format!("could not wait for tesseract: {e}"))),
            }
        };
        (status, out.join().unwrap_or_default(), err.join().unwrap_or_default())
    });

    let status = status?;
    if !status.success() {
        let stderr = String::from_utf8_lossy(&err);
        let reason = stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no error message");
        return Err(PlatformError::Ocr(format!("tesseract failed ({status}): {reason}")));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_the_language_list() {
        let v5 = "List of available languages in \"/usr/share/tesseract-ocr/5/tessdata/\" (5):\ndeu\neng\nosd\nscript/Latin\nchi_sim\n";
        assert_eq!(parse_languages(v5), names(&["deu", "eng", "chi_sim"]));
        assert_eq!(parse_languages("Error opening data file ./eng.traineddata\n\n"), Vec::<String>::new());
    }

    #[test]
    fn reads_english_and_the_locale_language() {
        let installed = names(&["deu", "eng", "fra", "ukr"]);
        assert_eq!(choose_languages(&installed, Some("de_DE.UTF-8")), names(&["eng", "deu"]));
        assert_eq!(choose_languages(&installed, Some("en_GB.UTF-8")), names(&["eng"]));
        assert_eq!(choose_languages(&installed, Some("uk_UA")), names(&["eng", "ukr"]));
        assert_eq!(choose_languages(&installed, Some("ja_JP.UTF-8")), names(&["eng"]));
        assert_eq!(choose_languages(&installed, None), names(&["eng"]));
        // Without English: the locale's language, else whatever is there.
        assert_eq!(choose_languages(&names(&["deu", "fra"]), Some("fr_FR.UTF-8")), names(&["fra"]));
        assert_eq!(choose_languages(&names(&["deu", "fra"]), Some("C")), names(&["deu"]));
        assert_eq!(choose_languages(&[], Some("en_US.UTF-8")), Vec::<String>::new());
    }

    #[test]
    fn maps_locales_to_tesseract_languages() {
        assert_eq!(tesseract_code("pt_BR.UTF-8"), Some("por"));
        assert_eq!(tesseract_code("zh_CN.UTF-8"), Some("chi_sim"));
        assert_eq!(tesseract_code("zh_TW"), Some("chi_tra"));
        assert_eq!(tesseract_code("sr_RS@latin"), None);
        assert_eq!(tesseract_code("nb-NO"), Some("nor"));
        assert_eq!(tesseract_code("C"), None);
        assert_eq!(tesseract_code("POSIX"), None);
        assert_eq!(tesseract_code(""), None);
    }

    /// Runs Tesseract where it is installed (on any system, not only the ones that use it).
    #[test]
    fn reads_the_sample_image_when_tesseract_is_installed() {
        if !status().available {
            eprintln!("skipped: {}", status().detail);
            return;
        }
        let text = recognize(super::super::tests::SAMPLE).expect("tesseract runs").to_lowercase();
        eprintln!("tesseract read:\n{text}");
        for expected in ["hello from ditto", "invoice", "48213", "quarterly report", "light on dark"] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
    }
}
