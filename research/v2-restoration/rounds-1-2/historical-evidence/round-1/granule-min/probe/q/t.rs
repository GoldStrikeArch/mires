fn main() { let mut s = String::with_capacity(8); s.push('a'); let p = s.as_mut_ptr(); let q = s.as_ptr(); println!("{}", p as usize == q as usize); }
