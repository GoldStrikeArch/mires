#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
use alloc::vec::Vec;
use mires_js::{js_bindings, js_export, throw_value, JsValue};
mod codec;
use codec::{compress, decompress, Format};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("invalid compressed stream"))
}

js_bindings! {
    #[js(call = "Reflect.get")]
    fn get(object: JsValue, key: JsValue) -> JsValue;
    #[js(get = "length")]
    fn length(array: JsValue) -> usize;
    #[js(index)]
    fn unit(array: JsValue, index: usize) -> u32;
    #[js(new = "Array")]
    fn array(length: usize) -> JsValue;
    #[js(index_set)]
    fn put(array: JsValue, index: usize, value: u32);
}

fn result(data: &[u16]) -> JsValue {
    let out = array(data.len());
    for (i, &n) in data.iter().enumerate() {
        put(out, i, n as u32);
    }
    out
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let format = match get(input, JsValue::erase("format")).read::<&str>() {
        "raw" => Format::Raw,
        "utf16" => Format::Utf16,
        "base64" => Format::Base64,
        "uri" => Format::Uri,
        "bytes" => Format::Bytes,
        _ => panic!("unknown format"),
    };
    let op = get(input, JsValue::erase("op")).read::<&str>();
    let data = get(input, JsValue::erase("data"));
    if data.is_null() || data.is_undefined() {
        return array(0);
    }
    let rep = get(input, JsValue::erase("repeat"));
    let repeat = if rep.is_undefined() || op == "decompress" {
        1
    } else {
        rep.read::<f64>() as usize
    };
    let mut source = Vec::with_capacity(length(data) * repeat);
    for _ in 0..repeat {
        for i in 0..length(data) {
            source.push(unit(data, i) as u16);
        }
    }
    match op {
        "compress" => result(&compress(&source, format)),
        "decompress" => match decompress(&source, format) {
            Some(v) => result(&v),
            None => JsValue::null(),
        },
        "roundtrip" => {
            let packed = compress(&source, format);
            let restored = decompress(&packed, format).unwrap();
            if restored != source {
                panic!("roundtrip mismatch");
            }
            let out = array(2);
            put(out, 0, packed.len() as u32);
            put(out, 1, restored.len() as u32);
            out
        }
        _ => panic!("unknown operation"),
    }
}
