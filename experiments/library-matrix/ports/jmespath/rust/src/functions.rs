use crate::eval::{compare, eval, kind};
use crate::host::*;
use alloc::vec::Vec;
use core::cmp::Ordering;

fn subtype(a: V, t: &str) -> bool {
    is_array(a) && (0..len(a)).all(|i| kind(at(a, i)) == t)
}
fn valid(value: V, signature: &str) -> bool {
    signature.split('|').any(|t| match t {
        "any" => true,
        "numbers" => subtype(value, "number"),
        "strings" => subtype(value, "string"),
        _ => kind(value) == t,
    })
}
fn signature(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "abs" | "floor" | "ceil" => &["number"],
        "avg" | "sum" => &["numbers"],
        "contains" => &["array|string", "any"],
        "starts_with" | "ends_with" => &["string", "string"],
        "length" => &["object|string|array"],
        "map" => &["expref", "array"],
        "max" | "min" | "sort" => &["numbers|strings"],
        "keys" | "values" | "merge" => &["object"],
        "max_by" | "min_by" | "sort_by" => &["array", "expref"],
        "join" => &["string", "strings"],
        "reverse" => &["string|array"],
        "to_array" | "to_number" | "to_string" | "type" | "not_null" => &["any"],
        _ => return None,
    })
}

pub fn call(name: &str, args: V) -> Answer {
    let sig = signature(name).ok_or("unknown-function")?;
    let variadic = matches!(name, "merge" | "not_null");
    if len(args) < sig.len() || (!variadic && len(args) != sig.len()) {
        return Err("invalid-arity");
    }
    for (i, t) in sig.iter().enumerate() {
        if !valid(at(args, i), t) {
            return Err("invalid-type");
        }
    }
    let a = at(args, 0);
    let b = at(args, 1);
    Ok(match name {
        "abs" => num(number(a).abs()),
        "ceil" => num(ceil(number(a))),
        "floor" => num(floor(number(a))),
        "avg" | "sum" => {
            let mut total = 0.0;
            for i in 0..len(a) {
                total += number(at(a, i));
            }
            num(if name == "avg" {
                total / (len(a) as f64)
            } else {
                total
            })
        }
        "contains" => V::erase(index_of(a, b) >= 0),
        "starts_with" => V::erase(last_index_of(a, b) == 0),
        "ends_with" => V::erase(index_from(a, b, len(a) as i32 - len(b) as i32) >= 0),
        "length" => num(if kind(a) == "object" {
            len(keys(a))
        } else {
            len(a)
        } as f64),
        "join" => join(b, a),
        "type" => s(kind(a)),
        "keys" => keys(a),
        "values" => values(a),
        "to_array" => {
            if is_array(a) {
                a
            } else {
                list(&[a])
            }
        }
        "to_number" => {
            if a.is_number() {
                a
            } else if a.is_string() && !number(a).is_nan() {
                num(number(a))
            } else {
                nil()
            }
        }
        "to_string" => {
            if a.is_string() {
                a
            } else {
                stringify(a)
            }
        }
        "not_null" => {
            let mut result = nil();
            for i in 0..len(args) {
                let v = at(args, i);
                if !v.is_null() {
                    result = v;
                    break;
                }
            }
            result
        }
        "reverse" => {
            if a.is_string() {
                let mut x = s("");
                for i in (0..len(a)).rev() {
                    x = x + slice(a, i, i + 1);
                }
                x
            } else {
                reverse(slice(a, 0, len(a)))
            }
        }
        "sort" => sort(slice(a, 0, len(a))),
        "merge" => {
            let out = obj();
            for i in 0..len(args) {
                let x = at(args, i);
                if x.is_null() || x.is_undefined() {
                    continue;
                }
                let ks = keys(x);
                for j in 0..len(ks) {
                    let key = at(ks, j);
                    put(
                        out,
                        key,
                        prop(if x.is_object() { x } else { boxed(x) }, key),
                    );
                }
            }
            out
        }
        "max" | "min" => {
            if len(a) == 0 {
                return Ok(nil());
            }
            let mut best = at(a, 0);
            for i in 1..len(a) {
                let item = at(a, i);
                let ord = if item.is_string() {
                    locale_compare(item, best).cmp(&0)
                } else {
                    compare(item, best).unwrap_or(Ordering::Equal)
                };
                if ord
                    == if name == "max" {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    }
                {
                    best = item;
                }
            }
            best
        }
        "map" => {
            let out = array();
            for i in 0..len(b) {
                push(out, eval(a, at(b, i))?);
            }
            out
        }
        "min_by" | "max_by" => {
            let max = name == "max_by";
            let mut extreme = num(if max {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            });
            let mut out = und();
            for i in 0..len(a) {
                let item = at(a, i);
                let key = eval(b, item)?;
                if !matches!(kind(key), "number" | "string") {
                    return Err("invalid-type");
                }
                if compare(key, extreme)
                    == Some(if max {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    })
                {
                    extreme = key;
                    out = item;
                }
            }
            out
        }
        "sort_by" => {
            if len(a) == 0 {
                return Ok(array());
            }
            let mut pairs = Vec::new();
            let t = kind(eval(b, at(a, 0))?);
            if !matches!(t, "number" | "string") {
                return Err("invalid-type");
            }
            for i in 0..len(a) {
                let item = at(a, i);
                let key = eval(b, item)?;
                if kind(key) != t {
                    return Err("invalid-type");
                }
                pairs.push((key, item));
            }
            pairs.sort_by(|x, y| compare(x.0, y.0).unwrap_or(Ordering::Equal));
            let out = array();
            for (_, item) in pairs {
                push(out, item);
            }
            out
        }
        _ => return Err("unknown-function"),
    })
}
