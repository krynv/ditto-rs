//! Text helpers: safe truncation and best-effort HTML / RTF to plain-text conversion
//! (Ditto shows a description for clips that only carry rich formats).

/// Returns the first `max_chars` characters of `s` (never splits a UTF-8 sequence).
pub fn truncate_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// Windows' "HTML Format" (CF_HTML) prefixes the markup with a `Version:/StartHTML:/...`
/// header. Return just the fragment when such a header (or fragment comments) is present.
pub fn strip_cf_html_header(html: &str) -> &str {
    if html.starts_with("Version:") {
        let offset = |key: &str| -> Option<usize> {
            let start = html.find(key)? + key.len();
            let digits: String = html[start..].chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().ok()
        };
        if let (Some(s), Some(e)) = (offset("StartFragment:"), offset("EndFragment:")) {
            if s <= e && e <= html.len() && html.is_char_boundary(s) && html.is_char_boundary(e) {
                return &html[s..e];
            }
        }
        if let (Some(s), Some(e)) = (offset("StartHTML:"), offset("EndHTML:")) {
            if s <= e && e <= html.len() && html.is_char_boundary(s) && html.is_char_boundary(e) {
                return &html[s..e];
            }
        }
    }
    if let (Some(s), Some(e)) = (html.find("<!--StartFragment-->"), html.find("<!--EndFragment-->")) {
        let s = s + "<!--StartFragment-->".len();
        if s <= e {
            return &html[s..e];
        }
    }
    html
}

fn decode_entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            num.parse().ok()?
        };
        return char::from_u32(code);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "hellip" => '…',
        "mdash" => '—',
        "ndash" => '–',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "bull" => '•',
        "middot" => '·',
        "euro" => '€',
        "pound" => '£',
        "yen" => '¥',
        "cent" => '¢',
        "deg" => '°',
        "times" => '×',
        "divide" => '÷',
        "laquo" => '«',
        "raquo" => '»',
        _ => return None,
    })
}

const BLOCK_TAGS: &[&str] = &[
    "br", "p", "div", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "table", "ul", "ol", "pre",
    "blockquote", "hr", "section", "article", "header", "footer", "dt", "dd",
];

/// Best-effort conversion of HTML markup to readable plain text.
pub fn html_to_text(html: &str) -> String {
    let html = strip_cf_html_header(html);
    let lower = html.to_ascii_lowercase(); // same byte offsets as `html`
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0;
    let mut in_pre = 0usize;
    let mut last_space = true;

    let push_newline = |out: &mut String, last_space: &mut bool| {
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
        *last_space = true;
    };

    while i < html.len() {
        let rest = &html[i..];
        let c = rest.chars().next().unwrap_or(' ');
        if c == '<' {
            if lower[i..].starts_with("<!--") {
                i = lower[i..].find("-->").map_or(html.len(), |e| i + e + 3);
                continue;
            }
            let Some(end) = rest.find('>') else { break };
            let tag = &lower[i + 1..i + end];
            let closing = tag.starts_with('/');
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            i += end + 1;
            if !closing && matches!(name.as_str(), "script" | "style" | "head" | "title") {
                let close = format!("</{name}");
                i = lower[i..].find(&close).map_or(html.len(), |e| {
                    let after = i + e;
                    lower[after..].find('>').map_or(html.len(), |g| after + g + 1)
                });
                continue;
            }
            if name == "pre" {
                if closing {
                    in_pre = in_pre.saturating_sub(1);
                } else {
                    in_pre += 1;
                }
            }
            if BLOCK_TAGS.contains(&name.as_str()) && (closing || name == "br" || name == "hr" || name == "li") {
                push_newline(&mut out, &mut last_space);
            } else if closing && (name == "td" || name == "th") {
                out.push('\t');
                last_space = true;
            }
            continue;
        }
        if c == '&' {
            if let Some(semi) = rest[1..].find(';').filter(|&p| p <= 10) {
                if let Some(ch) = decode_entity(&rest[1..1 + semi]) {
                    out.push(ch);
                    last_space = ch == ' ';
                    i += semi + 2;
                    continue;
                }
            }
        }
        if in_pre == 0 && c.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(c);
            last_space = false;
        }
        i += c.len_utf8();
    }
    tidy_lines(&out)
}

fn tidy_lines(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut blank_run = 0;
    for line in s.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 || result.is_empty() {
                continue;
            }
        } else {
            blank_run = 0;
        }
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(line.trim_start_matches(' '));
    }
    result.trim_end().to_string()
}

const CP1252_HIGH: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
];

/// Decode one Windows-1252 byte.
pub fn cp1252_char(b: u8) -> char {
    match b {
        0x80..=0x9f => CP1252_HIGH[(b - 0x80) as usize],
        _ => b as char,
    }
}

/// Decode Windows-1252 ("ANSI") bytes.
pub fn decode_cp1252(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| cp1252_char(b)).collect()
}

const RTF_SKIP_DESTINATIONS: &[&str] = &[
    "fonttbl", "colortbl", "stylesheet", "info", "pict", "object", "header", "footer", "headerl",
    "headerr", "footerl", "footerr", "listtable", "listoverridetable", "rsidtbl", "generator",
    "xmlnstbl", "themedata", "colorschememapping", "latentstyles", "datastore", "fldinst",
    "filetbl", "revtbl", "pgdsctbl", "expandedcolortbl", "mmathPr", "operator", "author",
];

/// Best-effort conversion of RTF to plain text.
pub fn rtf_to_text(rtf: &str) -> String {
    #[derive(Clone, Copy)]
    struct State {
        skip: bool,
        uc: usize,
    }
    let chars: Vec<char> = rtf.chars().collect();
    let mut out = String::new();
    let mut stack = vec![State { skip: false, uc: 1 }];
    let mut pending_skip = 0usize;
    let mut high_surrogate: Option<u32> = None;
    let mut i = 0;

    fn emit(out: &mut String, stack: &[State], pending_skip: &mut usize, ch: char) {
        if stack.last().is_some_and(|s| s.skip) {
            return;
        }
        if *pending_skip > 0 {
            *pending_skip -= 1;
            return;
        }
        out.push(ch);
    }

    while i < chars.len() {
        let c = chars[i];
        match c {
            '{' => {
                let top = *stack.last().unwrap_or(&State { skip: false, uc: 1 });
                stack.push(top);
                pending_skip = 0;
                i += 1;
            }
            '}' => {
                stack.pop();
                if stack.is_empty() {
                    stack.push(State { skip: false, uc: 1 });
                }
                pending_skip = 0;
                i += 1;
            }
            '\\' => {
                i += 1;
                let Some(&n) = chars.get(i) else { break };
                match n {
                    '\\' | '{' | '}' => {
                        emit(&mut out, &stack, &mut pending_skip, n);
                        i += 1;
                    }
                    '\'' => {
                        let hex: String = chars.iter().skip(i + 1).take(2).collect();
                        if let Ok(b) = u8::from_str_radix(&hex, 16) {
                            emit(&mut out, &stack, &mut pending_skip, cp1252_char(b));
                        }
                        i += 3;
                    }
                    '*' => {
                        if let Some(top) = stack.last_mut() {
                            top.skip = true;
                        }
                        i += 1;
                    }
                    '~' => {
                        emit(&mut out, &stack, &mut pending_skip, ' ');
                        i += 1;
                    }
                    '_' => {
                        emit(&mut out, &stack, &mut pending_skip, '-');
                        i += 1;
                    }
                    '\n' | '\r' => {
                        emit(&mut out, &stack, &mut pending_skip, '\n');
                        i += 1;
                    }
                    n if n.is_ascii_alphabetic() => {
                        let start = i;
                        while i < chars.len() && chars[i].is_ascii_alphabetic() {
                            i += 1;
                        }
                        let word: String = chars[start..i].iter().collect();
                        let neg = chars.get(i) == Some(&'-');
                        if neg {
                            i += 1;
                        }
                        let ps = i;
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                        let param: Option<i64> = if i > ps {
                            chars[ps..i].iter().collect::<String>().parse::<i64>().ok().map(|v| if neg { -v } else { v })
                        } else {
                            None
                        };
                        if chars.get(i) == Some(&' ') {
                            i += 1;
                        }
                        let simple = |ch: char, out: &mut String, pending: &mut usize| emit(out, &stack, pending, ch);
                        match word.as_str() {
                            "par" | "line" | "sect" | "page" | "row" => simple('\n', &mut out, &mut pending_skip),
                            "tab" | "cell" => simple('\t', &mut out, &mut pending_skip),
                            "emdash" => simple('—', &mut out, &mut pending_skip),
                            "endash" => simple('–', &mut out, &mut pending_skip),
                            "bullet" => simple('•', &mut out, &mut pending_skip),
                            "lquote" => simple('‘', &mut out, &mut pending_skip),
                            "rquote" => simple('’', &mut out, &mut pending_skip),
                            "ldblquote" => simple('“', &mut out, &mut pending_skip),
                            "rdblquote" => simple('”', &mut out, &mut pending_skip),
                            "uc" => {
                                if let Some(top) = stack.last_mut() {
                                    top.uc = param.unwrap_or(1).max(0) as usize;
                                }
                            }
                            "u" => {
                                if let Some(p) = param {
                                    let cp = if p < 0 { (p + 65536) as u32 } else { p as u32 };
                                    let skip_now = stack.last().is_some_and(|s| s.skip);
                                    if !skip_now {
                                        if (0xD800..0xDC00).contains(&cp) {
                                            high_surrogate = Some(cp);
                                        } else if (0xDC00..0xE000).contains(&cp) {
                                            if let Some(h) = high_surrogate.take() {
                                                let combined = 0x10000 + ((h - 0xD800) << 10) + (cp - 0xDC00);
                                                if let Some(ch) = char::from_u32(combined) {
                                                    out.push(ch);
                                                }
                                            }
                                        } else if let Some(ch) = char::from_u32(cp) {
                                            out.push(ch);
                                        }
                                    }
                                    pending_skip = stack.last().map_or(1, |s| s.uc);
                                }
                            }
                            w if RTF_SKIP_DESTINATIONS.contains(&w) => {
                                if let Some(top) = stack.last_mut() {
                                    top.skip = true;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => i += 1,
                }
            }
            '\r' | '\n' => i += 1,
            _ => {
                emit(&mut out, &stack, &mut pending_skip, c);
                i += 1;
            }
        }
    }
    tidy_lines(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_respects_char_boundaries() {
        assert_eq!(truncate_chars("héllo", 2), "hé");
        assert_eq!(truncate_chars("hi", 10), "hi");
        assert_eq!(truncate_chars("日本語テキスト", 3), "日本語");
    }

    #[test]
    fn html_basic() {
        let t = html_to_text("<html><head><style>p{}</style></head><body><p>Hello&nbsp;<b>world</b></p><p>a &amp; b</p><br>x</body></html>");
        // `<br>` after a closed paragraph renders as an empty line, like a browser would.
        assert_eq!(t, "Hello world\na & b\n\nx");
    }

    #[test]
    fn html_cf_header() {
        let frag = "<b>hi</b>";
        let body = format!("<html><body><!--StartFragment-->{frag}<!--EndFragment--></body></html>");
        let header_len = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n".len();
        let start_frag = header_len + body.find(frag).unwrap();
        let end_frag = start_frag + frag.len();
        let full = format!(
            "Version:0.9\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\nStartFragment:{:010}\r\nEndFragment:{:010}\r\n{body}",
            header_len,
            header_len + body.len(),
            start_frag,
            end_frag
        );
        assert_eq!(strip_cf_html_header(&full), frag);
        assert_eq!(html_to_text(&full), "hi");
    }

    #[test]
    fn rtf_basic() {
        // Built with concat! so the RTF unicode escape stays a literal backslash-u sequence.
        let rtf = concat!(r"{\rtf1\ansi\deff0{\fonttbl{\f0 Arial;}}{\colortbl;\red0\green0\blue0;}\f0 Hello \b world\b0\par Caf\'e9 \", "u8364?", r"\par}");
        assert_eq!(rtf_to_text(rtf), "Hello world\nCafé €");
    }

    #[test]
    fn cp1252() {
        assert_eq!(decode_cp1252(&[0x80, b'a', 0xe9]), "€aé");
    }
}
