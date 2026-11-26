//! Ditto's quick-paste search.
//!
//! Behaviour follows `CFormatSQL::Parse`/`AddToSQL` and `CQPasteWnd` in the original:
//!
//! * **Wildcard** (default): space-separated terms are ANDed, `OR`/`AND`/`NOT`/`!` are
//!   keywords, `"double quotes"` group a phrase, each term matches anywhere in the field.
//!   Ditto turns the terms into SQL, so `AND` binds tighter than `OR`; we keep that.
//!   `*` inside a term is a wildcard (Ditto 3.25 escapes it into a literal `%`, which looks
//!   unintended; we implement the documented intent).
//! * **Contains**: the whole input is one literal substring.
//! * **Regex**: the whole input is a regular expression.
//!
//! The query is evaluated against the description (default), the full text and/or the
//! quick-paste text; a clip matches when any enabled field matches. A `/q ` (or `\q `)
//! prefix searches only quick-paste text and `/f ` (or `\f `) only the full text.
//! Matching is case-insensitive (Unicode-aware, where SQLite's `LIKE` is ASCII-only).

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    #[default]
    Wildcard,
    Contains,
    Regex,
}

/// Which fields a search looks at (Ditto: Search Description / Full Text / Quick Paste).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchScopes {
    pub description: bool,
    pub full_text: bool,
    pub quick_paste: bool,
}

impl Default for SearchScopes {
    fn default() -> Self {
        Self { description: true, full_text: false, quick_paste: false }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchSpec {
    pub query: String,
    #[serde(default)]
    pub mode: SearchMode,
    #[serde(default)]
    pub scopes: SearchScopes,
}

/// The fields of one clip a search is evaluated against.
#[derive(Debug, Clone, Copy, Default)]
pub struct SearchFields<'a> {
    pub description: &'a str,
    pub quick_paste: Option<&'a str>,
    pub full_text: Option<&'a str>,
}

#[derive(Debug)]
enum Pattern {
    /// Lower-cased literal.
    Substring(String),
    /// Case-insensitive wildcard expression.
    Wildcard(Regex),
}

#[derive(Debug)]
struct Term {
    negated: bool,
    pattern: Pattern,
}

#[derive(Debug)]
enum Matcher {
    /// OR of AND-groups.
    Boolean(Vec<Vec<Term>>),
    Contains(String),
    Regex(Regex),
}

#[derive(Debug)]
pub struct CompiledSearch {
    matcher: Matcher,
    scopes: SearchScopes,
}

impl CompiledSearch {
    /// Compile a search. Returns `Ok(None)` when the query filters nothing.
    pub fn compile(spec: &SearchSpec) -> Result<Option<Self>> {
        let mut query = spec.query.as_str();
        let mut scopes = spec.scopes;
        if let Some(rest) = query.strip_prefix("/q ").or_else(|| query.strip_prefix("\\q ")) {
            query = rest;
            scopes = SearchScopes { description: false, full_text: false, quick_paste: true };
        } else if let Some(rest) = query.strip_prefix("/f ").or_else(|| query.strip_prefix("\\f ")) {
            query = rest;
            scopes = SearchScopes { description: false, full_text: true, quick_paste: false };
        }
        if !scopes.description && !scopes.full_text && !scopes.quick_paste {
            scopes.description = true;
        }
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let matcher = match spec.mode {
            SearchMode::Contains => Matcher::Contains(trimmed.to_lowercase()),
            SearchMode::Regex => Matcher::Regex(
                RegexBuilder::new(trimmed)
                    .case_insensitive(true)
                    .size_limit(1 << 22)
                    .build()
                    .map_err(|e| Error::Pattern(e.to_string()))?,
            ),
            SearchMode::Wildcard => match parse_boolean(query)? {
                Some(groups) => Matcher::Boolean(groups),
                None => return Ok(None),
            },
        };
        Ok(Some(Self { matcher, scopes }))
    }

    pub fn scopes(&self) -> SearchScopes {
        self.scopes
    }

    /// Whether [`SearchFields::full_text`] must be supplied.
    pub fn needs_full_text(&self) -> bool {
        self.scopes.full_text
    }

    pub fn matches(&self, fields: &SearchFields<'_>) -> bool {
        (self.scopes.description && self.matches_field(fields.description))
            || (self.scopes.quick_paste && fields.quick_paste.is_some_and(|q| self.matches_field(q)))
            || (self.scopes.full_text && fields.full_text.is_some_and(|t| self.matches_field(t)))
    }

    /// Evaluate the query against a single string.
    pub fn matches_field(&self, field: &str) -> bool {
        match &self.matcher {
            Matcher::Contains(needle) => field.to_lowercase().contains(needle.as_str()),
            Matcher::Regex(re) => re.is_match(field),
            Matcher::Boolean(groups) => {
                let lower = field.to_lowercase();
                groups.iter().any(|group| {
                    !group.is_empty()
                        && group.iter().all(|t| {
                            let hit = match &t.pattern {
                                Pattern::Substring(s) => lower.contains(s.as_str()),
                                Pattern::Wildcard(re) => re.is_match(field),
                            };
                            hit != t.negated
                        })
                })
            }
        }
    }
}

fn parse_boolean(query: &str) -> Result<Option<Vec<Vec<Term>>>> {
    // Ditto replaces brackets with spaces before tokenising.
    let cleaned: String = query.chars().map(|c| if c == '[' || c == ']' { ' ' } else { c }).collect();

    let mut groups: Vec<Vec<Term>> = vec![Vec::new()];
    let mut negate_next = false;
    let mut or_next = false;

    let mut handle = |word: &str, quoted: bool, groups: &mut Vec<Vec<Term>>| -> Result<()> {
        let word = word.trim();
        if word.is_empty() {
            return Ok(());
        }
        if !quoted {
            match word.to_uppercase().as_str() {
                "NOT" | "!" => {
                    negate_next = true;
                    return Ok(());
                }
                "OR" => {
                    or_next = true;
                    return Ok(());
                }
                "AND" => {
                    or_next = false;
                    return Ok(());
                }
                _ => {}
            }
        }
        let pattern = if word.contains('*') {
            let expr = word.split('*').map(regex::escape).collect::<Vec<_>>().join(".*");
            Pattern::Wildcard(
                RegexBuilder::new(&expr)
                    .case_insensitive(true)
                    .build()
                    .map_err(|e| Error::Pattern(e.to_string()))?,
            )
        } else {
            Pattern::Substring(word.to_lowercase())
        };
        if or_next && groups.last().is_some_and(|g| !g.is_empty()) {
            groups.push(Vec::new());
        }
        groups.last_mut().expect("at least one group").push(Term { negated: negate_next, pattern });
        negate_next = false;
        or_next = false;
        Ok(())
    };

    let mut current = String::new();
    let mut in_quotes = false;
    let mut quoted = false;
    for ch in cleaned.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                quoted = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                handle(&current, quoted, &mut groups)?;
                current.clear();
                quoted = false;
            }
            c => current.push(c),
        }
    }
    handle(&current, quoted, &mut groups)?;

    groups.retain(|g| !g.is_empty());
    Ok(if groups.is_empty() { None } else { Some(groups) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(query: &str, mode: SearchMode) -> CompiledSearch {
        CompiledSearch::compile(&SearchSpec { query: query.into(), mode, scopes: SearchScopes::default() })
            .unwrap()
            .expect("non-empty query")
    }

    fn hit(query: &str, text: &str) -> bool {
        search(query, SearchMode::Wildcard).matches_field(text)
    }

    #[test]
    fn empty_and_keyword_only_queries_do_not_filter() {
        for q in ["", "   ", "OR", "NOT AND"] {
            let s = CompiledSearch::compile(&SearchSpec { query: q.into(), ..Default::default() }).unwrap();
            assert!(s.is_none(), "{q:?}");
        }
    }

    #[test]
    fn terms_are_anded_and_case_insensitive() {
        assert!(hit("hello WORLD", "Hello there, world"));
        assert!(!hit("hello mars", "Hello there, world"));
        assert!(hit("ÄPFEL", "grüne äpfel"));
    }

    #[test]
    fn or_and_not_follow_sql_precedence() {
        // foo OR bar baz  ==  foo OR (bar AND baz)
        assert!(hit("foo OR bar baz", "foo"));
        assert!(hit("foo OR bar baz", "bar baz"));
        assert!(!hit("foo OR bar baz", "bar"));
        assert!(hit("cat NOT dog", "a cat"));
        assert!(!hit("cat NOT dog", "a cat and a dog"));
        assert!(hit("cat ! dog", "cat"));
    }

    #[test]
    fn quotes_make_phrases_and_literals() {
        assert!(hit("\"hello world\"", "say hello world"));
        assert!(!hit("\"hello world\"", "hello big world"));
        // A quoted keyword is just text.
        assert!(hit("\"or\"", "more"));
    }

    #[test]
    fn wildcards() {
        assert!(hit("h*o", "say hello"));
        assert!(!hit("h*z", "say hello"));
        assert!(hit("a*c", "A.b.C"));
    }

    #[test]
    fn contains_and_regex_modes() {
        let c = search("a OR b", SearchMode::Contains);
        assert!(c.matches_field("xa or by"));
        assert!(!c.matches_field("a"));
        let r = search(r"^\d{3}-\d{4}$", SearchMode::Regex);
        assert!(r.matches_field("555-1234"));
        assert!(!r.matches_field("call 555-1234"));
        let bad = CompiledSearch::compile(&SearchSpec { query: "(".into(), mode: SearchMode::Regex, ..Default::default() });
        assert!(matches!(bad, Err(Error::Pattern(_))));
    }

    #[test]
    fn scopes_and_prefixes() {
        let fields = SearchFields { description: "short desc", quick_paste: Some("sig"), full_text: Some("short desc and a long tail") };
        assert!(search("desc", SearchMode::Wildcard).matches(&fields));
        assert!(!search("tail", SearchMode::Wildcard).matches(&fields));
        assert!(search("/f tail", SearchMode::Wildcard).matches(&fields));
        assert!(!search("/f sig", SearchMode::Wildcard).matches(&fields));
        assert!(search("/q sig", SearchMode::Wildcard).matches(&fields));
        assert!(!search("/q desc", SearchMode::Wildcard).matches(&fields));
        assert!(search("/f tail", SearchMode::Wildcard).needs_full_text());

        let all = CompiledSearch::compile(&SearchSpec {
            query: "sig".into(),
            mode: SearchMode::Wildcard,
            scopes: SearchScopes { description: true, full_text: true, quick_paste: true },
        })
        .unwrap()
        .unwrap();
        assert!(all.matches(&fields));
        // A missing field never matches, even for negated terms (SQL NULL semantics).
        let no_qp = SearchFields { quick_paste: None, ..fields };
        assert!(!search("/q NOT zzz", SearchMode::Wildcard).matches(&no_qp));
    }
}
