//! `#tags` and `[[wikilinks]]` in note markdown (Obsidian syntax), plus entity slugs.
//!
//! One extractor feeds indexing (`note_refs`), the read-view renderer and rename
//! rewriting. It walks pulldown-cmark's events with source offsets, skips code, links,
//! autolinks and raw HTML, joins adjacent text ranges (pulldown-cmark splits text at
//! `[`, `]`, `*`, and around escapes) and scans the joined source text.
//!
//! The editor's twin is `src/lib/note-links.ts`; both pass the case table in
//! `tests/fixtures/note-links-cases.json`.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;

/// The markdown flavour notes are parsed with (same as the renderer).
pub fn markdown_options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefKind {
    Tag,
    Link,
}

impl RefKind {
    /// The `note_refs.Kind` value.
    pub fn as_str(self) -> &'static str {
        match self {
            RefKind::Tag => "tag",
            RefKind::Link => "link",
        }
    }
}

/// A tag or wikilink found in a note body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    pub kind: RefKind,
    /// Normalized target: the tag without `#`, or the trimmed title; lowercased.
    pub target: String,
    /// Byte range of the whole `#tag` or `[[…]]` in the source.
    pub range: Range<usize>,
    /// Byte range of the tag after `#`, or of the trimmed title inside the brackets.
    pub target_range: Range<usize>,
    /// Trimmed wikilink alias (`[[Title|alias]]`); `None` for tags and empty aliases.
    pub alias: Option<String>,
}

/// Letters and digits (any script), `_`, `-` and `/` (nesting).
pub fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

/// All tags and wikilinks in `src`, in source order.
pub fn extract(src: &str) -> Vec<Ref> {
    let mut out = Vec::new();
    // Nesting depth of links, images, code blocks and HTML blocks: no refs inside.
    let mut skip = 0usize;
    let mut run: Option<Range<usize>> = None;
    let flush = |run: &mut Option<Range<usize>>, out: &mut Vec<Ref>| {
        if let Some(r) = run.take() {
            scan_run(src, r, out);
        }
    };

    for (event, range) in Parser::new_ext(src, markdown_options()).into_offset_iter() {
        match event {
            Event::Start(
                Tag::Link { .. }
                | Tag::Image { .. }
                | Tag::CodeBlock(_)
                | Tag::HtmlBlock
                | Tag::MetadataBlock(_),
            ) => {
                flush(&mut run, &mut out);
                skip += 1;
            }
            Event::End(
                TagEnd::Link
                | TagEnd::Image
                | TagEnd::CodeBlock
                | TagEnd::HtmlBlock
                | TagEnd::MetadataBlock(_),
            ) => {
                flush(&mut run, &mut out);
                skip = skip.saturating_sub(1);
            }
            Event::Text(_) if skip == 0 => match run.as_mut() {
                // Adjacent, or separated only by an escape's backslash (`\|` in a table).
                Some(cur)
                    if cur.end == range.start
                        || (cur.end + 1 == range.start && src.as_bytes()[cur.end] == b'\\') =>
                {
                    cur.end = range.end;
                }
                _ => {
                    flush(&mut run, &mut out);
                    run = Some(range);
                }
            },
            _ => flush(&mut run, &mut out),
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Is the source byte before `pos` a backslash (an escaped `#` or `[`)?
fn escaped(src: &str, pos: usize) -> bool {
    pos > 0 && src.as_bytes()[pos - 1] == b'\\'
}

/// Scans one run of joined text for tags and wikilinks.
fn scan_run(src: &str, run: Range<usize>, out: &mut Vec<Ref>) {
    let base = run.start;
    let text = &src[run];
    let mut i = 0;
    let mut prev: Option<char> = None;
    while let Some(c) = text[i..].chars().next() {
        let rest = &text[i..];
        let abs = base + i;
        let token_start = match prev {
            None => true,
            Some(p) => p.is_whitespace(),
        };
        if token_start {
            // A bare URL (`scheme://…`) is one token: no tags or links inside it.
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            if rest[..end].contains("://") {
                prev = rest[..end].chars().last();
                i += end;
                continue;
            }
        }
        if rest.starts_with("[[") && prev != Some('!') && !escaped(src, abs) {
            if let Some(r) = parse_wikilink(rest, abs) {
                i += r.range.len();
                prev = Some(']');
                out.push(r);
                continue;
            }
        }
        let boundary = match prev {
            None => true,
            Some(p) => p.is_whitespace() || p == '(',
        };
        if c == '#' && boundary && !escaped(src, abs) {
            if let Some(r) = parse_tag(rest, abs) {
                i += r.range.len();
                prev = src[r.range.clone()].chars().last();
                out.push(r);
                continue;
            }
        }
        prev = Some(c);
        i += c.len_utf8();
    }
}

/// Parses `[[Title]]` / `[[Title|alias]]` at the start of `rest` (`abs` = its offset).
fn parse_wikilink(rest: &str, abs: usize) -> Option<Ref> {
    let close = rest[2..].find("]]")? + 2;
    let inner = &rest[2..close];
    if inner.contains(['[', ']', '#', '\n', '\r']) {
        return None;
    }
    let (title_part, alias) = match inner.split_once('|') {
        Some((_, alias)) if alias.contains('|') => return None,
        // In a table the separator is written `\|`.
        Some((title, alias)) => (title.strip_suffix('\\').unwrap_or(title), Some(alias)),
        None => (inner, None),
    };
    let title = title_part.trim();
    if title.is_empty() {
        return None;
    }
    let title_start = abs + 2 + (title_part.len() - title_part.trim_start().len());
    Some(Ref {
        kind: RefKind::Link,
        target: title.to_lowercase(),
        range: abs..abs + close + 2,
        target_range: title_start..title_start + title.len(),
        alias: alias
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(String::from),
    })
}

/// Parses `#tag` at the start of `rest` (`abs` = its offset). Trailing `/` is not part
/// of the tag; a tag must not start with `/` and must not be digits only.
fn parse_tag(rest: &str, abs: usize) -> Option<Ref> {
    let after = &rest[1..];
    let end = after.find(|c| !is_tag_char(c)).unwrap_or(after.len());
    let body = after[..end].trim_end_matches('/');
    if body.is_empty() || body.starts_with('/') || body.chars().all(char::is_numeric) {
        return None;
    }
    Some(Ref {
        kind: RefKind::Tag,
        target: body.to_lowercase(),
        range: abs..abs + 1 + body.len(),
        target_range: abs + 1..abs + 1 + body.len(),
        alias: None,
    })
}

/// The normalized tag when `term` (a search term) is a `#tag`, e.g. `#Acme,` → `acme`.
pub fn tag_term(term: &str) -> Option<String> {
    if !term.starts_with('#') {
        return None;
    }
    parse_tag(term, 0).map(|r| r.target)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Issue,
    Todo,
}

/// `issue/<id>` / `todo/<id>` (a normalized tag) → the entity it chips.
pub fn entity_tag(tag: &str) -> Option<(EntityKind, i64)> {
    let (kind, id) = tag.split_once('/')?;
    let kind = match kind {
        "issue" => EntityKind::Issue,
        "todo" => EntityKind::Todo,
        _ => return None,
    };
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    id.parse().ok().map(|id| (kind, id))
}

/// Does `tag` (normalized) match `filter` (normalized), itself or nested below it?
pub fn tag_matches(tag: &str, filter: &str) -> bool {
    tag == filter
        || tag
            .strip_prefix(filter)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Tag slug of a customer or project name: lowercased, every run of characters that
/// can't appear in a tag segment (anything but letters, digits and `_`) and any `-`
/// becomes one `-` (so `Acme - Corp` → `acme-corp`), leading/trailing `-` dropped.
/// `None` when nothing usable or digits only remain.
pub fn slug(name: &str) -> Option<String> {
    let mut out = String::with_capacity(name.len());
    let mut gap = false;
    for c in name.chars() {
        if c.is_alphanumeric() || c == '_' {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    let out = out.to_lowercase();
    if out.is_empty() || out.chars().all(char::is_numeric) {
        None
    } else {
        Some(out)
    }
}

/// Customer slugs that would collide with the chip namespaces (`#issue/<id>`, `#todo/<id>`).
pub const RESERVED_CUSTOMER_SLUGS: [&str; 2] = ["issue", "todo"];

/// Tag slug of a customer name: like `slug`, but `None` for a reserved slug
/// (`issue`, `todo`), so such a customer (and its projects) gets no tag.
pub fn customer_slug(name: &str) -> Option<String> {
    slug(name).filter(|s| !RESERVED_CUSTOMER_SLUGS.contains(&s.as_str()))
}

/// Applies non-overlapping `(range, replacement)` edits.
fn splice(body: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(body.len());
    let mut pos = 0;
    for (range, text) in edits {
        out.push_str(&body[pos..range.start]);
        out.push_str(&text);
        pos = range.end;
    }
    out.push_str(&body[pos..]);
    out
}

/// Retargets wikilinks to `old_target` (lowercased title) at `new_title`, keeping
/// aliases. `None` when nothing changed.
pub fn rewrite_links(body: &str, old_target: &str, new_title: &str) -> Option<String> {
    let edits: Vec<(Range<usize>, String)> = extract(body)
        .into_iter()
        .filter(|r| r.kind == RefKind::Link && r.target == old_target)
        .filter(|r| &body[r.target_range.clone()] != new_title)
        .map(|r| (r.target_range, new_title.to_string()))
        .collect();
    (!edits.is_empty()).then(|| splice(body, edits))
}

/// Rewrites tags at or below `old_path` (normalized, e.g. `acme` or `acme/web`): the
/// last segment of that path becomes `new_segment`; segments before it and nested
/// segments after it are kept as written (`#Acme/web/x` → `#Acme/portal/x`).
/// `None` when nothing changed.
pub fn rewrite_tags(body: &str, old_path: &str, new_segment: &str) -> Option<String> {
    let depth = old_path.split('/').count();
    let mut edits = Vec::new();
    for r in extract(body) {
        if r.kind != RefKind::Tag || !tag_matches(&r.target, old_path) {
            continue;
        }
        let tag = &body[r.target_range.clone()];
        // Byte range of segment `depth - 1` within the tag.
        let mut start = 0;
        for _ in 1..depth {
            start += tag[start..].find('/').map_or(tag.len(), |p| p + 1);
        }
        let end = tag[start..].find('/').map_or(tag.len(), |p| start + p);
        if &tag[start..end] != new_segment {
            let at = r.target_range.start;
            edits.push((at + start..at + end, new_segment.to_string()));
        }
    }
    (!edits.is_empty()).then(|| splice(body, edits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Cases {
        extract: Vec<ExtractCase>,
        slug: Vec<(String, Option<String>)>,
    }

    #[derive(Deserialize)]
    struct ExtractCase {
        name: String,
        md: String,
        refs: Vec<ExpectedRef>,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct ExpectedRef {
        kind: String,
        target: String,
        text: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        alias: Option<String>,
    }

    fn cases() -> Cases {
        serde_json::from_str(include_str!("../../tests/fixtures/note-links-cases.json")).unwrap()
    }

    fn actual(md: &str) -> Vec<ExpectedRef> {
        extract(md)
            .into_iter()
            .map(|r| ExpectedRef {
                kind: r.kind.as_str().to_string(),
                target: r.target.clone(),
                text: md[r.range.clone()].to_string(),
                title: (r.kind == RefKind::Link).then(|| md[r.target_range.clone()].to_string()),
                alias: r.alias,
            })
            .collect()
    }

    #[test]
    fn extract_matches_shared_case_table() {
        for case in cases().extract {
            assert_eq!(actual(&case.md), case.refs, "{}", case.name);
        }
    }

    #[test]
    fn slug_matches_shared_case_table() {
        for (name, expected) in cases().slug {
            assert_eq!(slug(&name), expected, "{name:?}");
        }
    }

    #[test]
    fn wikilink_inside_link_text_is_skipped() {
        // Not in the shared table: lezer (the editor) parses this nesting differently.
        assert!(extract("[#text [[In link]]](https://x.se/#frag)").is_empty());
    }

    #[test]
    fn tag_target_range_excludes_the_hash() {
        let md = "x #Acme/Web.";
        let r = &extract(md)[0];
        assert_eq!(&md[r.range.clone()], "#Acme/Web");
        assert_eq!(&md[r.target_range.clone()], "Acme/Web");
    }

    #[test]
    fn tag_term_parses_search_terms() {
        assert_eq!(tag_term("#Acme"), Some("acme".into()));
        assert_eq!(tag_term("#acme/web,"), Some("acme/web".into()));
        assert_eq!(tag_term("#123"), None);
        assert_eq!(tag_term("#"), None);
        assert_eq!(tag_term("acme"), None);
    }

    #[test]
    fn entity_tag_recognises_issue_and_todo_chips() {
        assert_eq!(entity_tag("issue/12"), Some((EntityKind::Issue, 12)));
        assert_eq!(entity_tag("todo/3"), Some((EntityKind::Todo, 3)));
        assert_eq!(entity_tag("issue/x"), None);
        assert_eq!(entity_tag("issue/12/a"), None);
        assert_eq!(entity_tag("issue/+1"), None);
        assert_eq!(entity_tag("acme"), None);
        assert_eq!(entity_tag("issue/99999999999999999999"), None);
    }

    #[test]
    fn tag_matches_itself_and_nested_tags_only() {
        assert!(tag_matches("acme", "acme"));
        assert!(tag_matches("acme/web", "acme"));
        assert!(tag_matches("acme/web/x", "acme/web"));
        assert!(!tag_matches("acmeco", "acme"));
        assert!(!tag_matches("acme", "acme/web"));
    }

    #[test]
    fn rewrite_links_keeps_aliases_and_skips_code() {
        let body = "[[Old]] [[old|alias]] [[ OLD ]] `[[Old]]` [[Older]] | x\n\n| a |\n|---|\n| [[Old\\|t]] |";
        let out = rewrite_links(body, "old", "New name").unwrap();
        assert_eq!(
            out,
            "[[New name]] [[New name|alias]] [[ New name ]] `[[Old]]` [[Older]] | x\n\n| a |\n|---|\n| [[New name\\|t]] |"
        );
        assert_eq!(rewrite_links(body, "missing", "X"), None);
        assert_eq!(rewrite_links("[[New]]", "new", "New"), None, "unchanged");
        assert_eq!(
            rewrite_links("[[new]]", "new", "New").as_deref(),
            Some("[[New]]"),
            "case-only rename"
        );
    }

    #[test]
    fn rewrite_tags_handles_nesting_case_and_partial_matches() {
        let body = "#acme #Acme/web #acme/web/x #acmeco `#acme` [#acme](https://a.se/#acme)";
        assert_eq!(
            rewrite_tags(body, "acme", "globex").unwrap(),
            "#globex #globex/web #globex/web/x #acmeco `#acme` [#acme](https://a.se/#acme)"
        );
        assert_eq!(
            rewrite_tags(body, "acme/web", "portal").unwrap(),
            "#acme #Acme/portal #acme/portal/x #acmeco `#acme` [#acme](https://a.se/#acme)"
        );
        assert_eq!(rewrite_tags(body, "globex", "x"), None);
        assert_eq!(rewrite_tags("#acme", "acme", "acme"), None, "unchanged");
    }
}
