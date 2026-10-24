//! Search text: Markdown reduced to the words a person would search for, and
//! user input turned into a safe FTS5 query.

use pulldown_cmark::{Event, Options, Parser, TagEnd};

/// Plain text of a Markdown note: markup and link URLs dropped, image alt
/// text and code kept.
pub fn markdown_to_text(md: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut out = String::with_capacity(md.len());
    for event in Parser::new_ext(md, options) {
        match event {
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::Item
                | TagEnd::CodeBlock
                | TagEnd::TableCell
                | TagEnd::TableRow,
            ) => out.push('\n'),
            _ => {}
        }
    }
    out
}

/// Builds an FTS5 query matching notes that contain every word of `input`,
/// the last word as a prefix (search as you type). Returns `None` when the
/// input has no words. FTS5 operators in the input are treated as text.
pub fn fts_query(input: &str) -> Option<String> {
    let words: Vec<&str> = input.split_whitespace().collect();
    let last = words.len().checked_sub(1)?;
    Some(
        words
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let quoted = format!("\"{}\"", w.replace('"', "\"\""));
                if i == last { quoted + "*" } else { quoted }
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_markup() {
        let md = "# Trip\n\n- [x] **passport**\n- [ ] [charger](https://shop.example/x)\n\n![A cat](cat.png)\n\n| a | b |\n|---|---|\n| `code` | ~~old~~ |\n\nZażółć &#32; gęślą";
        let text = markdown_to_text(md);
        for word in [
            "Trip",
            "passport",
            "charger",
            "A cat",
            "code",
            "old",
            "Zażółć",
            "gęślą",
        ] {
            assert!(text.contains(word), "{word} missing from {text:?}");
        }
        for markup in ["#", "**", "https", "shop", "|", "`", "~~", "[x]"] {
            assert!(!text.contains(markup), "{markup} left in {text:?}");
        }
    }

    #[test]
    fn query_quotes_words_and_prefixes_the_last() {
        assert_eq!(fts_query("  "), None);
        assert_eq!(fts_query("gr"), Some("\"gr\"*".into()));
        assert_eq!(fts_query("trip  pass"), Some("\"trip\" \"pass\"*".into()));
        // Operators and quotes are inert.
        assert_eq!(
            fts_query("a OR \"b NEAR(c"),
            Some("\"a\" \"OR\" \"\"\"b\" \"NEAR(c\"*".into())
        );
    }
}
