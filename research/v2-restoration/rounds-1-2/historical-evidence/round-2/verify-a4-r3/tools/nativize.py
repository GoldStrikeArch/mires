import sys
src = open(sys.argv[1]).read()
src = src.replace('#![no_std]', '').replace('#![no_main]', '')
shim = ('mod prelude { #![allow(dead_code)] '
        'pub fn print_i32(x:i32){println!("{}",x)} pub fn print_u32(x:u32){println!("{}",x)} '
        'pub fn print_usize(x:usize){println!("{}",x)} pub fn print_i64(x:i64){println!("{}",x)} '
        'pub fn print_u64(x:u64){println!("{}",x as i64)} pub fn print_f64(x:f64){println!("{}",x)} '
        'pub fn print_bool(x:bool){println!("{}",x)} pub fn print_str(s:&str){println!("{}",s)} }')
marker = '#[path = "prelude.rs"]\nmod prelude;'
assert marker in src, "probe must use the shared prelude"
src = src.replace(marker, shim)
src = src.replace('#[unsafe(no_mangle)]\npub fn rust_entry()', 'pub fn rust_entry()')
src += '\nfn main(){ rust_entry(); }\n'
open(sys.argv[2], 'w').write(src)
