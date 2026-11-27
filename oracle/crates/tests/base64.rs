use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;

#[test]
fn rfc4648_section_10_vectors() {
    let vectors = [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
    ];
    for (plain, encoded) in vectors {
        assert_eq!(STANDARD.encode(plain.as_bytes()), encoded);
        assert_eq!(STANDARD.decode(encoded).unwrap(), plain.as_bytes());
        let stripped = encoded.trim_end_matches('=');
        assert_eq!(STANDARD_NO_PAD.encode(plain.as_bytes()), stripped);
        assert_eq!(STANDARD_NO_PAD.decode(stripped).unwrap(), plain.as_bytes());
    }
}

#[test]
fn url_safe_alphabet_differences() {
    let bytes = [0xfb, 0xff, 0xbf, 0xfe, 0xef, 0xef];
    let std_enc = STANDARD.encode(bytes);
    let url_enc = URL_SAFE.encode(bytes);
    let url_nopad = URL_SAFE_NO_PAD.encode(bytes);
    assert!(std_enc.contains('+') || std_enc.contains('/'));
    assert!(!url_enc.contains('+') && !url_enc.contains('/'));
    assert_eq!(url_enc, url_nopad);
    assert_eq!(URL_SAFE.decode(&url_enc).unwrap(), bytes);
    assert_eq!(URL_SAFE_NO_PAD.decode(&url_nopad).unwrap(), bytes);
}

#[test]
fn all_byte_values_roundtrip() {
    let all_bytes: Vec<u8> = (0u8..=255).collect();
    for len in [0, 1, 2, 3, 4, 17, 64, 255, 256] {
        let slice = &all_bytes[..len];
        let enc = STANDARD.encode(slice);
        assert_eq!(STANDARD.decode(&enc).unwrap(), slice);
        let url = URL_SAFE_NO_PAD.encode(slice);
        assert_eq!(URL_SAFE_NO_PAD.decode(&url).unwrap(), slice);
    }
}

#[test]
fn slice_apis_and_error_handling() {
    let mut enc_buf = [0u8; 16];
    let written = STANDARD.encode_slice(b"mires!", &mut enc_buf).unwrap();
    assert_eq!(&enc_buf[..written], b"bWlyZXMh");

    let mut dec_buf = [0u8; 16];
    let dec_len = STANDARD
        .decode_slice(&enc_buf[..written], &mut dec_buf)
        .unwrap();
    assert_eq!(&dec_buf[..dec_len], b"mires!");

    assert!(STANDARD.decode("not!valid!base64").is_err());
    assert!(STANDARD.decode("Zg=").is_err());
    assert!(STANDARD.decode("A").is_err());
    assert!(STANDARD_NO_PAD.decode("Zg==").is_err());
}
