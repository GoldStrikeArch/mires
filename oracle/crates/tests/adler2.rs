use std::io::Cursor;

use adler2::{adler32, adler32_slice, Adler32};

#[test]
fn rfc1950_known_vectors() {
    assert_eq!(adler32_slice(b""), 1);
    assert_eq!(adler32_slice(b"Wikipedia"), 0x11e6_0398);
    assert_eq!(adler32_slice(b"123456789"), 0x091e_01de);
}

#[test]
fn incremental_and_mod_wraparound() {
    // Length > 5552 forces multiple modulo-65521 reductions inside Adler-32.
    let data: Vec<u8> = (0..12_000u32)
        .map(|i| ((i * 73 + 19) % 256) as u8)
        .collect();
    let expected = adler32_slice(&data);

    for step in [1, 13, 255, 5552, 6000] {
        let mut hasher = Adler32::new();
        for chunk in data.chunks(step) {
            hasher.write_slice(chunk);
        }
        assert_eq!(hasher.checksum(), expected, "step={step}");
    }
}

#[test]
fn reader_streaming_and_from_checksum() {
    let text = b"the quick brown fox jumps over the lazy dog 0123456789";
    let expected = adler32_slice(text);
    assert_eq!(adler32(Cursor::new(&text[..])).unwrap(), expected);

    let prefix_sum = adler32_slice(&text[..20]);
    let mut resumed = Adler32::from_checksum(prefix_sum);
    resumed.write_slice(&text[20..]);
    assert_eq!(resumed.checksum(), expected);
}
