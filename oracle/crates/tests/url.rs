use url::{Host, Url};

#[test]
fn url_parse_components_and_query_pairs() {
    let u = Url::parse("https://alice:secret@example.com:8443/a/b/c?k=1&msg=hello%20world#top")
        .unwrap();
    assert_eq!(u.scheme(), "https");
    assert_eq!(u.username(), "alice");
    assert_eq!(u.password(), Some("secret"));
    assert_eq!(u.host(), Some(Host::Domain("example.com")));
    assert_eq!(u.port(), Some(8443));
    assert_eq!(u.path(), "/a/b/c");
    assert_eq!(u.fragment(), Some("top"));

    let pairs: Vec<(String, String)> = u.query_pairs().into_owned().collect();
    assert_eq!(
        pairs,
        vec![
            ("k".to_string(), "1".to_string()),
            ("msg".to_string(), "hello world".to_string()),
        ]
    );
}

#[test]
fn url_join_ipv6_and_mutation() {
    let base = Url::parse("https://example.com/dir/sub/page.html").unwrap();
    assert_eq!(
        base.join("../other.html?q=2").unwrap().as_str(),
        "https://example.com/dir/other.html?q=2"
    );

    let mut v6 = Url::parse("http://[2001:db8::1]:8080/api").unwrap();
    assert_eq!(v6.host_str(), Some("[2001:db8::1]"));
    assert_eq!(v6.port(), Some(8080));
    v6.path_segments_mut().unwrap().push("v2").push("items");
    assert_eq!(v6.as_str(), "http://[2001:db8::1]:8080/api/v2/items");
}
