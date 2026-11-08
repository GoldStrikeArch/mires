//! Medium browser workloads for the JavaScript backend.

#![no_std]
#![deny(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::float_cmp,
    clippy::missing_errors_doc,
    clippy::must_use_candidate
)]

extern crate alloc;

pub mod explorer;
pub mod fflate;
pub mod host;
pub mod idb;
pub mod idb_lines;
pub mod ky;
pub mod marked;
pub mod minisearch;
pub mod uplot;
pub mod workbox;
pub mod workbox_direct;
