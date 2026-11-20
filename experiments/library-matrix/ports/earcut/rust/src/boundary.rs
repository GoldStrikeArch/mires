//! JS <-> Rust boundary for the earcut 3.2.4 matrix contract (CONTRACT.md, "Input" / "Output").

use alloc::vec::Vec;

use mires_js::{js_bindings, throw_value, JsArray, JsValue};

use crate::earcut::earcut;
use crate::refine::refine;
use crate::util::deviation;

js_bindings! {
    #[js(call = "Reflect.get")]
    fn property(target: JsValue, key: JsValue) -> JsValue;

    #[js(get = "length")]
    fn length(value: JsValue) -> usize;

    #[js(index)]
    fn element(array: JsValue, index: usize) -> JsValue;

    #[js(index)]
    fn get_f64(array: JsValue, index: usize) -> f64;

    #[js(index)]
    fn get_u32(array: JsValue, index: usize) -> u32;

    #[js(new = "Object")]
    fn new_object() -> JsValue;

    #[js(new = "Array")]
    fn new_array_len(len: usize) -> JsValue;

    #[js(index_set)]
    fn set_js(target: JsValue, index: usize, value: JsValue);

    #[js(index_set)]
    fn set_u32(target: JsValue, index: usize, value: u32);

    #[js(index_set)]
    fn set_prop(target: JsValue, key: JsValue, value: JsValue);

    #[js(new = "TypeError")]
    fn new_type_error(message: JsValue) -> JsValue;
}

#[inline]
fn get_opt(object: JsValue, key: &str) -> Option<JsValue> {
    let v = property(object, JsValue::erase(key));
    if v.is_undefined() || v.is_null() {
        None
    } else {
        Some(v)
    }
}

#[inline]
fn get_dim(object: JsValue) -> usize {
    match get_opt(object, "dim") {
        Some(v) => v.read::<f64>() as usize,
        None => 2,
    }
}

fn read_f64_vec(arr: JsValue) -> Vec<f64> {
    let len = length(arr);
    let mut out = Vec::with_capacity(len);
    let mut i = 0;
    while i < len {
        out.push(get_f64(arr, i));
        i += 1;
    }
    out
}

fn read_u32_vec(arr: JsValue) -> Vec<u32> {
    let len = length(arr);
    let mut out = Vec::with_capacity(len);
    let mut i = 0;
    while i < len {
        out.push(get_u32(arr, i));
        i += 1;
    }
    out
}

fn read_optional_holes(object: JsValue) -> Option<Vec<u32>> {
    match get_opt(object, "holeIndices") {
        Some(v) => {
            let len = length(v);
            if len == 0 {
                None
            } else {
                Some(read_u32_vec(v))
            }
        }
        None => None,
    }
}

fn write_u32_array(vals: &[u32]) -> JsValue {
    let len = vals.len();
    let out = new_array_len(len);
    let mut i = 0;
    while i < len {
        set_u32(out, i, vals[i]);
        i += 1;
    }
    out
}

fn throw_type_error(msg: &str) -> ! {
    throw_value(new_type_error(JsValue::erase(msg)))
}

// The first point fixes the stride. JS array indexing preserves undefined for
// missing components of subsequent points, including an empty point.
fn flatten(rings: JsValue) -> JsValue {
    if length(rings) == 0 || length(element(rings, 0)) == 0 {
        throw_type_error("flatten: empty outer ring");
    }
    let dimensions = length(element(element(rings, 0), 0));
    let mut vertices = JsArray::new();
    let mut holes = JsArray::new();
    let mut hole_index = 0;
    let mut previous_length = 0;
    for i in 0..length(rings) {
        let ring = element(rings, i);
        for j in 0..length(ring) {
            let point = element(ring, j);
            for d in 0..dimensions {
                vertices.push(element(point, d));
            }
        }
        if previous_length != 0 {
            hole_index += previous_length;
            holes.push(JsValue::erase(hole_index as f64));
        }
        previous_length = length(ring);
    }
    let out = new_object();
    set_prop(out, JsValue::erase("vertices"), vertices.as_value());
    set_prop(out, JsValue::erase("holes"), holes.as_value());
    set_prop(out, JsValue::erase("dimensions"), JsValue::erase(dimensions as f64));
    out
}

pub fn run(input: JsValue) -> JsValue {
    let op_val = property(input, JsValue::erase("op"));
    let op = op_val.read::<&str>();

    if op == "earcut" {
        let data = read_f64_vec(property(input, JsValue::erase("data")));
        let holes = read_optional_holes(input);
        let dim = get_dim(input);
        let tris = earcut(&data, holes.as_deref(), dim);
        return write_u32_array(&tris);
    }

    if op == "earcutRefine" {
        let data = read_f64_vec(property(input, JsValue::erase("data")));
        let holes = read_optional_holes(input);
        let dim = get_dim(input);
        let mut tris = earcut(&data, holes.as_deref(), dim);
        refine(&mut tris, &data, dim);
        return write_u32_array(&tris);
    }

    if op == "refine" {
        let mut tris = read_u32_vec(property(input, JsValue::erase("triangles")));
        let coords = read_f64_vec(property(input, JsValue::erase("coords")));
        let dim = get_dim(input);
        refine(&mut tris, &coords, dim);
        return write_u32_array(&tris);
    }

    if op == "deviation" {
        let data = read_f64_vec(property(input, JsValue::erase("data")));
        let holes = read_optional_holes(input);
        let dim = get_dim(input);
        let tris = read_u32_vec(property(input, JsValue::erase("triangles")));
        let dev = deviation(&data, holes.as_deref(), dim, &tris);
        return JsValue::erase(dev);
    }

    if op == "flatten" {
        return flatten(property(input, JsValue::erase("rings")));
    }

    if op == "batch" {
        let items_js = property(input, JsValue::erase("items"));
        let ilen = length(items_js);
        let out = new_array_len(ilen);
        let mut i = 0;
        while i < ilen {
            let item = element(items_js, i);
            let data = read_f64_vec(property(item, JsValue::erase("data")));
            let holes = read_optional_holes(item);
            let dim = get_dim(item);
            let do_refine = match get_opt(item, "refine") {
                Some(v) => v.is_truthy(),
                None => false,
            };
            let mut tris = earcut(&data, holes.as_deref(), dim);
            if do_refine {
                refine(&mut tris, &data, dim);
            }
            set_js(out, i, write_u32_array(&tris));
            i += 1;
        }
        return out;
    }

    throw_type_error("unknown operation")
}
