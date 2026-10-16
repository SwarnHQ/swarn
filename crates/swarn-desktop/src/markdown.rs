//! Deliberately shallow markdown (§5.2): paragraphs, headings, lists, code fences, inline code,
//! bold and links. It reparses the whole message on each change, which is fine for one streaming
//! block; an unclosed fence just runs to the end, so half-streamed text never breaks the layout.

use std::ops::Range;

#[derive(Debug, PartialEq)]
pub enum Md {
    Para(Inline),
    Heading(Inline),
    /// `None` for bullets, the number for ordered items.
    Item(Option<u32>, Inline),
    Code(String),
}

/// Text with the ranges that need styling. Markers (backticks, `**`, link syntax) are removed.
#[derive(Debug, Default, PartialEq)]
pub struct Inline {
    pub text: String,
    pub code: Vec<Range<usize>>,
    pub bold: Vec<Range<usize>>,
    pub links: Vec<Range<usize>>,
}

pub fn parse(source: &str) -> Vec<Md> {
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut code: Option<Vec<&str>> = None;

    let flush = |para: &mut Vec<&str>, out: &mut Vec<Md>| {
        if !para.is_empty() {
            out.push(Md::Para(inline(&para.join(" "))));
            para.clear();
        }
    };

    for line in source.lines() {
        let trimmed = line.trim_start();
        if let Some(lines) = &mut code {
            if trimmed.starts_with("```") {
                out.push(Md::Code(lines.join("\n")));
                code = None;
            } else {
                lines.push(line);
            }
            continue;
        }
        if trimmed.starts_with("```") {
            flush(&mut para, &mut out);
            code = Some(Vec::new());
        } else if trimmed.is_empty() {
            flush(&mut para, &mut out);
        } else if let Some(rest) = trimmed.strip_prefix('#') {
            flush(&mut para, &mut out);
            out.push(Md::Heading(inline(rest.trim_start_matches('#').trim())));
        } else if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            flush(&mut para, &mut out);
            out.push(Md::Item(None, inline(rest)));
        } else if let Some((n, rest)) = ordered(trimmed) {
            flush(&mut para, &mut out);
            out.push(Md::Item(Some(n), inline(rest)));
        } else {
            para.push(trimmed);
        }
    }
    flush(&mut para, &mut out);
    if let Some(lines) = code {
        out.push(Md::Code(lines.join("\n")));
    }
    out
}

fn ordered(line: &str) -> Option<(u32, &str)> {
    let (digits, rest) = line.split_once(". ")?;
    Some((digits.parse().ok()?, rest))
}

fn inline(source: &str) -> Inline {
    let mut out = Inline::default();
    let mut rest = source;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
        {
            push_styled(&mut out, &after[..end], |o| &mut o.code);
            rest = &after[end + 1..];
        } else if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            push_styled(&mut out, &after[..end], |o| &mut o.bold);
            rest = &after[end + 2..];
        } else if let Some(after) = rest.strip_prefix('[')
            && let Some(close) = after.find("](")
            && let Some(end) = after[close..].find(')')
        {
            push_styled(&mut out, &after[..close], |o| &mut o.links);
            rest = &after[close + end + 1..];
        } else {
            let ch = rest.chars().next().expect("non-empty");
            out.text.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

fn push_styled(out: &mut Inline, text: &str, ranges: fn(&mut Inline) -> &mut Vec<Range<usize>>) {
    let start = out.text.len();
    out.text.push_str(text);
    let end = out.text.len();
    ranges(out).push(start..end);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_and_inline_styles() {
        let md = parse(
            "# Plan\n\nEdit `main.rs` and **test** it,\nsee [docs](http://x).\n\n- one\n2. two\n\n```rust\nfn a() {}\n```",
        );
        assert_eq!(md.len(), 5);
        assert!(matches!(&md[0], Md::Heading(i) if i.text == "Plan"));
        let Md::Para(p) = &md[1] else {
            panic!("{md:?}")
        };
        assert_eq!(p.text, "Edit main.rs and test it, see docs.");
        assert_eq!(&p.text[p.code[0].clone()], "main.rs");
        assert_eq!(&p.text[p.bold[0].clone()], "test");
        assert_eq!(&p.text[p.links[0].clone()], "docs");
        assert!(matches!(&md[2], Md::Item(None, i) if i.text == "one"));
        assert!(matches!(&md[3], Md::Item(Some(2), i) if i.text == "two"));
        assert_eq!(md[4], Md::Code("fn a() {}".into()));
    }

    #[test]
    fn half_streamed_text_still_parses() {
        let md = parse("Look:\n```\nlet x = `1");
        assert_eq!(
            md,
            [Md::Para(inline("Look:")), Md::Code("let x = `1".into())]
        );
        // An unmatched backtick is just a character.
        assert_eq!(inline("a `b").text, "a `b");
    }
}
