// Pre-existing (not track_caller related): a static `Option<fn()>` initialised to `Some(f)` reads as `None`.
fn plain() -> u32 {
    77
}
static OPT: Option<fn() -> u32> = Some(plain);
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(OPT.map(|f| f()).unwrap_or(0));
    print_bool(OPT.is_some());
}
