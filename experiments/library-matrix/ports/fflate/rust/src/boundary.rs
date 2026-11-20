//! JS <-> Rust boundary for the fflate matrix contract (CONTRACT.md, "Input" / "Output").

use alloc::vec;
use alloc::vec::Vec;

use mires_js::{js_bindings, throw_value, JsUint8Array, JsValue};

use crate::codec::str_from_u8_utf16le;
use crate::deflate::{deflate_sync, CompressOptions};
use crate::framing::{
    decompress_sync, gunzip_sync, gzip_sync, unzlib_sync, zlib_sync, FlateError,
};
use crate::inflate::{inflate_sync, DecompressOptions};
use crate::stream::{
    CompressStream, DecompressStream, GunzipStream, InflateStream, StreamSink, UnzlibStream,
};

js_bindings! {
    #[js(call = "Reflect.get")]
    fn property(target: JsValue, key: JsValue) -> JsValue;

    #[js(get = "length")]
    fn length(value: JsValue) -> usize;

    #[js(index)]
    fn element(array: JsValue, index: usize) -> JsValue;

    #[js(method = "charCodeAt")]
    fn code_unit(text: JsValue, index: usize) -> u16;
}

#[inline]
fn get(object: JsValue, key: &str) -> Option<JsValue> {
    let value = property(object, JsValue::erase(key));
    if value.is_undefined() {
        None
    } else {
        Some(value)
    }
}

#[inline]
fn get_bool(object: JsValue, key: &str) -> bool {
    match get(object, key) {
        Some(v) => v.is_truthy(),
        None => false,
    }
}

/// Decoded payload bytes: borrows directly from a UTF-8 host `&str` without copying, or owns a
/// `Vec<u8>` for Latin-1 strings and JSON byte arrays.
enum Payload<'a> {
    Borrowed(&'a [u8]),
    Owned(Vec<u8>),
}

impl<'a> Payload<'a> {
    #[inline]
    fn as_bytes(&self) -> &[u8] {
        match self {
            Payload::Borrowed(s) => s,
            Payload::Owned(v) => v.as_slice(),
        }
    }

    fn into_vec(self) -> Vec<u8> {
        match self {
            Payload::Borrowed(s) => s.to_vec(),
            Payload::Owned(v) => v,
        }
    }
}

/// Reads a JS string as Latin-1 bytes (`str.charCodeAt(i) & 0xFF`, matching `strToU8(s, true)`).
fn read_latin1_bytes(text: JsValue) -> Vec<u8> {
    let len = length(text);
    let mut buf = vec![0u8; len];
    let sl: &mut [u8] = &mut buf;
    let mut i = 0;
    while i < len {
        sl[i] = (code_unit(text, i) & 0xFF) as u8;
        i += 1;
    }
    buf
}

/// Decodes a `Payload` object (`{ text, latin1? }` or `{ bytes }`) without copying UTF-8 strings.
fn read_payload(spec: JsValue) -> Payload<'static> {
    if let Some(bytes_val) = get(spec, "bytes") {
        let len = length(bytes_val);
        let mut buf = vec![0u8; len];
        let sl: &mut [u8] = &mut buf;
        let mut i = 0;
        while i < len {
            sl[i] = (element(bytes_val, i).read::<f64>() as u32 & 0xFF) as u8;
            i += 1;
        }
        return Payload::Owned(buf);
    }
    let text = property(spec, JsValue::erase("text"));
    if get_bool(spec, "latin1") {
        Payload::Owned(read_latin1_bytes(text))
    } else {
        Payload::Borrowed(text.read::<&str>().as_bytes())
    }
}

fn read_compress_options(input: JsValue) -> CompressOptions {
    let opts = match get(input, "options") {
        Some(o) => o,
        None => return CompressOptions::empty(),
    };
    CompressOptions {
        level: get(opts, "level").map(|v| v.read::<f64>() as u8),
        mem: get(opts, "mem").map(|v| v.read::<f64>() as u8),
        dictionary: get(opts, "dictionary").map(|p| read_payload(p).into_vec()),
        mtime: get(opts, "mtime").map(|v| v.read::<f64>()),
        filename: get(opts, "filename").map(read_latin1_bytes),
    }
}

fn read_decompress_options(input: JsValue) -> DecompressOptions {
    let opts = match get(input, "options") {
        Some(o) => o,
        None => return DecompressOptions::empty(),
    };
    DecompressOptions {
        dictionary: get(opts, "dictionary").map(|p| read_payload(p).into_vec()),
        out_size: get(opts, "outSize").map(|v| v.read::<f64>() as usize),
    }
}

fn run_compress_stream(mut stream: CompressStream, input: JsValue) -> Result<Vec<u8>, FlateError> {
    let mut sink = StreamSink::new(!get_bool(input, "noHandler"));
    let steps = property(input, JsValue::erase("steps"));
    let n = length(steps);
    let mut i = 0;
    while i < n {
        let step = element(steps, i);
        let action_val = property(step, JsValue::erase("action"));
        let action = action_val.read::<&str>();
        if action == "push" {
            let data = read_payload(property(step, JsValue::erase("data")));
            let is_final = get_bool(step, "final");
            stream.push(data.as_bytes(), is_final, &mut sink)?;
        } else if action == "flush" {
            let sync = get_bool(step, "sync");
            stream.flush(sync, &mut sink)?;
        } else {
            throw_value(JsValue::erase("fflate: unknown stream action"));
        }
        i += 1;
    }
    Ok(sink.out)
}

fn read_push_step(step: JsValue) -> (Payload<'static>, bool) {
    let action_val = property(step, JsValue::erase("action"));
    let action = action_val.read::<&str>();
    if action != "push" {
        throw_value(JsValue::erase("fflate: unknown stream action"));
    }
    (
        read_payload(property(step, JsValue::erase("data"))),
        get_bool(step, "final"),
    )
}

fn run_decompress_stream(kind: u8, input: JsValue) -> Result<Vec<u8>, FlateError> {
    let opts = read_decompress_options(input);
    let mut sink = StreamSink::new(!get_bool(input, "noHandler"));
    let steps = property(input, JsValue::erase("steps"));
    let n = length(steps);

    match kind {
        0 => {
            let mut strm = InflateStream::new(&opts);
            let mut i = 0;
            while i < n {
                let (data, is_final) = read_push_step(element(steps, i));
                strm.push(data.as_bytes(), is_final, &mut sink)?;
                i += 1;
            }
        }
        1 => {
            let mut strm = GunzipStream::new(&opts);
            let mut i = 0;
            while i < n {
                let (data, is_final) = read_push_step(element(steps, i));
                strm.push(data.as_bytes(), is_final, &mut sink)?;
                i += 1;
            }
        }
        2 => {
            let mut strm = UnzlibStream::new(&opts);
            let mut i = 0;
            while i < n {
                let (data, is_final) = read_push_step(element(steps, i));
                strm.push(data.as_bytes(), is_final, &mut sink)?;
                i += 1;
            }
        }
        _ => {
            let mut strm = DecompressStream::new(opts);
            let mut i = 0;
            while i < n {
                let (data, is_final) = read_push_step(element(steps, i));
                strm.push(data.as_bytes(), is_final, &mut sink)?;
                i += 1;
            }
        }
    }
    Ok(sink.out)
}

fn execute(input: JsValue) -> Result<Vec<u8>, FlateError> {
    let op_val = property(input, JsValue::erase("op"));
    let op = op_val.read::<&str>();
    match op {
        "deflateSync" => {
            let data = read_payload(property(input, JsValue::erase("data")));
            let opts = read_compress_options(input);
            Ok(deflate_sync(data.as_bytes(), &opts))
        }
        "inflateSync" => {
            let data = read_payload(property(input, JsValue::erase("data")));
            let opts = read_decompress_options(input);
            inflate_sync(data.as_bytes(), &opts)
        }
        "gzipSync" => {
            let data = read_payload(property(input, JsValue::erase("data")));
            let opts = read_compress_options(input);
            Ok(gzip_sync(data.as_bytes(), &opts))
        }
        "gunzipSync" => {
            let data = read_payload(property(input, JsValue::erase("data")));
            let opts = read_decompress_options(input);
            gunzip_sync(data.as_bytes(), &opts)
        }
        "zlibSync" => {
            let data = read_payload(property(input, JsValue::erase("data")));
            let opts = read_compress_options(input);
            Ok(zlib_sync(data.as_bytes(), &opts))
        }
        "unzlibSync" => {
            let data = read_payload(property(input, JsValue::erase("data")));
            let opts = read_decompress_options(input);
            unzlib_sync(data.as_bytes(), &opts)
        }
        "decompressSync" => {
            let data = read_payload(property(input, JsValue::erase("data")));
            let opts = read_decompress_options(input);
            decompress_sync(data.as_bytes(), &opts)
        }
        "strToU8" => {
            let text = property(input, JsValue::erase("text"));
            if get_bool(input, "latin1") {
                Ok(read_latin1_bytes(text))
            } else {
                Ok(text.read::<&str>().as_bytes().to_vec())
            }
        }
        "strFromU8" => {
            let data = read_payload(property(input, JsValue::erase("data")));
            let latin1 = get_bool(input, "latin1");
            Ok(str_from_u8_utf16le(data.as_bytes(), latin1))
        }
        "Deflate" => {
            let opts = read_compress_options(input);
            run_compress_stream(CompressStream::new_deflate(opts), input)
        }
        "Gzip" => {
            let opts = read_compress_options(input);
            run_compress_stream(CompressStream::new_gzip(opts), input)
        }
        "Zlib" => {
            let opts = read_compress_options(input);
            run_compress_stream(CompressStream::new_zlib(opts), input)
        }
        "Inflate" => run_decompress_stream(0, input),
        "Gunzip" => run_decompress_stream(1, input),
        "Unzlib" => run_decompress_stream(2, input),
        "Decompress" => run_decompress_stream(3, input),
        _ => throw_value(JsValue::erase("fflate: unknown operation")),
    }
}

fn error_message(code: u8) -> &'static str {
    match code {
        0 => "unexpected EOF",
        1 => "invalid block type",
        2 => "invalid length/literal",
        3 => "invalid distance",
        4 => "stream finished",
        5 => "no stream handler",
        6 => "invalid header",
        _ => "flate error",
    }
}

pub fn run(input: JsValue) -> JsValue {
    let expect_code = get_bool(input, "expectErrorCode");
    match execute(input) {
        Ok(bytes) => {
            if expect_code {
                throw_value(JsValue::erase("expected FlateError, but call succeeded"));
            }
            JsUint8Array::from_slice(&bytes).as_value()
        }
        Err(FlateError::Code(code)) => {
            if expect_code {
                let pair = [0xFFu8, code];
                JsUint8Array::from_slice(&pair).as_value()
            } else {
                throw_value(JsValue::erase(error_message(code)))
            }
        }
        Err(FlateError::RangeError) => {
            throw_value(JsValue::erase("offset is out of bounds"))
        }
    }
}
