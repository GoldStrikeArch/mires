//! Empty vectors and UTF-8 word reads preserve slice storage and byte offsets.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

extern crate alloc;
use alloc::vec::Vec;

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    let mut checks = 0;
    for length in 0..=80_u32 {
        let bytes = (0..length).map(|index| index as u8).collect::<Vec<_>>();
        assert_eq!(
            bytes.iter().map(|value| u32::from(*value)).sum::<u32>(),
            length * length.saturating_sub(1) / 2,
        );
        let integers = (0..length).collect::<Vec<_>>();
        assert_eq!(integers.iter().copied().sum::<u32>(), length * length.saturating_sub(1) / 2);
        assert_eq!((0..length).map(|_| ()).collect::<Vec<_>>().iter().count(), length as usize);
        let mut visits = 0;
        let units = (0..length).map(|_| visits += 1).collect::<Vec<_>>();
        assert_eq!(units.len(), length as usize);
        assert_eq!(visits, length);
        checks += 5;

        for offset in 0..8 {
            let mut bytes = (0..length as usize + offset).map(|_| b'a').collect::<Vec<_>>();
            assert!(core::str::from_utf8(&bytes[offset..]).is_ok());
            checks += 1;
            if length != 0 {
                for index in [0, length as usize / 2, length as usize - 1] {
                    bytes[offset + index] = 0x80;
                    let error = core::str::from_utf8(&bytes[offset..]).unwrap_err();
                    assert_eq!(error.valid_up_to(), index);
                    assert_eq!(error.error_len(), Some(1));
                    bytes[offset + index] = b'a';
                    checks += 2;
                }
            }
            if length >= 12 {
                let pointer = bytes.as_ptr();
                let aligned = pointer.wrapping_add(pointer.align_offset(4) + 4);
                assert_eq!(aligned.addr(), aligned.cast::<u32>().addr());
                checks += 1;
            }
            // Grow the same vector after a word view has been observed.
            bytes.extend_from_slice("\u{e9}\u{20ac}\u{1f600}".as_bytes());
            assert!(core::str::from_utf8(&bytes[offset..]).is_ok());
            bytes.pop();
            let error = core::str::from_utf8(&bytes[offset..]).unwrap_err();
            assert_eq!(error.valid_up_to(), length as usize + 5);
            assert_eq!(error.error_len(), None);
            checks += 3;
        }
    }
    print_i32(checks);
}
