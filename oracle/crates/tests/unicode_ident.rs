use unicode_ident::{is_xid_continue, is_xid_start};

fn is_valid_rust_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || is_xid_start(first)) && chars.all(is_xid_continue)
}

#[test]
fn ascii_and_unicode_scripts() {
    for ch in ['a', 'Z', 'α', 'Ω', 'Ж', 'あ', '漢', 'ع', 'अ'] {
        assert!(is_xid_start(ch), "{ch:?} should be XID_Start");
        assert!(is_xid_continue(ch), "{ch:?} should be XID_Continue");
    }

    for ch in ['0', '9', '_', '\u{0301}', '\u{0660}'] {
        assert!(!is_xid_start(ch), "{ch:?} should not be XID_Start");
        assert!(is_xid_continue(ch), "{ch:?} should be XID_Continue");
    }

    for ch in [' ', '-', '+', '!', '\n', '🚀', '©', '\u{0000}', '\u{200B}'] {
        assert!(!is_xid_start(ch), "{ch:?} should not be XID_Start");
        assert!(!is_xid_continue(ch), "{ch:?} should not be XID_Continue");
    }
}

#[test]
fn rust_identifier_validation() {
    assert!(is_valid_rust_ident("_foo_123"));
    assert!(is_valid_rust_ident("αβγ_1"));
    assert!(is_valid_rust_ident("変数名"));
    assert!(!is_valid_rust_ident(""));
    assert!(!is_valid_rust_ident("1abc"));
    assert!(!is_valid_rust_ident("foo-bar"));
    assert!(!is_valid_rust_ident("foo🚀"));
}

#[test]
fn uax31_invariant_start_implies_continue() {
    // Check a wide sample across BMP and supplementary planes: XID_Start => XID_Continue.
    for cp in (0u32..0x0600).chain(0x3000..0x3100).chain(0x1F600..0x1F650) {
        if let Some(ch) = char::from_u32(cp) {
            if is_xid_start(ch) {
                assert!(
                    is_xid_continue(ch),
                    "U+{cp:04X} violates XID_Start => XID_Continue"
                );
            }
        }
    }
}
