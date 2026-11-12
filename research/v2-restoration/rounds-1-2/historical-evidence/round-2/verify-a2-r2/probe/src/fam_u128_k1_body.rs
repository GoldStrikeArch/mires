include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    containers_copy::<u128, 1>(0);
}
