// Pre-existing: `&fn_item` promoted constant: "constants of type `fn() {f}` cannot be read out of an allocation".
fn plain() -> u32 {
    77
}
fn call_generic_ref<F: Fn() -> u32>(f: &F) -> u32 {
    f()
}
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(call_generic_ref(&plain));
}
