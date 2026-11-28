use regex_lite::{Regex, RegexBuilder};

#[test]
fn regex_lite_captures_and_replace_all() {
    let re = Regex::new(r"(\d{4})-(\d{2})-(\d{2})").unwrap();
    let text = "dates: 2026-11-28 and 1999-01-02";

    let caps: Vec<(String, String, String)> = re
        .captures_iter(text)
        .map(|c| (c[1].to_string(), c[2].to_string(), c[3].to_string()))
        .collect();
    assert_eq!(
        caps,
        vec![
            ("2026".to_string(), "11".to_string(), "28".to_string()),
            ("1999".to_string(), "01".to_string(), "02".to_string()),
        ]
    );
    assert_eq!(
        re.replace_all(text, "$3/$2/$1"),
        "dates: 28/11/2026 and 02/01/1999"
    );
}

#[test]
fn regex_lite_builder_flags_and_alternation() {
    let re = RegexBuilder::new(r"^(?:foo|bar)_(\w+)$")
        .case_insensitive(true)
        .multi_line(true)
        .build()
        .unwrap();
    let input = "nope\nFOO_alpha\nbar_42\nbaz_0";
    let matched: Vec<&str> = re.find_iter(input).map(|m| m.as_str()).collect();
    assert_eq!(matched, vec!["FOO_alpha", "bar_42"]);
}
