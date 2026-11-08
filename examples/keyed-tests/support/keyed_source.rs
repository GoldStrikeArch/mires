//! Cross-crate keyed source used by `18_cross_crate.rs`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

#[derive(Clone, Copy)]
pub struct Row {
    pub id: u32,
    pub label: u32,
}

pub fn rows(changed: bool) -> Vec<Row> {
    if changed {
        Vec::from([
            Row { id: 3, label: 30 },
            Row { id: 1, label: 10 },
            Row { id: 2, label: 20 },
        ])
    } else {
        Vec::from([
            Row { id: 1, label: 1 },
            Row { id: 2, label: 2 },
            Row { id: 3, label: 3 },
        ])
    }
}
