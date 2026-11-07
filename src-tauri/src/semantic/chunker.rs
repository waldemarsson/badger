//! Heading-aware chunking of a note for embedding.
//!
//! The body is split on markdown headings (pulldown-cmark events, not plain text, so a
//! `#` inside code is not a heading). Within a section, blocks (paragraphs, list items,
//! code blocks, table rows) are packed into chunks of about `max_tokens` tokens, prefix
//! included; a block too long on its own is split between words. No overlap. Each chunk's
//! model input starts with the note title and its heading path.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// Target chunk size in tokens (prefix included).
pub const CHUNK_TOKENS: usize = 200;
/// Body budget floor when a long title or heading path eats the chunk budget.
const MIN_BODY_TOKENS: usize = 32;
/// Separator between heading levels in the prefix.
const PATH_SEPARATOR: &str = " › ";

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// Model input (before `passage_text`): title, heading path, body.
    pub input: String,
    /// The body text alone: the semantic snippet. Empty for a title-only chunk.
    pub text: String,
}

/// Splits a note into chunks. Always returns at least one chunk (title only when the
/// body has no text). `count` returns a text's token count.
pub fn chunk_note(
    title: &str,
    body: &str,
    max_tokens: usize,
    count: &dyn Fn(&str) -> usize,
) -> Vec<Chunk> {
    let mut b = Builder {
        title: clean(title),
        max_tokens,
        count,
        headings: Vec::new(),
        heading: None,
        unit: String::new(),
        units: Vec::new(),
        chunks: Vec::new(),
    };
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(body, options) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                b.flush_unit();
                b.flush_section();
                b.heading = Some((level as usize, String::new()));
            }
            Event::End(TagEnd::Heading(level)) => {
                if let Some((_, text)) = b.heading.take() {
                    let level = level as usize;
                    b.headings.retain(|(l, _)| *l < level);
                    let text = collapse(&text);
                    if !text.is_empty() {
                        b.headings.push((level, text));
                    }
                }
            }
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => b.push(&t),
            Event::SoftBreak | Event::HardBreak => b.push(" "),
            Event::End(TagEnd::TableCell) => b.push(" "),
            Event::Rule
            | Event::Start(
                Tag::Paragraph
                | Tag::BlockQuote(_)
                | Tag::CodeBlock(_)
                | Tag::List(_)
                | Tag::Item
                | Tag::Table(_),
            )
            | Event::End(
                TagEnd::Paragraph
                | TagEnd::Item
                | TagEnd::TableRow
                | TagEnd::TableHead
                | TagEnd::CodeBlock,
            ) => b.flush_unit(),
            _ => {}
        }
    }
    b.flush_unit();
    b.flush_section();
    if b.chunks.is_empty() {
        // A body of headings only keeps them searchable (prefix = title + headings).
        let input = b.prefix();
        b.chunks.push(Chunk {
            input,
            text: String::new(),
        });
    }
    b.chunks
}

struct Builder<'a> {
    title: String,
    max_tokens: usize,
    count: &'a dyn Fn(&str) -> usize,
    /// Open headings above the current section: (level, text).
    headings: Vec<(usize, String)>,
    /// The heading being read, if inside one.
    heading: Option<(usize, String)>,
    /// The block being read.
    unit: String,
    /// Finished blocks of the current section.
    units: Vec<String>,
    chunks: Vec<Chunk>,
}

impl Builder<'_> {
    fn push(&mut self, text: &str) {
        match &mut self.heading {
            Some((_, h)) => h.push_str(text),
            None => self.unit.push_str(text),
        }
    }

    fn flush_unit(&mut self) {
        let unit = collapse(&std::mem::take(&mut self.unit));
        if !unit.is_empty() {
            self.units.push(unit);
        }
    }

    fn prefix(&self) -> String {
        if self.headings.is_empty() {
            return self.title.clone();
        }
        let path: Vec<&str> = self.headings.iter().map(|(_, h)| h.as_str()).collect();
        format!("{}\n{}", self.title, path.join(PATH_SEPARATOR))
    }

    /// Packs the section's blocks into chunks under the current heading path.
    fn flush_section(&mut self) {
        let units = std::mem::take(&mut self.units);
        if units.is_empty() {
            return;
        }
        let prefix = self.prefix();
        let budget = self
            .max_tokens
            .saturating_sub((self.count)(&prefix))
            .max(MIN_BODY_TOKENS);
        let mut parts: Vec<String> = Vec::new();
        let mut used = 0;
        for unit in units {
            let tokens = (self.count)(&unit);
            if tokens > budget {
                self.emit(&prefix, &mut parts);
                used = 0;
                for piece in split_words(&unit, budget, self.count) {
                    parts.push(piece);
                    self.emit(&prefix, &mut parts);
                }
                continue;
            }
            if used + tokens > budget {
                self.emit(&prefix, &mut parts);
                used = 0;
            }
            used += tokens;
            parts.push(unit);
        }
        self.emit(&prefix, &mut parts);
    }

    fn emit(&mut self, prefix: &str, parts: &mut Vec<String>) {
        if parts.is_empty() {
            return;
        }
        let text = parts.join("\n");
        parts.clear();
        self.chunks.push(Chunk {
            input: format!("{prefix}\n\n{text}"),
            text,
        });
    }
}

/// Splits `text` between words into pieces of at most `budget` tokens (a single word
/// longer than the budget is its own piece; the model truncates it).
fn split_words(text: &str, budget: usize, count: &dyn Fn(&str) -> usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut used = 0;
    for word in text.split_whitespace() {
        let tokens = count(word).max(1);
        if used + tokens > budget && !current.is_empty() {
            pieces.push(std::mem::take(&mut current));
            used = 0;
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
        used += tokens;
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

/// Strips highlight markers (they must only come from FTS5) and collapses whitespace.
fn collapse(text: &str) -> String {
    clean(text).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clean(text: &str) -> String {
    text.replace(
        [
            crate::commands::notes::HIT_START,
            crate::commands::notes::HIT_END,
        ],
        "",
    )
    .trim()
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> usize {
        text.split_whitespace().count()
    }

    fn chunks(title: &str, body: &str, max: usize) -> Vec<Chunk> {
        chunk_note(title, body, max, &words)
    }

    #[test]
    fn empty_and_whitespace_bodies_yield_one_title_only_chunk() {
        for body in ["", "   \n\n  ", "---"] {
            assert_eq!(
                chunks("Plan", body, 200),
                vec![Chunk {
                    input: "Plan".into(),
                    text: String::new()
                }],
                "body {body:?}"
            );
        }
    }

    #[test]
    fn headings_only_body_keeps_the_headings() {
        let c = chunks("Plan", "# Goals\n## Q1", 200);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].input, "Plan\nGoals › Q1");
        assert_eq!(c[0].text, "");
    }

    #[test]
    fn splits_on_headings_and_prefixes_title_and_heading_path() {
        let body = "Intro line.\n\n# Goals\nShip it.\n\n## Q1\n- hire\n- plan\n\n# Risks\nLate.";
        let c = chunks("Roadmap", body, 200);
        let inputs: Vec<&str> = c.iter().map(|c| c.input.as_str()).collect();
        assert_eq!(
            inputs,
            vec![
                "Roadmap\n\nIntro line.",
                "Roadmap\nGoals\n\nShip it.",
                "Roadmap\nGoals › Q1\n\nhire\nplan",
                "Roadmap\nRisks\n\nLate.",
            ]
        );
        assert_eq!(c[2].text, "hire\nplan");
    }

    #[test]
    fn hash_in_code_is_not_a_heading() {
        let c = chunks("Code", "```sh\n# not a heading\necho hi\n```", 200);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].text, "# not a heading echo hi");
    }

    #[test]
    fn packs_blocks_up_to_the_budget_without_overlap() {
        // 10 paragraphs of 30 words; budget 100 incl. the 1-word prefix → 3 per chunk.
        let para = vec!["word"; 30].join(" ");
        let body = vec![para.as_str(); 10].join("\n\n");
        let c = chunks("T", &body, 100);
        assert_eq!(c.len(), 4);
        for chunk in &c {
            assert!(words(&chunk.input) <= 100, "{}", words(&chunk.input));
        }
        let total: usize = c.iter().map(|c| words(&c.text)).sum();
        assert_eq!(total, 300, "no overlap, nothing lost");
    }

    #[test]
    fn very_long_paragraph_is_split_between_words() {
        let body: Vec<String> = (0..1000).map(|i| format!("w{i}")).collect();
        let c = chunks("Long", &body.join(" "), 200);
        assert!(c.len() >= 5);
        for chunk in &c {
            assert!(words(&chunk.input) <= 200);
            assert!(chunk.input.starts_with("Long\n\n"));
        }
        assert_eq!(c.iter().map(|c| words(&c.text)).sum::<usize>(), 1000);
        assert!(c[0].text.starts_with("w0 w1"));
        assert!(c.last().unwrap().text.ends_with("w999"));
    }

    #[test]
    fn keeps_unicode_and_strips_highlight_markers() {
        let c = chunks("Möte 🚀", "Åsa \u{2}räknar\u{3} på äppelpriser ✅", 200);
        assert_eq!(c[0].input, "Möte 🚀\n\nÅsa räknar på äppelpriser ✅");
        assert_eq!(c[0].text, "Åsa räknar på äppelpriser ✅");
    }

    #[test]
    fn a_deeper_heading_closes_when_a_shallower_one_starts() {
        let c = chunks("T", "# A\n### C\nx\n## B\ny", 200);
        assert_eq!(c[0].input, "T\nA › C\n\nx");
        assert_eq!(c[1].input, "T\nA › B\n\ny");
    }
}
