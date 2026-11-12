include!("fam_common.rs");
#[unsafe(no_mangle)]
pub fn rust_entry() {
    // Box<[[T; N]]> of rows -> Vec -> into_flattened -> boxed -> array, and Rc/Arc from it
    let b: Box<[[u32; 2]]> = (0..4u32).map(|i| [i, i + 7]).collect();
    let f: Vec<u32> = b.into_vec().into_flattened();
    let bf: Box<[u32]> = f.into_boxed_slice();
    out(1, hs(&bf));
    let arr: Box<[u32; 8]> = bf.try_into().ok().unwrap();
    out(2, hs(&arr[..]));
    let (ch, _) = arr.as_chunks::<2>();
    out(3, hr(ch));
    let rc: Rc<[u32; 8]> = Rc::from(arr);
    out(4, hs(&rc[..]));
    let v2: Vec<[u32; 2]> = rc.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
    let rc2: Rc<[[u32; 2]]> = Rc::from(v2.into_boxed_slice());
    out(5, hr(&rc2) ^ hs(rc2.as_flattened()));
    let v8: Vec<[u8; 2]> = (0..4u8).map(|i| [i, i + 7]).collect();
    let bf8: Box<[u8]> = v8.into_flattened().into_boxed_slice();
    let arr8: Box<[u8; 8]> = bf8.try_into().ok().unwrap();
    out(6, hs(&arr8[..]));
    let rc8: Rc<[u8; 8]> = Rc::from(arr8);
    out(7, hs(&rc8[..]));
    let s: Rc<[u8]> = rc8;
    out(8, hs(&s));
}
