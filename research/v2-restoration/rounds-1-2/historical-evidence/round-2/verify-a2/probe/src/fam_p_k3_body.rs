include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    containers_copy::<P, 3>(0);
}
