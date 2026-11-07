//! Markdown → HTML for the notes read view and "Copy as HTML".
//!
//! Copy mode output leaves the app, and the read view must not lean on the CSP alone, so
//! the output is locked down here: raw HTML is shown as text, only `http`, `https` and
//! `mailto` links survive, and images never load.
//!
//! `#tags` and `[[wikilinks]]` (see `note_links`) become `<a href="#">` with data
//! attributes in the read view, which the reader dispatches on: `data-tag` (normalized
//! tag) for tags and chips, `data-note` (title as written) plus `data-note-id` or
//! `data-unresolved` for wikilinks. `#issue/<id>` and `#todo/<id>` render as chips with
//! the entity's title (`note-chip-todo` also carries `data-status`); unknown ids stay
//! plain tags. Copy mode writes all of them as plain text.

use crate::note_links::{self, EntityKind, Ref, RefKind};
use crate::AppError;
use pulldown_cmark::{html, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};
use serde::Deserialize;
use sqlx::{Pool, Sqlite};
use std::collections::{BTreeSet, HashMap};
use tauri::State;

const ALLOWED_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

/// Is `url` an absolute `http:`, `https:` or `mailto:` URL (scheme in any case)?
fn is_allowed_url(url: &str) -> bool {
    let url = url.trim_start();
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };
    ALLOWED_SCHEMES
        .iter()
        .any(|s| scheme.eq_ignore_ascii_case(s))
}

fn link_allowed(link_type: LinkType, url: &str) -> bool {
    // `<user@example.com>` arrives without `mailto:`; push_html adds it.
    link_type == LinkType::Email || is_allowed_url(url)
}

/// An open link or image whose end tag still has to be handled.
enum Open {
    /// A link emitted as `<a>`.
    Anchor,
    /// A link dropped for its URL: its text is kept, the tags are not.
    Dropped,
    /// An image turned into `<a>`; `url` stands in for an empty alt text.
    ImageAnchor { url: String, has_text: bool },
    /// An image that renders as its alt text only.
    ImageText,
}

/// `render_markdown` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RenderMode {
    /// The read view: tags, chips and wikilinks are clickable anchors.
    Read,
    /// "Copy as HTML": tags as `#tag`, wikilinks as alias or title, chips as the title.
    Copy,
}

/// What a note's wikilinks and chips resolve to.
#[derive(Debug, Default)]
pub struct Resolver {
    /// Lowercased title → (id, title).
    pub notes: HashMap<String, (i64, String)>,
    /// Issue id → title.
    pub issues: HashMap<i64, String>,
    /// Todo id → (title, status).
    pub todos: HashMap<i64, (String, String)>,
}

impl Resolver {
    /// Loads what `refs` need: note titles when there are wikilinks, and the issues and
    /// todos that chips reference.
    pub async fn load(refs: &[Ref], pool: &Pool<Sqlite>) -> Result<Self, AppError> {
        let mut resolver = Resolver::default();
        if refs.iter().any(|r| r.kind == RefKind::Link) {
            let notes: Vec<(i64, String)> = sqlx::query_as("SELECT id, Title FROM notes")
                .fetch_all(pool)
                .await?;
            for (id, title) in notes {
                resolver
                    .notes
                    .entry(title.to_lowercase())
                    .or_insert((id, title));
            }
        }
        let mut issues = BTreeSet::new();
        let mut todos = BTreeSet::new();
        for r in refs.iter().filter(|r| r.kind == RefKind::Tag) {
            match note_links::entity_tag(&r.target) {
                Some((EntityKind::Issue, id)) => issues.insert(id),
                Some((EntityKind::Todo, id)) => todos.insert(id),
                None => false,
            };
        }
        for id in issues {
            let title: Option<String> = sqlx::query_scalar("SELECT Title FROM issues WHERE id = ?")
                .bind(id)
                .fetch_optional(pool)
                .await?;
            if let Some(title) = title {
                resolver.issues.insert(id, title);
            }
        }
        for id in todos {
            let todo: Option<(String, String)> =
                sqlx::query_as("SELECT Title, Status FROM todos WHERE id = ?")
                    .bind(id)
                    .fetch_optional(pool)
                    .await?;
            if let Some(todo) = todo {
                resolver.todos.insert(id, todo);
            }
        }
        Ok(resolver)
    }
}

/// Escapes text for an HTML attribute value or element content.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// The events standing in for one tag or wikilink.
fn ref_event(markdown: &str, r: &Ref, mode: RenderMode, resolver: &Resolver) -> Event<'static> {
    let written = &markdown[r.range.clone()];
    let target_text = &markdown[r.target_range.clone()];
    match r.kind {
        RefKind::Tag => {
            let chip = match note_links::entity_tag(&r.target) {
                Some((EntityKind::Issue, id)) => resolver
                    .issues
                    .get(&id)
                    .map(|title| (title.as_str(), "note-chip note-chip-issue", None)),
                Some((EntityKind::Todo, id)) => resolver.todos.get(&id).map(|(title, status)| {
                    (
                        title.as_str(),
                        "note-chip note-chip-todo",
                        Some(status.as_str()),
                    )
                }),
                None => None,
            };
            match (mode, chip) {
                (RenderMode::Copy, Some((title, _, _))) => {
                    Event::Text(CowStr::from(title.to_string()))
                }
                (RenderMode::Copy, None) => Event::Text(CowStr::from(written.to_string())),
                (RenderMode::Read, Some((title, class, status))) => {
                    let status = status
                        .map(|s| format!(" data-status=\"{}\"", escape(s)))
                        .unwrap_or_default();
                    Event::InlineHtml(CowStr::from(format!(
                        "<a href=\"#\" class=\"{class}\" data-tag=\"{}\"{status}>{}</a>",
                        escape(&r.target),
                        escape(title)
                    )))
                }
                (RenderMode::Read, None) => Event::InlineHtml(CowStr::from(format!(
                    "<a href=\"#\" class=\"note-tag\" data-tag=\"{}\">{}</a>",
                    escape(&r.target),
                    escape(written)
                ))),
            }
        }
        RefKind::Link => {
            let text = r.alias.as_deref().unwrap_or(target_text);
            if mode == RenderMode::Copy {
                return Event::Text(CowStr::from(text.to_string()));
            }
            let resolved = match resolver.notes.get(&r.target) {
                Some((id, _)) => format!(" data-note-id=\"{id}\""),
                None => " data-unresolved=\"\"".to_string(),
            };
            Event::InlineHtml(CowStr::from(format!(
                "<a href=\"#\" class=\"note-wikilink\" data-note=\"{}\"{resolved}>{}</a>",
                escape(target_text),
                escape(text)
            )))
        }
    }
}

/// Renders note markdown (tables, strikethrough, task lists) to safe HTML; `refs` (from
/// `note_links::extract(markdown)`) are rendered per `mode` and resolved by `resolver`.
pub fn render(markdown: &str, refs: &[Ref], mode: RenderMode, resolver: &Resolver) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    // Refs not yet passed (both in source order).
    let mut next_ref = 0;
    let mut open: Vec<Open> = Vec::new();
    let mut events: Vec<Event> = Vec::new();
    // Number of `<a>` currently open; a nested anchor would be invalid HTML.
    let anchors = |open: &[Open]| {
        open.iter()
            .filter(|o| matches!(o, Open::Anchor | Open::ImageAnchor { .. }))
            .count()
    };

    for (event, range) in Parser::new_ext(markdown, options).into_offset_iter() {
        match event {
            // Text in a tag or wikilink: pulldown-cmark may split one ref over several
            // text events (`[`, `[`, `Foo`, `]`, `]`); the ref is emitted where it starts.
            Event::Text(text) => {
                while refs
                    .get(next_ref)
                    .is_some_and(|r| r.range.end <= range.start)
                {
                    next_ref += 1;
                }
                if !refs
                    .get(next_ref)
                    .is_some_and(|r| r.range.start < range.end)
                {
                    if let Some(Open::ImageAnchor { has_text, .. }) = open.last_mut() {
                        *has_text = true;
                    }
                    events.push(Event::Text(text));
                    continue;
                }
                let overlapping = refs[next_ref..]
                    .iter()
                    .take_while(|r| r.range.start < range.end);
                if text.as_ref() != &markdown[range.clone()] {
                    // An entity or escape: it is all inside a ref or all outside one.
                    if !overlapping
                        .clone()
                        .any(|r| r.range.start <= range.start && range.end <= r.range.end)
                    {
                        events.push(Event::Text(text));
                    }
                    continue;
                }
                let mut pos = range.start;
                for r in overlapping {
                    if r.range.start > pos {
                        let before = markdown[pos..r.range.start].to_string();
                        events.push(Event::Text(CowStr::from(before)));
                    }
                    if r.range.start >= range.start {
                        events.push(ref_event(markdown, r, mode, resolver));
                    }
                    pos = pos.max(r.range.end.min(range.end));
                }
                if pos < range.end {
                    let after = markdown[pos..range.end].to_string();
                    events.push(Event::Text(CowStr::from(after)));
                }
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                if anchors(&open) == 0 && link_allowed(link_type, &dest_url) {
                    open.push(Open::Anchor);
                    events.push(Event::Start(Tag::Link {
                        link_type,
                        dest_url,
                        title,
                        id,
                    }));
                } else {
                    open.push(Open::Dropped);
                }
            }
            Event::Start(Tag::Image {
                dest_url,
                title,
                id,
                ..
            }) => {
                if anchors(&open) == 0 && is_allowed_url(&dest_url) {
                    open.push(Open::ImageAnchor {
                        url: dest_url.to_string(),
                        has_text: false,
                    });
                    events.push(Event::Start(Tag::Link {
                        link_type: LinkType::Inline,
                        dest_url,
                        title,
                        id,
                    }));
                } else {
                    open.push(Open::ImageText);
                }
            }
            Event::End(TagEnd::Link | TagEnd::Image) => match open.pop() {
                Some(Open::Anchor) => events.push(Event::End(TagEnd::Link)),
                Some(Open::ImageAnchor { url, has_text }) => {
                    if !has_text {
                        events.push(Event::Text(CowStr::from(url)));
                    }
                    events.push(Event::End(TagEnd::Link));
                }
                Some(Open::Dropped | Open::ImageText) | None => {}
            },
            // Raw HTML is shown as text; an HTML block becomes a paragraph.
            Event::Start(Tag::HtmlBlock) => events.push(Event::Start(Tag::Paragraph)),
            Event::End(TagEnd::HtmlBlock) => {
                if matches!(events.last(), Some(Event::SoftBreak)) {
                    events.pop();
                }
                events.push(Event::End(TagEnd::Paragraph));
            }
            Event::Html(raw) => {
                let line = raw.trim_end_matches('\n');
                if !line.is_empty() {
                    events.push(Event::Text(CowStr::from(line.to_string())));
                }
                events.push(Event::SoftBreak);
            }
            Event::InlineHtml(raw) => events.push(Event::Text(raw)),
            other => {
                if let Some(Open::ImageAnchor { has_text, .. }) = open.last_mut() {
                    if matches!(other, Event::Text(_) | Event::Code(_)) {
                        *has_text = true;
                    }
                }
                events.push(other);
            }
        }
    }

    let mut out = String::with_capacity(markdown.len() * 3 / 2);
    html::push_html(&mut out, events.into_iter());
    out
}

/// Plain text for the search index: text, code and raw HTML as written; markers, link
/// URLs and image URLs dropped (image alt text kept). Blocks end with a newline and table
/// cells with a space, so words from neighbouring blocks don't run together.
pub fn to_plain_text(markdown: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut out = String::with_capacity(markdown.len());
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => {
                out.push_str(&t)
            }
            Event::SoftBreak | Event::HardBreak | Event::Rule => out.push('\n'),
            // A block starting inside another (a nested list or quote in an item) must not
            // run into the parent's text.
            Event::Start(
                Tag::Paragraph
                | Tag::Heading { .. }
                | Tag::BlockQuote(_)
                | Tag::CodeBlock(_)
                | Tag::List(_)
                | Tag::Item
                | Tag::Table(_),
            ) if !out.is_empty() && !out.ends_with('\n') => out.push('\n'),
            Event::End(TagEnd::TableCell) => out.push(' '),
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::Item
                | TagEnd::TableRow
                | TagEnd::TableHead
                | TagEnd::CodeBlock,
            ) if !out.ends_with('\n') => out.push('\n'),
            _ => {}
        }
    }
    out.trim_end().to_string()
}

/// Renders markdown for the read view (`Read`) or "Copy as HTML" (`Copy`), resolving
/// wikilinks and chips against the database.
pub async fn inner_render_markdown(
    markdown: &str,
    mode: RenderMode,
    pool: &Pool<Sqlite>,
) -> Result<String, AppError> {
    let refs = note_links::extract(markdown);
    let resolver = Resolver::load(&refs, pool).await?;
    Ok(render(markdown, &refs, mode, &resolver))
}

/// Async so a long note doesn't render on the main thread.
#[tauri::command]
pub async fn render_markdown(
    markdown: String,
    mode: RenderMode,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<String, AppError> {
    inner_render_markdown(&markdown, mode, pool.inner()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rendering without tags and wikilinks (plain markdown).
    fn to_html(markdown: &str) -> String {
        render(markdown, &[], RenderMode::Read, &Resolver::default())
    }

    #[test]
    fn renders_basic_elements() {
        let html = to_html("# Title\n\nSome *em* and **strong** and ~~gone~~ and `code`.\n\n> quote\n\n- a\n- b\n\n1. one\n\n---\n");
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<em>em</em>"));
        assert!(html.contains("<strong>strong</strong>"));
        assert!(html.contains("<del>gone</del>"));
        assert!(html.contains("<code>code</code>"));
        assert!(html.contains("<blockquote>"));
        assert!(html.contains("<ul>\n<li>a</li>"));
        assert!(html.contains("<ol>"));
        assert!(html.contains("<hr />"));
    }

    #[test]
    fn renders_fenced_code_with_language_class() {
        let html = to_html("```rust\nfn main() {}\n```\n");
        assert!(html.contains(r#"<pre><code class="language-rust">fn main() {}"#));
    }

    #[test]
    fn escapes_code_block_language() {
        let html = to_html("```\"><script>\nx\n```\n");
        assert!(!html.contains("<script"));
    }

    #[test]
    fn renders_tables_with_alignment() {
        let html = to_html("| a | b |\n|:--|--:|\n| 1 | 2 |\n");
        assert!(html.contains("<table>"));
        assert!(html.contains(r#"<th style="text-align: left">a</th>"#));
        assert!(html.contains(r#"<td style="text-align: right">2</td>"#));
    }

    #[test]
    fn renders_task_lists_as_disabled_checkboxes() {
        let html = to_html("- [ ] open\n- [x] done\n");
        assert!(html.contains(r#"<input disabled="" type="checkbox"/>"#));
        assert!(html.contains(r#"<input disabled="" type="checkbox" checked=""/>"#));
    }

    #[test]
    fn does_not_render_footnotes() {
        // Without the extension, `[^1]: note` is a link reference definition (to the
        // relative URL `note`, so the reference renders as plain text).
        let html = to_html("Text[^1]\n\n[^1]: note\n");
        assert!(!html.contains("footnote"), "{html}");
        assert!(!html.contains("<sup"), "{html}");
        assert!(!html.contains("<a"), "{html}");
        let html = to_html("Text[^2]\n");
        assert_eq!(html, "<p>Text[^2]</p>\n");
    }

    #[test]
    fn escapes_block_html() {
        let html = to_html("<script>alert(1)</script>\n\nafter\n");
        assert!(!html.contains("<script"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(html.contains("<p>after</p>"));
    }

    #[test]
    fn escapes_multiline_block_html_as_one_paragraph() {
        let html = to_html("<details>\n<summary>Hi</summary>\n</details>\n");
        assert!(!html.contains("<details"));
        assert!(html.starts_with(
            "<p>&lt;details&gt;\n&lt;summary&gt;Hi&lt;/summary&gt;\n&lt;/details&gt;</p>"
        ));
    }

    #[test]
    fn escapes_inline_html() {
        let html = to_html("a <img src=x onerror=alert(1)> b <kbd>K</kbd>\n");
        assert!(!html.contains("<img"));
        assert!(!html.contains("<kbd"));
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
    }

    #[test]
    fn keeps_allowed_links() {
        for url in [
            "http://a.se",
            "https://a.se/x?y=1#z",
            "HTTPS://A.SE",
            "mailto:me@a.se",
        ] {
            let html = to_html(&format!("[t]({url})"));
            assert!(
                html.contains(&format!(r#"<a href="{url}">t</a>"#)),
                "{url}: {html}"
            );
        }
    }

    #[test]
    fn drops_disallowed_links_to_text() {
        for url in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "data:text/html,x",
            "file:///etc/passwd",
            "vbscript:x",
            "foo.md",
            "#x",
            "//evil.com",
        ] {
            let html = to_html(&format!("[the *text*](<{url}>)"));
            assert!(!html.contains("<a"), "{url}: {html}");
            assert_eq!(html, "<p>the <em>text</em></p>\n", "{url}");
        }
    }

    #[test]
    fn autolinks_follow_the_same_rules() {
        assert!(to_html("<https://a.se>").contains(r#"<a href="https://a.se">https://a.se</a>"#));
        assert!(to_html("<me@a.se>").contains(r#"<a href="mailto:me@a.se">me@a.se</a>"#));
        let html = to_html("<javascript:alert(1)>");
        assert!(!html.contains("<a"), "{html}");
    }

    #[test]
    fn reference_links_follow_the_same_rules() {
        let html = to_html("[ok][1] [bad][2]\n\n[1]: https://a.se\n[2]: javascript:x\n");
        assert!(html.contains(r#"<a href="https://a.se">ok</a>"#));
        assert!(html.contains(" bad</p>"));
        assert!(!html.contains("javascript"));
    }

    #[test]
    fn images_become_links_and_never_load() {
        let html = to_html("![A cat](https://a.se/cat.png)");
        assert_eq!(html, "<p><a href=\"https://a.se/cat.png\">A cat</a></p>\n");

        let html = to_html("![](https://a.se/cat.png)");
        assert!(html.contains(r#"<a href="https://a.se/cat.png">https://a.se/cat.png</a>"#));

        let html = to_html("![A cat](cat.png) ![x](data:image/png;base64,AA)");
        assert_eq!(html, "<p>A cat x</p>\n");
        assert!(!html.contains("<img"));
    }

    #[test]
    fn image_inside_link_does_not_nest_anchors() {
        let html = to_html("[![alt](https://a.se/i.png)](https://b.se)");
        assert_eq!(html, "<p><a href=\"https://b.se\">alt</a></p>\n");
    }

    #[test]
    fn empty_input_renders_nothing() {
        assert_eq!(to_html(""), "");
        assert_eq!(to_html("   \n\n"), "");
    }

    #[test]
    fn plain_text_drops_markers_and_urls() {
        let text = to_plain_text(
            "# Head\n\nSome **bold** [link](https://example.com/secret) ![alt](https://x.test/i.png)\n\n- [ ] task\n\n| a | b |\n| - | - |\n| c | d |\n\n```rust\nlet x = 1;\n```\n",
        );
        assert_eq!(
            text,
            "Head\nSome bold link alt\ntask\na b \nc d \nlet x = 1;"
        );
        assert!(!text.contains("example.com"));
    }

    #[test]
    fn plain_text_separates_nested_blocks() {
        assert_eq!(to_plain_text("- parent\n  - child"), "parent\nchild");
        assert_eq!(to_plain_text("1. one\n   > quoted"), "one\nquoted");
    }

    #[test]
    fn plain_text_of_empty_input_is_empty() {
        assert_eq!(to_plain_text(""), "");
    }

    // ── Tags and wikilinks ────────────────────────────────────────────────────

    fn resolver() -> Resolver {
        let mut r = Resolver::default();
        r.notes.insert("plan".into(), (3, "Plan".into()));
        r.notes
            .insert("a \"q\" & <co".into(), (4, "A \"q\" & <co".into()));
        r.issues.insert(12, "Login <bug>".into());
        r.todos.insert(5, ("Call Åsa".into(), "in_progress".into()));
        r
    }

    fn render_with(markdown: &str, mode: RenderMode) -> String {
        render(markdown, &note_links::extract(markdown), mode, &resolver())
    }

    #[test]
    fn read_mode_renders_tags_and_wikilinks_as_data_anchors() {
        let html = render_with(
            "See #Acme/Web, [[plan]] and [[Plan|the plan]] or [[Missing]].",
            RenderMode::Read,
        );
        assert_eq!(
            html,
            "<p>See <a href=\"#\" class=\"note-tag\" data-tag=\"acme/web\">#Acme/Web</a>, \
             <a href=\"#\" class=\"note-wikilink\" data-note=\"plan\" data-note-id=\"3\">plan</a> and \
             <a href=\"#\" class=\"note-wikilink\" data-note=\"Plan\" data-note-id=\"3\">the plan</a> or \
             <a href=\"#\" class=\"note-wikilink\" data-note=\"Missing\" data-unresolved=\"\">Missing</a>.</p>\n"
        );
    }

    #[test]
    fn read_mode_renders_chips_and_unknown_ids_as_plain_tags() {
        let html = render_with("#issue/12 #todo/5 #todo/99 #issue/7", RenderMode::Read);
        assert_eq!(
            html,
            "<p><a href=\"#\" class=\"note-chip note-chip-issue\" data-tag=\"issue/12\">Login &lt;bug&gt;</a> \
             <a href=\"#\" class=\"note-chip note-chip-todo\" data-tag=\"todo/5\" data-status=\"in_progress\">Call Åsa</a> \
             <a href=\"#\" class=\"note-tag\" data-tag=\"todo/99\">#todo/99</a> \
             <a href=\"#\" class=\"note-tag\" data-tag=\"issue/7\">#issue/7</a></p>\n"
        );
    }

    #[test]
    fn read_mode_escapes_titles_and_keeps_raw_html_escaped() {
        let html = render_with(
            "[[A \"q\" & <co]] <b onclick=x>#t</b> `#code` &amp; #x",
            RenderMode::Read,
        );
        assert!(html.contains(
            "<a href=\"#\" class=\"note-wikilink\" data-note=\"A &quot;q&quot; &amp; &lt;co\" data-note-id=\"4\">A &quot;q&quot; &amp; &lt;co</a>"
        ), "{html}");
        assert!(html.contains("&lt;b onclick=x&gt;<a href=\"#\" class=\"note-tag\" data-tag=\"t\">#t</a>&lt;/b&gt;"), "{html}");
        assert!(html.contains("<code>#code</code> &amp; <a"), "{html}");
        assert!(!html.contains("<b"), "{html}");
    }

    #[test]
    fn read_mode_handles_tables_headings_and_formatting() {
        let html = render_with(
            "# Title #tag\n\n| a |\n|---|\n| [[Plan\\|alias]] x |\n\n*[[plan]]* [ok](https://a.se/#x)",
            RenderMode::Read,
        );
        assert!(
            html.contains(
                "<h1>Title <a href=\"#\" class=\"note-tag\" data-tag=\"tag\">#tag</a></h1>"
            ),
            "{html}"
        );
        assert!(html.contains("<td><a href=\"#\" class=\"note-wikilink\" data-note=\"Plan\" data-note-id=\"3\">alias</a> x</td>"), "{html}");
        assert!(
            html.contains("<em><a href=\"#\" class=\"note-wikilink\" data-note=\"plan\""),
            "{html}"
        );
        assert!(
            html.contains("<a href=\"https://a.se/#x\">ok</a>"),
            "{html}"
        );
    }

    #[test]
    fn copy_mode_writes_tags_links_and_chips_as_plain_text() {
        let html = render_with(
            "#Acme [[Plan|the plan]] [[Missing]] #issue/12 #todo/5 #todo/99 <i>",
            RenderMode::Copy,
        );
        assert_eq!(
            html,
            "<p>#Acme the plan Missing Login &lt;bug&gt; Call Åsa #todo/99 &lt;i&gt;</p>\n"
        );
    }

    #[tokio::test]
    async fn inner_render_resolves_against_the_database() {
        let pool = crate::test_db().await;
        sqlx::raw_sql(
            "INSERT INTO customers (id, Name) VALUES (1, 'Acme');
             INSERT INTO projects (id, CustomerId, Name) VALUES (1, 1, 'Web');
             INSERT INTO issues (id, ProjectId, Title) VALUES (12, 1, 'Login bug');
             INSERT INTO todos (id, Title, Status, CreatedAt) VALUES (5, 'Ship', 'done', 'x');
             INSERT INTO notes (id, Title, CreatedAt, UpdatedAt) VALUES (3, 'Åsa', 'x', 'x');",
        )
        .execute(&pool)
        .await
        .unwrap();
        let html = inner_render_markdown("[[ÅSA]] #issue/12 #todo/5", RenderMode::Read, &pool)
            .await
            .unwrap();
        assert!(
            html.contains("data-note=\"ÅSA\" data-note-id=\"3\">ÅSA</a>"),
            "{html}"
        );
        assert!(
            html.contains("data-tag=\"issue/12\">Login bug</a>"),
            "{html}"
        );
        assert!(html.contains("data-status=\"done\">Ship</a>"), "{html}");
        let copy = inner_render_markdown("[[ÅSA]] #todo/5", RenderMode::Copy, &pool)
            .await
            .unwrap();
        assert_eq!(copy, "<p>ÅSA Ship</p>\n");
    }
}
