use crc32fast::Hasher;

#[test]
fn ieee_standard_check_vectors() {
    assert_eq!(crc32fast::hash(b""), 0x0000_0000);
    assert_eq!(crc32fast::hash(b"123456789"), 0xcbf4_3926);
    assert_eq!(
        crc32fast::hash(b"The quick brown fox jumps over the lazy dog"),
        0x414f_a339
    );
}

#[test]
fn streaming_chunk_invariance() {
    let payload: Vec<u8> = (0..4096u32).map(|i| ((i * 31 + 17) & 0xff) as u8).collect();
    let oneshot = crc32fast::hash(&payload);

    for chunk_size in [1, 3, 7, 16, 63, 257, 1024] {
        let mut h = Hasher::new();
        for chunk in payload.chunks(chunk_size) {
            h.update(chunk);
        }
        assert_eq!(h.finalize(), oneshot, "chunk_size={chunk_size}");
    }
}

#[test]
fn combine_reset_and_clone() {
    let a = b"first part of the message - ";
    let b = b"second part of the message!";
    let mut combined_input = Vec::new();
    combined_input.extend_from_slice(a);
    combined_input.extend_from_slice(b);

    let mut ha = Hasher::new();
    ha.update(a);
    let mut ha_clone = ha.clone();
    ha_clone.update(b);

    let mut hb = Hasher::new();
    hb.update(b);

    ha.combine(&hb);
    let expected = crc32fast::hash(&combined_input);
    assert_eq!(ha.finalize(), expected);
    assert_eq!(ha_clone.finalize(), expected);

    let mut reusable = Hasher::new();
    reusable.update(b"discard me");
    reusable.reset();
    reusable.update(b"123456789");
    assert_eq!(reusable.finalize(), 0xcbf4_3926);
}
