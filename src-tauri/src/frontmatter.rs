//! YAML frontmatter at the top of a markdown note (Obsidian style).
//!
//! Only a block at the very start counts: a `---` line, then lines up to the first `---`
//! or `...` line (trailing spaces allowed on both). A `---` later in a note stays a rule
//! or setext underline, so pulldown-cmark's metadata option (which matches anywhere) is
//! not used. Parsers see the body after the block; `tags:` feed the tag index and
//! `aliases:` resolve wikilinks.
//!
//! Values use a small YAML subset: `key: a`, `key: a, b`, `key: [a, "b"]` or a block
//! list (`- a` lines below `key:`). Keys match case-insensitively. Anything else is
//! ignored, never an error.
//!
//! The editor's twin is `src/lib/frontmatter.ts`; both pass
//! `tests/fixtures/frontmatter-cases.json`.

use std::ops::Range;

/// The frontmatter block: `0..body` is the block with its closing line, and `inner` the
/// lines between the delimiters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frontmatter {
    pub inner: Range<usize>,
    pub body: usize,
}

/// `line` without its line ending, and its end including the ending.
fn next_line(src: &str, at: usize) -> (&str, usize) {
    let rest = &src[at..];
    match rest.find('\n') {
        Some(n) => (
            rest[..n].strip_suffix('\r').unwrap_or(&rest[..n]),
            at + n + 1,
        ),
        None => (rest, src.len()),
    }
}

fn is_delimiter(line: &str, closing: bool) -> bool {
    let t = line.trim_end_matches(' ');
    t == "---" || (closing && t == "...")
}

/// The frontmatter block at the start of `src`, if it has one.
pub fn parse(src: &str) -> Option<Frontmatter> {
    let (first, mut at) = next_line(src, 0);
    if !is_delimiter(first, false) || (at == src.len() && !src.ends_with('\n')) {
        return None;
    }
    let inner_start = at;
    while at < src.len() {
        let (line, next) = next_line(src, at);
        if is_delimiter(line, true) {
            return Some(Frontmatter {
                inner: inner_start..at,
                body: next,
            });
        }
        at = next;
    }
    None
}

/// Where the body starts: after the frontmatter block, or 0.
pub fn body_start(src: &str) -> usize {
    parse(src).map_or(0, |f| f.body)
}

/// Byte ranges of the values of `key`, quotes stripped, empty values dropped. `split`
/// tells which characters separate values in a plain (unbracketed) scalar.
fn values(src: &str, key: &str, split: fn(char) -> bool) -> Vec<Range<usize>> {
    let Some(fm) = parse(src) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut at = fm.inner.start;
    let mut in_list = false;
    while at < fm.inner.end {
        let (line, next) = next_line(src, at);
        let start = at;
        at = next;
        if in_list {
            let trimmed = line.trim_start();
            if let Some(item) = trimmed.strip_prefix('-') {
                if item.is_empty() || item.starts_with(' ') {
                    let from = start + (line.len() - item.len());
                    push_value(src, from..from + item.len(), &mut out);
                    continue;
                }
            }
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            in_list = false;
        }
        // A key starts in column 0.
        let Some((k, _)) = line.split_once(':') else {
            continue;
        };
        if !k.trim_end().eq_ignore_ascii_case(key) || k.starts_with([' ', '\t']) {
            continue;
        }
        let value_from = start + k.len() + 1;
        let value_to = start + line.len();
        let value = src[value_from..value_to].trim();
        if value.is_empty() {
            in_list = true;
        } else if value.starts_with('[') && value.ends_with(']') {
            let open = value_from + src[value_from..].find('[').unwrap_or(0) + 1;
            let close = value_from + src[value_from..value_to].rfind(']').unwrap_or(0);
            split_into(src, open..close, |c| c == ',', &mut out);
        } else if !value.starts_with('#') {
            split_into(src, value_from..value_to, split, &mut out);
        }
    }
    out
}

fn split_into(
    src: &str,
    range: Range<usize>,
    split: fn(char) -> bool,
    out: &mut Vec<Range<usize>>,
) {
    let mut from = range.start;
    for (i, c) in src[range.clone()].char_indices() {
        if split(c) {
            push_value(src, from..range.start + i, out);
            from = range.start + i + c.len_utf8();
        }
    }
    push_value(src, from..range.end, out);
}

/// Trims whitespace and one pair of matching quotes, then keeps the value if non-empty.
fn push_value(src: &str, range: Range<usize>, out: &mut Vec<Range<usize>>) {
    let raw = &src[range.clone()];
    let lead = raw.len() - raw.trim_start().len();
    let t = raw.trim();
    let (mut from, mut to) = (range.start + lead, range.start + lead + t.len());
    if t.len() >= 2
        && (t.starts_with('"') && t.ends_with('"') || t.starts_with('\'') && t.ends_with('\''))
    {
        from += 1;
        to -= 1;
        let inner = &src[from..to];
        from += inner.len() - inner.trim_start().len();
        to = from + inner.trim().len();
    }
    if from < to {
        out.push(from..to);
    }
}

/// Byte ranges of the `tags:` values, a leading `#` excluded (`tags: [#a, b]`). Plain
/// values split on commas and whitespace; whether one is a valid tag is the caller's call.
pub fn tag_ranges(src: &str) -> Vec<Range<usize>> {
    values(src, "tags", |c| c == ',' || c.is_whitespace())
        .into_iter()
        .map(|r| {
            if src[r.clone()].starts_with('#') {
                r.start + 1..r.end
            } else {
                r
            }
        })
        .filter(|r| r.start < r.end)
        .collect()
}

/// The `aliases:` values, as written. Plain values split on commas.
pub fn aliases(src: &str) -> Vec<String> {
    values(src, "aliases", |c| c == ',')
        .into_iter()
        .map(|r| src[r].to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Case {
        name: String,
        md: String,
        /// The text after the block (the whole note when there is none).
        body: String,
        tags: Vec<String>,
        aliases: Vec<String>,
    }

    #[test]
    fn matches_shared_case_table() {
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../../tests/fixtures/frontmatter-cases.json"))
                .expect("valid fixture");
        for c in cases {
            assert_eq!(&c.md[body_start(&c.md)..], c.body, "body: {}", c.name);
            let tags: Vec<&str> = tag_ranges(&c.md).into_iter().map(|r| &c.md[r]).collect();
            assert_eq!(tags, c.tags, "tags: {}", c.name);
            assert_eq!(aliases(&c.md), c.aliases, "aliases: {}", c.name);
        }
    }
}
