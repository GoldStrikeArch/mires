//! An interoperable stored-block gzip workload inspired by fflate.

use alloc::vec::Vec;

/// Produces a valid gzip member containing DEFLATE stored blocks.
#[must_use]
pub fn gzip_store(input: &[u8]) -> Vec<u8> {
    let blocks = input.len().div_ceil(u16::MAX as usize).max(1);
    let mut output = Vec::with_capacity(10 + input.len() + blocks * 5 + 8);
    output.extend_from_slice(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255]);
    if input.is_empty() {
        output.extend_from_slice(&[1, 0, 0, 255, 255]);
    } else {
        for (index, block) in input.chunks(u16::MAX as usize).enumerate() {
            output.push(u8::from(index + 1 == blocks));
            let length = block.len() as u16;
            output.extend_from_slice(&length.to_le_bytes());
            output.extend_from_slice(&(!length).to_le_bytes());
            output.extend_from_slice(block);
        }
    }
    output.extend_from_slice(&crc32(input).to_le_bytes());
    output.extend_from_slice(&(input.len() as u32).to_le_bytes());
    output
}

/// Decodes the stored-block gzip members produced by [`gzip_store`].
pub fn gunzip_store(input: &[u8]) -> Option<Vec<u8>> {
    if input.len() < 23 || input[..3] != [0x1f, 0x8b, 8] {
        return None;
    }
    let mut cursor = 10;
    let end = input.len() - 8;
    let mut output = Vec::new();
    loop {
        let header = *input.get(cursor)?;
        cursor += 1;
        if header & 0b110 != 0 {
            return None;
        }
        let length = u16::from_le_bytes([*input.get(cursor)?, *input.get(cursor + 1)?]);
        let inverse = u16::from_le_bytes([*input.get(cursor + 2)?, *input.get(cursor + 3)?]);
        cursor += 4;
        if length != !inverse || cursor + usize::from(length) > end {
            return None;
        }
        output.extend_from_slice(&input[cursor..cursor + usize::from(length)]);
        cursor += usize::from(length);
        if header & 1 == 1 {
            break;
        }
    }
    let expected_crc = u32::from_le_bytes(input[end..end + 4].try_into().ok()?);
    let expected_size = u32::from_le_bytes(input[end + 4..].try_into().ok()?);
    (cursor == end && crc32(&output) == expected_crc && output.len() as u32 == expected_size)
        .then_some(output)
}

fn crc32(input: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in input {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_stored_gzip() {
        let source = b"browser compiler workload";
        assert_eq!(gunzip_store(&gzip_store(source)).as_deref(), Some(source.as_slice()));
    }
}
