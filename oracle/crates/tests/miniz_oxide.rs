use miniz_oxide::deflate::{compress_to_vec, compress_to_vec_zlib};
use miniz_oxide::inflate::{decompress_to_vec, decompress_to_vec_zlib};

#[test]
fn raw_and_zlib_roundtrips_across_levels() {
    let repetitive = b"mires rust-to-js differential oracle! ".repeat(80);
    let pseudo_random: Vec<u8> = (0u32..2048)
        .map(|i| ((i.wrapping_mul(1_103_515_245).wrapping_add(12_345)) >> 16) as u8)
        .collect();

    for payload in [&b""[..], b"a", &repetitive[..], &pseudo_random[..]] {
        for level in [0u8, 1, 6, 9] {
            let raw = compress_to_vec(payload, level);
            assert_eq!(decompress_to_vec(&raw).unwrap(), payload);

            let zlib = compress_to_vec_zlib(payload, level);
            assert_eq!(decompress_to_vec_zlib(&zlib).unwrap(), payload);
        }
    }
}

#[test]
fn repetitive_input_actually_compresses() {
    let data = b"ABCD1234".repeat(256);
    let zlib = compress_to_vec_zlib(&data, 6);
    assert!(zlib.len() < data.len() / 10);
    assert_eq!(decompress_to_vec_zlib(&zlib).unwrap(), data);
}

#[test]
fn known_zlib_vector_and_corruption_detection() {
    // Standard zlib-compressed "hello" (CMF=0x78, FLG=0x9c).
    let hello_zlib: &[u8] = &[
        0x78, 0x9c, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00, 0x06, 0x2c, 0x02, 0x15,
    ];
    assert_eq!(decompress_to_vec_zlib(hello_zlib).unwrap(), b"hello");

    let mut corrupted = hello_zlib.to_vec();
    *corrupted.last_mut().unwrap() ^= 0xff;
    assert!(decompress_to_vec_zlib(&corrupted).is_err());
    assert!(decompress_to_vec_zlib(&hello_zlib[..4]).is_err());
}
