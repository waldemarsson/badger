//! Markdown → HTML for the notes read view and "Copy as HTML".
//!
//! The WebView has no CSP, so the output is locked down here: raw HTML is shown as
//! text, only `http`, `https` and `mailto` links survive, and images never load.

use pulldown_cmark::{html, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};

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

/// Renders note markdown (tables, strikethrough, task lists) to safe HTML.
pub fn to_html(markdown: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut open: Vec<Open> = Vec::new();
    let mut events: Vec<Event> = Vec::new();
    // Number of `<a>` currently open; a nested anchor would be invalid HTML.
    let anchors = |open: &[Open]| {
        open.iter()
            .filter(|o| matches!(o, Open::Anchor | Open::ImageAnchor { .. }))
            .count()
    };

    for event in Parser::new_ext(markdown, options) {
        match event {
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

/// Renders markdown for the read view and "Copy as HTML". Async so a long note
/// doesn't render on the main thread.
#[tauri::command]
pub async fn render_markdown(markdown: String) -> String {
    to_html(&markdown)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
