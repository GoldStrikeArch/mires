use pulldown_cmark::{html, Event, HeadingLevel, Options, Parser, Tag};

fn render(md: &str, opts: Options) -> String {
    let parser = Parser::new_ext(md, opts);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

#[test]
fn commonmark_headings_emphasis_and_code_blocks() {
    let md = "# Title\n\nParagraph with *em*, **strong**, and `code <1>`.\n\n```rust\nfn main() {}\n```\n";
    let html_out = render(md, Options::empty());
    assert_eq!(
        html_out,
        "<h1>Title</h1>\n\
         <p>Paragraph with <em>em</em>, <strong>strong</strong>, and <code>code &lt;1&gt;</code>.</p>\n\
         <pre><code class=\"language-rust\">fn main() {}\n</code></pre>\n"
    );
}

#[test]
fn nested_lists_blockquotes_and_links() {
    let md = "> Quote with [link](https://example.com \"Title\")\n\n- one\n  - nested\n- two\n";
    let html_out = render(md, Options::empty());
    assert!(html_out.contains("<blockquote>"));
    assert!(html_out.contains("<a href=\"https://example.com\" title=\"Title\">link</a>"));
    assert!(html_out
        .contains("<ul>\n<li>one\n<ul>\n<li>nested</li>\n</ul>\n</li>\n<li>two</li>\n</ul>"));
}

#[test]
fn gfm_tables_strikethrough_and_tasklists() {
    let md = "| Key | Val |\n| --- | ---: |\n| ~~old~~ | 42 |\n\n- [x] done\n- [ ] todo\n";
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let html_out = render(md, opts);
    assert!(html_out.contains("<table>"));
    assert!(html_out.contains("<del>old</del>"));
    assert!(html_out.contains("style=\"text-align: right\""));
    assert!(html_out.contains("type=\"checkbox\""));
    assert!(html_out.contains("checked=\"\""));
}

#[test]
fn event_stream_heading_extraction() {
    let md = "# One\n\nText\n\n## Two\n\n### Three\n";
    let mut headings = Vec::new();
    let mut current: Option<HeadingLevel> = None;
    for ev in Parser::new(md) {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => current = Some(level),
            Event::Text(t) if current.is_some() => {
                headings.push((current.unwrap(), t.into_string()));
            }
            Event::End(_) => current = None,
            _ => {}
        }
    }
    assert_eq!(
        headings,
        vec![
            (HeadingLevel::H1, "One".into()),
            (HeadingLevel::H2, "Two".into()),
            (HeadingLevel::H3, "Three".into()),
        ]
    );
}
