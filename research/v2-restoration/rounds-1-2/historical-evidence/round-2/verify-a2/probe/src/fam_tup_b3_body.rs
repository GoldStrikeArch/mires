include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    c_b::<(u8, u32), 3>(0);
}
