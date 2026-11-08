//! The section codec: postcard for structure, deflate for bytes.
//!
//! Every typed payload of an object file (interns, summaries, bodies, origins) is postcard
//! serialized; the sections that carry many bytes are then deflated. Bodies and origins are
//! deflated **one blob at a time** so the link step can inflate exactly the reachable ones and
//! never touch the rest, and interns as one blob because a reader always needs the whole table. The
//! summary index is left uncompressed: it is small, read eagerly by every link, and its cost is in
//! decoding rather than in bytes.
//!
//! [`Codec`] names the byte codec in the object header so a reader never guesses; the only value
//! today is [`Codec::Deflate`] (raw deflate, RFC 1951, no zlib wrapper), through `miniz_oxide`.

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::origin::OriginTable;
use super::semantic::Body;

/// The byte codec an object's compressed sections were written with.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Codec {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// Raw deflate (RFC 1951) at [`DEFLATE_LEVEL`].
    Deflate,
}

/// The `miniz_oxide` compression level: 6 is zlib's default and, on postcard bytes, within a
/// percent of level 9 at a third of the time.
pub const DEFLATE_LEVEL: u8 = 6;

/// A codec failure: bytes that do not inflate, or inflated bytes that do not decode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodecError(pub String);

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CodecError {}

impl Codec {
    /// Compresses `bytes`.
    pub fn compress(self, bytes: &[u8]) -> Vec<u8> {
        match self {
            Codec::Deflate => miniz_oxide::deflate::compress_to_vec(bytes, DEFLATE_LEVEL),
        }
    }

    /// Decompresses `bytes`, which must inflate to exactly `expected_len` bytes.
    ///
    /// The expected length is stored beside every compressed blob, so a reader can size its buffer
    /// and reject a blob that inflates to something else — the cheapest possible check that a blob
    /// reference points at the blob it was written for.
    pub fn decompress(self, bytes: &[u8], expected_len: usize) -> Result<Vec<u8>, CodecError> {
        let out = match self {
            Codec::Deflate => {
                miniz_oxide::inflate::decompress_to_vec_with_limit(bytes, expected_len).map_err(
                    |err| CodecError(format!("deflate stream does not inflate: {err:?}")),
                )?
            }
        };
        if out.len() != expected_len {
            return Err(CodecError(format!(
                "deflate stream inflates to {} bytes, {expected_len} expected",
                out.len()
            )));
        }
        Ok(out)
    }
}

/// Serializes `value` with postcard.
pub fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    postcard::to_allocvec(value).expect("every object payload type serializes with postcard")
}

/// Deserializes a postcard `T` out of `bytes`, which must be consumed exactly.
pub fn decode<T: DeserializeOwned>(bytes: &[u8], what: &str) -> Result<T, CodecError> {
    let (value, rest) = postcard::take_from_bytes::<T>(bytes)
        .map_err(|err| CodecError(format!("corrupt {what} (postcard: {err})")))?;
    if !rest.is_empty() {
        return Err(CodecError(format!(
            "corrupt {what}: {} trailing bytes after the postcard payload",
            rest.len()
        )));
    }
    Ok(value)
}

/// A body's postcard bytes: what the object stores (deflated) and what its content hash is of.
pub fn encode_body(body: &Body) -> Vec<u8> {
    encode(body)
}

pub fn decode_body(bytes: &[u8]) -> Result<Body, CodecError> {
    decode(bytes, "JIR body")
}

pub fn encode_origins(origins: &OriginTable) -> Vec<u8> {
    encode(origins)
}

pub fn decode_origins(bytes: &[u8]) -> Result<OriginTable, CodecError> {
    decode(bytes, "JIR origin table")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deflate_round_trips_and_checks_the_length() {
        let data: Vec<u8> = (0..5000u32).map(|i| (i % 13) as u8).collect();
        let packed = Codec::Deflate.compress(&data);
        assert!(packed.len() < data.len());
        assert_eq!(
            Codec::Deflate.decompress(&packed, data.len()).unwrap(),
            data
        );
        assert!(Codec::Deflate.decompress(&packed, data.len() - 1).is_err());
        assert!(Codec::Deflate.decompress(&packed, data.len() + 1).is_err());
        assert!(Codec::Deflate
            .decompress(&packed[..packed.len() / 2], data.len())
            .is_err());
    }

    #[test]
    fn empty_input_round_trips() {
        let packed = Codec::Deflate.compress(b"");
        assert_eq!(
            Codec::Deflate.decompress(&packed, 0).unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn decode_rejects_trailing_bytes() {
        let mut bytes = encode(&(1u32, "x".to_owned()));
        assert_eq!(
            decode::<(u32, String)>(&bytes, "pair").unwrap(),
            (1, "x".to_owned())
        );
        bytes.push(0);
        let err = decode::<(u32, String)>(&bytes, "pair").unwrap_err();
        assert!(err.0.contains("trailing"), "{err}");
    }
}
