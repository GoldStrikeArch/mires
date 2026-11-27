use std::io::{Read, Write};

use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use flate2::{Compression, GzBuilder};

#[test]
fn gzip_roundtrip_and_header_metadata() {
    let payload = b"gzip differential test payload with repetition. ".repeat(40);
    let mut enc = GzBuilder::new()
        .filename("oracle.txt")
        .comment("mires differential test")
        .write(Vec::new(), Compression::default());
    enc.write_all(&payload).unwrap();
    let gz = enc.finish().unwrap();

    let mut dec = GzDecoder::new(&gz[..]);
    let header = dec.header().unwrap();
    assert_eq!(header.filename(), Some(&b"oracle.txt"[..]));
    assert_eq!(header.comment(), Some(&b"mires differential test"[..]));

    let mut out = Vec::new();
    dec.read_to_end(&mut out).unwrap();
    assert_eq!(out, payload);
}

#[test]
fn zlib_and_raw_deflate_compression_levels() {
    let data: Vec<u8> = (0..1500u32)
        .map(|i| {
            if i % 5 == 0 {
                b'X'
            } else {
                b'a' + (i % 26) as u8
            }
        })
        .collect();

    for level in [
        Compression::none(),
        Compression::fast(),
        Compression::best(),
    ] {
        let mut zenc = ZlibEncoder::new(Vec::new(), level);
        zenc.write_all(&data).unwrap();
        let zbytes = zenc.finish().unwrap();
        let mut zout = Vec::new();
        ZlibDecoder::new(&zbytes[..])
            .read_to_end(&mut zout)
            .unwrap();
        assert_eq!(zout, data);

        let mut denc = DeflateEncoder::new(Vec::new(), level);
        denc.write_all(&data).unwrap();
        let dbytes = denc.finish().unwrap();
        let mut dout = Vec::new();
        DeflateDecoder::new(&dbytes[..])
            .read_to_end(&mut dout)
            .unwrap();
        assert_eq!(dout, data);
    }
}

#[test]
fn streaming_small_chunk_reads() {
    let input = b"0123456789abcdef".repeat(64);
    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    for chunk in input.chunks(13) {
        enc.write_all(chunk).unwrap();
    }
    let compressed = enc.finish().unwrap();

    let mut dec = GzDecoder::new(&compressed[..]);
    let mut out = Vec::new();
    let mut buf = [0u8; 17];
    loop {
        let n = dec.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    assert_eq!(out, input);
}
