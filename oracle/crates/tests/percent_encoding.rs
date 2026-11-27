use percent_encoding::{
    percent_decode, percent_decode_str, percent_encode, utf8_percent_encode, AsciiSet, CONTROLS,
    NON_ALPHANUMERIC,
};

const QUERY_SET: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'#').add(b'<').add(b'>');

#[test]
fn ascii_and_custom_sets() {
    let encoded = utf8_percent_encode("hello world/a?b=1&c=2", NON_ALPHANUMERIC).to_string();
    assert_eq!(encoded, "hello%20world%2Fa%3Fb%3D1%26c%3D2");
    assert_eq!(
        percent_decode_str(&encoded).decode_utf8().unwrap(),
        "hello world/a?b=1&c=2"
    );

    let q = utf8_percent_encode("q=a <b#c> \"ok\"", QUERY_SET).to_string();
    assert_eq!(q, "q=a%20%3Cb%23c%3E%20%22ok%22");
    assert_eq!(
        percent_decode_str(&q).decode_utf8().unwrap(),
        "q=a <b#c> \"ok\""
    );
}

#[test]
fn multibyte_utf8_roundtrip() {
    let s = "café 日本語 🚀 αβγ";
    let enc = utf8_percent_encode(s, CONTROLS).to_string();
    assert!(enc.is_ascii());
    assert_eq!(percent_decode_str(&enc).decode_utf8().unwrap(), s);
}

#[test]
fn raw_bytes_and_malformed_percent_sequences() {
    let raw = [0x00u8, 0x20, 0x7f, 0x80, 0xff];
    let enc = percent_encode(&raw, CONTROLS).to_string();
    assert_eq!(enc, "%00 %7F%80%FF");
    let dec: Vec<u8> = percent_decode(enc.as_bytes()).collect();
    assert_eq!(dec, raw);

    // Incomplete or non-hex percent escapes pass through literally.
    assert_eq!(percent_decode_str("100%").decode_utf8().unwrap(), "100%");
    assert_eq!(percent_decode_str("%2").decode_utf8().unwrap(), "%2");
    assert_eq!(percent_decode_str("%ZZ%41").decode_utf8().unwrap(), "%ZZA");
    assert!(percent_decode_str("%FF").decode_utf8().is_err());
    assert_eq!(
        percent_decode_str("a%FFb").decode_utf8_lossy(),
        "a\u{FFFD}b"
    );
}
