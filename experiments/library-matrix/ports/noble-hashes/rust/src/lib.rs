#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
use alloc::{string::String, vec::Vec};
use mires_js::{js_bindings, js_export, throw_value, JsValue};
mod blake2;
mod sha2;
mod sha3;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { fail() }

fn fail() -> ! { throw_value(JsValue::erase("invalid hash input")) }

js_bindings! {
    #[js(call = "Reflect.get")]
    fn get(object: JsValue, key: JsValue) -> JsValue;
    #[js(get = "length")]
    fn len(value: JsValue) -> usize;
    #[js(index)]
    fn at(value: JsValue, index: usize) -> JsValue;
}

fn field(input: JsValue, key: &str) -> JsValue { get(input, JsValue::erase(key)) }
fn bytes(value: JsValue) -> Vec<u8> {
    if value.is_undefined() { return Vec::new(); }
    let mut data = Vec::with_capacity(len(value));
    for i in 0..len(value) { data.push(at(value,i).read::<f64>() as u8); }
    data
}

enum State {
    Sha2(sha2::Sha2),
    Hmac(sha2::HmacSha256),
    Keccak(sha3::Keccak),
    Blake(blake2::Blake2),
}

impl State {
    fn update(&mut self, block: &[u8]) {
        match self {
            Self::Sha2(s) => s.update(block),
            Self::Hmac(s) => s.update(block),
            Self::Keccak(s) => s.update(block),
            Self::Blake(s) => s.update(block),
        }
    }
    fn digest(self) -> Vec<u8> {
        match self {
            Self::Sha2(s) => s.digest(),
            Self::Hmac(s) => s.digest(),
            Self::Keccak(s) => s.digest(),
            Self::Blake(s) => s.digest(),
        }
    }
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    let name = field(input,"algorithm").read::<&str>();
    let mut state = match name {
        "sha256" => State::Sha2(sha2::Sha2::sha256()),
        "sha512" => State::Sha2(sha2::Sha2::sha512()),
        "sha3_256" | "keccak_256" => State::Keccak(sha3::Keccak::new(name == "keccak_256")),
        "hmac_sha256" => {
            let key = field(input,"key");
            if key.is_undefined() { fail(); }
            State::Hmac(sha2::HmacSha256::new(&bytes(key)))
        }
        "blake2b" | "blake2s" => {
            let wide = name == "blake2b";
            let max = if wide {64} else {32};
            let opt = field(input,"dkLen");
            let size = if opt.is_undefined() {max as f64} else {opt.read::<f64>()};
            let dk = size as usize;
            if dk < 1 || dk > max || dk as f64 != size { fail(); }
            let key = field(input,"key");
            let salt = field(input,"salt");
            let pers = field(input,"personalization");
            if !key.is_undefined() && (len(key) == 0 || len(key) > max) { fail(); }
            if !salt.is_undefined() && len(salt) != max/4 { fail(); }
            if !pers.is_undefined() && len(pers) != max/4 { fail(); }
            State::Blake(blake2::Blake2::new(wide,dk,&bytes(key),&bytes(salt),&bytes(pers)))
        }
        _ => fail(),
    };
    let raw = field(input,"bytes");
    if raw.is_undefined() {
        let blocks = field(input,"chunks");
        let mut chunks = Vec::with_capacity(len(blocks));
        for i in 0..len(blocks) { chunks.push(bytes(at(blocks,i))); }
        let rep = field(input,"repeat");
        let repeats = if rep.is_undefined() {1} else {rep.read::<f64>() as usize};
        for _ in 0..repeats {
            for block in &chunks { state.update(block); }
        }
    } else {
        state.update(&bytes(raw));
    }
    let digest = state.digest();
    let hex = b"0123456789abcdef";
    let mut text = String::with_capacity(digest.len()*2);
    for b in digest {
        text.push(hex[(b >> 4) as usize] as char);
        text.push(hex[(b & 15) as usize] as char);
    }
    JsValue::erase(text.as_str())
}
