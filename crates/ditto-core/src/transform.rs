//! "Special Paste" text transforms (Ditto: `COleClipSource::*` in OleClipSource.cpp).
//!
//! Each transform takes the clip's plain text; the paste is then text-only, because the
//! HTML/RTF formats would no longer match the transformed text.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextTransform {
    UpperCase,
    LowerCase,
    /// Lower-case everything, then upper-case the first letter of every word.
    Capitalize,
    /// Lower-case everything, then upper-case the first letter of every sentence.
    SentenceCase,
    InvertCase,
    /// Trim leading/trailing spaces, tabs and line breaks.
    TrimWhiteSpace,
    /// Ditto's "CamelCase": `hello big world` → `HelloBigWorld`.
    CamelCase,
    AsciiOnly,
    /// Replace every line break with a space.
    RemoveLineFeeds,
    AddOneLineFeed,
    AddTwoLineFeeds,
    /// Append a line break and the current local date/time.
    AddCurrentTime,
    /// Shuffle the inner letters of words longer than three characters.
    Typoglycemia,
    Slugify,
    /// Trim, turn `\` into `/` and drop surrounding quotes (Ditto: "Posixify paths").
    PosixifyPaths,
}

impl TextTransform {
    pub const ALL: [TextTransform; 15] = [
        TextTransform::UpperCase,
        TextTransform::LowerCase,
        TextTransform::Capitalize,
        TextTransform::SentenceCase,
        TextTransform::InvertCase,
        TextTransform::TrimWhiteSpace,
        TextTransform::CamelCase,
        TextTransform::AsciiOnly,
        TextTransform::RemoveLineFeeds,
        TextTransform::AddOneLineFeed,
        TextTransform::AddTwoLineFeeds,
        TextTransform::AddCurrentTime,
        TextTransform::Typoglycemia,
        TextTransform::Slugify,
        TextTransform::PosixifyPaths,
    ];

    /// Label as shown in Ditto's Special Paste menu.
    pub fn label(self) -> &'static str {
        match self {
            TextTransform::UpperCase => "UPPER CASE",
            TextTransform::LowerCase => "lower case",
            TextTransform::Capitalize => "Capitalize Case",
            TextTransform::SentenceCase => "Sentence case",
            TextTransform::InvertCase => "Invert Case",
            TextTransform::TrimWhiteSpace => "Trim White Space",
            TextTransform::CamelCase => "CamelCase",
            TextTransform::AsciiOnly => "Ascii Text Only",
            TextTransform::RemoveLineFeeds => "Remove Line Feeds",
            TextTransform::AddOneLineFeed => "Paste + One Line Feed",
            TextTransform::AddTwoLineFeeds => "Paste + Two Line Feeds",
            TextTransform::AddCurrentTime => "Paste + Current Time",
            TextTransform::Typoglycemia => "Typoglycemia",
            TextTransform::Slugify => "Slugify",
            TextTransform::PosixifyPaths => "Posixify Paths",
        }
    }

    pub fn apply(self, input: &str) -> String {
        match self {
            TextTransform::UpperCase => input.to_uppercase(),
            TextTransform::LowerCase => input.to_lowercase(),
            TextTransform::Capitalize => capitalize(input),
            TextTransform::SentenceCase => sentence_case(input),
            TextTransform::InvertCase => invert_case(input),
            TextTransform::TrimWhiteSpace => trim_white_space(input).to_string(),
            TextTransform::CamelCase => camel_case(input),
            TextTransform::AsciiOnly => input.chars().filter(char::is_ascii).collect(),
            TextTransform::RemoveLineFeeds => input.replace("\r\n", " ").replace(['\r', '\n'], " "),
            TextTransform::AddOneLineFeed => format!("{input}{LINE_ENDING}"),
            TextTransform::AddTwoLineFeeds => format!("{input}{LINE_ENDING}{LINE_ENDING}"),
            TextTransform::AddCurrentTime => {
                let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                format!("{input}{LINE_ENDING}{now}")
            }
            TextTransform::Typoglycemia => typoglycemia(input, &mut Rng::new()),
            TextTransform::Slugify => slugify(input, "-"),
            TextTransform::PosixifyPaths => posixify(input),
        }
    }
}

/// The platform's line ending (Ditto always appends `\r\n`; that is wrong on macOS/Linux).
pub const LINE_ENDING: &str = if cfg!(windows) { "\r\n" } else { "\n" };

fn upper_first(c: char) -> String {
    c.to_uppercase().collect()
}

fn capitalize(input: &str) -> String {
    let lower = input.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut capitalize_next = true;
    for c in lower.chars() {
        if capitalize_next && c != ' ' {
            out.push_str(&upper_first(c));
            capitalize_next = false;
        } else {
            out.push(c);
            // Ditto only treats a space as a word break.
            if c == ' ' {
                capitalize_next = true;
            }
        }
    }
    out
}

fn sentence_case(input: &str) -> String {
    let lower = input.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut chars = lower.chars();
    if let Some(first) = chars.next() {
        out.push_str(&upper_first(first));
    }
    let mut capitalize = false;
    for c in chars {
        if matches!(c, '.' | '!' | '?') {
            capitalize = true;
            out.push(c);
        } else if capitalize && c != ' ' {
            out.push_str(&upper_first(c));
            capitalize = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn invert_case(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_uppercase() {
                c.to_lowercase().collect::<String>()
            } else if c.is_lowercase() {
                c.to_uppercase().collect::<String>()
            } else {
                c.to_string()
            }
        })
        .collect()
}

fn trim_white_space(input: &str) -> &str {
    input.trim_matches(|c: char| c == ' ' || c == '\t' || c == '\r' || c == '\n')
}

fn camel_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut capital_next = true;
    for c in input.chars() {
        if c == ' ' {
            capital_next = true;
        } else if capital_next {
            out.push_str(&upper_first(c));
            capital_next = false;
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

fn posixify(input: &str) -> String {
    let t = trim_white_space(input);
    let t = t.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(t);
    t.replace('\\', "/")
}

/// Tiny xorshift generator seeded from std's per-process random keys (no `rand` dependency).
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(crate::now_ms() as u64);
        Self(h.finish() | 1)
    }

    #[cfg(test)]
    fn seeded(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn typoglycemia(input: &str, rng: &mut Rng) -> String {
    input
        .split(' ')
        .map(|word| {
            let chars: Vec<char> = word.chars().collect();
            // Ignore trailing sentence punctuation, like Ditto.
            let mut end = chars.len();
            while end > 0 && matches!(chars[end - 1], '.' | '!' | '?') {
                end -= 1;
            }
            if end <= 3 {
                return word.to_string();
            }
            let mut inner: Vec<char> = chars[1..end - 1].to_vec();
            for i in (1..inner.len()).rev() {
                let j = rng.below(i + 1);
                inner.swap(i, j);
            }
            let mut out = String::with_capacity(word.len());
            out.push(chars[0]);
            out.extend(inner);
            out.extend(&chars[end - 1..]);
            out
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn transliterate(c: char) -> Option<&'static str> {
    Some(match c {
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ą' | 'Ă' => "A",
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ą' | 'ă' => "a",
        'Æ' => "AE",
        'æ' => "ae",
        'Ç' | 'Ć' | 'Č' => "C",
        'ç' | 'ć' | 'č' => "c",
        'Ď' | 'Đ' | 'Ð' => "D",
        'ď' | 'đ' | 'ð' => "d",
        'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ę' | 'Ě' => "E",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => "e",
        'Ğ' => "G",
        'ğ' => "g",
        'Ì' | 'Í' | 'Î' | 'Ï' | 'Ī' | 'İ' => "I",
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ı' => "i",
        'Ł' | 'Ľ' | 'Ĺ' => "L",
        'ł' | 'ľ' | 'ĺ' => "l",
        'Ñ' | 'Ń' | 'Ň' => "N",
        'ñ' | 'ń' | 'ň' => "n",
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ő' | 'Ø' | 'Ō' => "O",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ő' | 'ø' | 'ō' => "o",
        'Œ' => "OE",
        'œ' => "oe",
        'Ř' | 'Ŕ' => "R",
        'ř' | 'ŕ' => "r",
        'Ś' | 'Š' | 'Ş' | 'Ș' => "S",
        'ś' | 'š' | 'ş' | 'ș' => "s",
        'ß' => "ss",
        'ẞ' => "SS",
        'Ť' | 'Ţ' | 'Ț' => "T",
        'ť' | 'ţ' | 'ț' => "t",
        'Þ' => "TH",
        'þ' => "th",
        'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ű' | 'Ū' | 'Ů' => "U",
        'ù' | 'ú' | 'û' | 'ü' | 'ű' | 'ū' | 'ů' => "u",
        'Ý' | 'Ÿ' => "Y",
        'ý' | 'ÿ' => "y",
        'Ź' | 'Ž' | 'Ż' => "Z",
        'ź' | 'ž' | 'ż' => "z",
        _ => return None,
    })
}

/// Ditto's Slugify: transliterate common Latin letters, lower-case, and join the remaining
/// alphanumeric runs with `separator`.
pub fn slugify(input: &str, separator: &str) -> String {
    let mut ascii = String::with_capacity(input.len());
    for c in input.chars() {
        match transliterate(c) {
            Some(s) => ascii.push_str(s),
            None => ascii.push(c),
        }
    }
    let lower = ascii.to_lowercase();
    lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(separator)
}

/// Expand Ditto's multi-paste separator tokens (`[LF]`, `[CR]`, `[TAB]`; default `[LF]`).
pub fn expand_separator(sep: &str) -> String {
    sep.replace("[LF]", LINE_ENDING).replace("[CR]", "\r").replace("[TAB]", "\t")
}

#[cfg(test)]
mod tests {
    use super::*;
    use TextTransform::*;

    #[test]
    fn case_transforms_match_ditto() {
        assert_eq!(UpperCase.apply("straße"), "STRASSE");
        assert_eq!(LowerCase.apply("ÀB"), "àb");
        assert_eq!(Capitalize.apply("hELLO big  WORLD"), "Hello Big  World");
        assert_eq!(SentenceCase.apply("hELLO. how ARE you? fine!ok"), "Hello. How are you? Fine!Ok");
        assert_eq!(InvertCase.apply("Hello W0rld"), "hELLO w0RLD");
        assert_eq!(CamelCase.apply("hello BIG world"), "HelloBigWorld");
    }

    #[test]
    fn whitespace_transforms() {
        assert_eq!(TrimWhiteSpace.apply(" \t\r\n hi there \n"), "hi there");
        assert_eq!(RemoveLineFeeds.apply("a\r\nb\nc\rd"), "a b c d");
        assert_eq!(AddOneLineFeed.apply("x"), format!("x{LINE_ENDING}"));
        assert_eq!(AddTwoLineFeeds.apply("x"), format!("x{LINE_ENDING}{LINE_ENDING}"));
        assert!(AddCurrentTime.apply("x").starts_with(&format!("x{LINE_ENDING}20")));
        assert_eq!(AsciiOnly.apply("héllo wörld ✓"), "hllo wrld ");
        assert_eq!(PosixifyPaths.apply(" \"C:\\Users\\me\\file.txt\" "), "C:/Users/me/file.txt");
    }

    #[test]
    fn slugify_transliterates() {
        assert_eq!(Slugify.apply("Ça va? Größe & Œuvre -- 2024!"), "ca-va-grosse-oeuvre-2024");
        assert_eq!(slugify("  --  ", "-"), "");
    }

    #[test]
    fn typoglycemia_keeps_outer_letters() {
        let mut rng = Rng::seeded(42);
        let out = typoglycemia("According to research, it doesn't matter!", &mut rng);
        let words: Vec<&str> = out.split(' ').collect();
        assert_eq!(words.len(), 6);
        assert_eq!(words[1], "to");
        for (orig, new) in "According to research, it doesn't matter!".split(' ').zip(&words) {
            assert_eq!(orig.len(), new.len());
            assert_eq!(orig.chars().next(), new.chars().next());
            assert_eq!(orig.chars().last(), new.chars().last());
            let mut a: Vec<char> = orig.chars().collect();
            let mut b: Vec<char> = new.chars().collect();
            a.sort();
            b.sort();
            assert_eq!(a, b);
        }
    }

    #[test]
    fn separators() {
        assert_eq!(expand_separator("[TAB]|[CR]"), "\t|\r");
        assert_eq!(expand_separator("[LF]"), LINE_ENDING);
    }
}
