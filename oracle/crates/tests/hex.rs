use hex::FromHex;

#[test]
fn rfc4648_hex_vectors() {
    let vectors = [
        ("", ""),
        ("f", "66"),
        ("fo", "666f"),
        ("foo", "666f6f"),
        ("foob", "666f6f62"),
        ("fooba", "666f6f6261"),
        ("foobar", "666f6f626172"),
    ];
    for (plain, lower) in vectors {
        assert_eq!(hex::encode(plain), lower);
        assert_eq!(hex::encode_upper(plain), lower.to_ascii_uppercase());
        assert_eq!(hex::decode(lower).unwrap(), plain.as_bytes());
        assert_eq!(
            hex::decode(lower.to_ascii_uppercase()).unwrap(),
            plain.as_bytes()
        );
    }
}

#[test]
fn full_byte_range_and_fixed_arrays() {
    let bytes: Vec<u8> = (0u8..=255).collect();
    let encoded = hex::encode(&bytes);
    assert_eq!(encoded.len(), 512);
    assert_eq!(hex::decode(&encoded).unwrap(), bytes);

    let arr = <[u8; 4]>::from_hex("deAdBeEf").unwrap();
    assert_eq!(arr, [0xde, 0xad, 0xbe, 0xef]);

    let mut dst = [0u8; 8];
    hex::encode_to_slice(arr, &mut dst).unwrap();
    assert_eq!(&dst, b"deadbeef");

    let mut back = [0u8; 4];
    hex::decode_to_slice(&dst, &mut back).unwrap();
    assert_eq!(back, arr);
}

#[test]
fn rejects_invalid_hex() {
    assert_eq!(hex::decode("abc"), Err(hex::FromHexError::OddLength));
    assert_eq!(
        hex::decode("0g"),
        Err(hex::FromHexError::InvalidHexCharacter { c: 'g', index: 1 })
    );
    assert_eq!(
        <[u8; 2]>::from_hex("010203"),
        Err(hex::FromHexError::InvalidStringLength)
    );
}
