include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let v: Vec<u32> = bb(vec![0x01020304, 0x05060708, 0x090a0b0c, 0x0d0e0f10]);
    let mut f: Vec<[u8; 4]> = v.into_iter().map(|x| x.to_le_bytes()).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([9, 9, 9, 9]); out(2, hr(&f));
}
