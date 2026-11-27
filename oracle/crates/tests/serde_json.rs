use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct UserRecord {
    id: u64,
    name: String,
    active: bool,
    score: Option<i32>,
    tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Event {
    Ping { seq: u32 },
    Message { from: String, body: String },
    Close,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum NumberOrText {
    Num(i64),
    Text(String),
}

#[test]
fn struct_roundtrip_compact_and_pretty() {
    let record = UserRecord {
        id: 9_007_199_254_740_991,
        name: "Mires Compiler".to_owned(),
        active: true,
        score: Some(-42),
        tags: vec!["rust".to_owned(), "javascript".to_owned(), "jir".to_owned()],
    };
    let compact = serde_json::to_string(&record).unwrap();
    let pretty = serde_json::to_string_pretty(&record).unwrap();
    assert!(pretty.contains('\n'));
    assert_eq!(
        serde_json::from_str::<UserRecord>(&compact).unwrap(),
        record
    );
    assert_eq!(serde_json::from_str::<UserRecord>(&pretty).unwrap(), record);
}

#[test]
fn internally_tagged_and_untagged_enums() {
    let events = vec![
        Event::Ping { seq: 1 },
        Event::Message {
            from: "alice".to_owned(),
            body: "hello\n\"world\" \\ \t 🚀".to_owned(),
        },
        Event::Close,
    ];
    let serialized = serde_json::to_string(&events).unwrap();
    let decoded: Vec<Event> = serde_json::from_str(&serialized).unwrap();
    assert_eq!(decoded, events);

    let mixed: Vec<NumberOrText> = serde_json::from_str(r#"[123, "abc", -99, ""]"#).unwrap();
    assert_eq!(
        mixed,
        vec![
            NumberOrText::Num(123),
            NumberOrText::Text("abc".to_owned()),
            NumberOrText::Num(-99),
            NumberOrText::Text(String::new()),
        ]
    );
}

#[test]
fn unicode_escapes_and_surrogate_pairs() {
    let parsed: String = serde_json::from_str(r#""caf\u00e9 \uD83D\uDE80 \u03B1\u03B2""#).unwrap();
    assert_eq!(parsed, "café 🚀 αβ");
    let reencoded = serde_json::to_string(&parsed).unwrap();
    let reparsed: String = serde_json::from_str(&reencoded).unwrap();
    assert_eq!(reparsed, parsed);
}

#[test]
fn value_pointer_and_mutation() {
    let mut doc = json!({
        "compiler": "mires",
        "tracks": [
            {"id": 1, "name": "backend"},
            {"id": 2, "name": "runtime"},
            {"id": 3, "name": "oracle"}
        ],
        "meta": {"stable": false, "count": 3}
    });
    assert_eq!(
        doc.pointer("/compiler"),
        Some(&Value::String("mires".into()))
    );
    assert_eq!(
        doc.pointer("/tracks/2/name"),
        Some(&Value::String("oracle".into()))
    );
    assert_eq!(doc.pointer("/tracks/99"), None);

    if let Some(count) = doc.pointer_mut("/meta/count") {
        *count = json!(42);
    }
    assert_eq!(doc["meta"]["count"], json!(42));
}

#[test]
fn hashmap_and_numeric_boundaries() {
    let mut map: HashMap<String, i64> = HashMap::new();
    map.insert("zero".into(), 0);
    map.insert("max_i64".into(), i64::MAX);
    map.insert("min_i64".into(), i64::MIN);
    map.insert("neg".into(), -123_456_789);
    let text = serde_json::to_string(&map).unwrap();
    let back: HashMap<String, i64> = serde_json::from_str(&text).unwrap();
    assert_eq!(back, map);
}

#[test]
fn rejects_malformed_and_mismatched_json() {
    assert!(serde_json::from_str::<Value>("{unquoted: 1}").is_err());
    assert!(serde_json::from_str::<Value>("[1, 2,]").is_err());
    assert!(serde_json::from_str::<UserRecord>(r#"{"id":"not-a-num"}"#).is_err());
    assert!(serde_json::from_str::<UserRecord>(r#"{"id":1}"#).is_err());
}
