#![no_std]
#![deny(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::float_cmp,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::struct_excessive_bools
)]

extern crate alloc;

mod api;
mod blocker_data;
mod browser;
mod browser_runtime;
mod canonical;
mod confidence;
mod data;
mod entropy;
mod hashing;
mod host_async;
mod host_bindings;
mod host_dom;
mod host_graphics;
mod manifest;
mod scheduler;
mod sources_async;
mod sources_dom;
mod sources_extra;
mod sources_graphics;
mod sources_runtime;
mod sources_sync;

pub use api::*;
pub use browser::*;
pub use browser_runtime::*;
pub use canonical::*;
pub use confidence::*;
pub use data::*;
pub use entropy::*;
pub use hashing::*;
pub use host_async::*;
pub use host_bindings::*;
pub use host_dom::*;
pub use host_graphics::*;
pub use manifest::*;
pub use scheduler::*;
pub use sources_async::*;
pub use sources_dom::*;
pub use sources_extra::*;
pub use sources_graphics::*;
pub use sources_runtime::{
    cancel_source_timeout,
    get_unstable_hardware_concurrency as runtime_unstable_hardware_concurrency,
    get_unstable_screen_resolution as runtime_unstable_screen_resolution, source_architecture,
    source_audio_base_latency, source_clock, source_color_depth, source_color_gamut,
    source_contrast, source_cookies_enabled, source_cpu_class, source_date_time_locale,
    source_device_memory, source_forced_colors, source_hardware_concurrency, source_hdr,
    source_indexed_db, source_inverted_colors, source_languages, source_local_storage,
    source_monochrome, source_open_database, source_os_cpu, source_pdf_viewer_enabled,
    source_platform, source_plugins, source_private_click_measurement, source_reduced_motion,
    source_reduced_transparency, source_screen_resolution, source_session_storage, source_timezone,
    source_touch_support, source_vendor, source_vendor_flavors,
};
pub use sources_sync::*;
