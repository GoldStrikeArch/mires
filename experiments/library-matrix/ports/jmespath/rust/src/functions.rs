use crate::eval::{compare, eval, kind};
use crate::host::*;
use mires_js::callback2;
use core::cmp::Ordering;

fn a_number(v: V) -> bool {
    v.is_number()
}

/// `sort_by`: decorate with the index, sort with the engine's `Array.prototype.sort`, and check
/// each key's type inside the comparator, as jmespath does. jmespath's first error ends its sort;
/// here the comparator records that error in `failure` and answers 0 to the calls the engine makes
/// after it, and the sorted result is discarded. With NaN keys the comparator is inconsistent, and
/// the engine's algorithm decides the order.
fn sort_by(a: V, expref: V) -> Answer {
    let items = slice(a, 0, len(a));
    if len(items) == 0 {
        return Ok(items);
    }
    let required = kind(eval(expref, at(items, 0))?);
    if !matches!(required, "number" | "string") {
        return Err("invalid-type");
    }
    let decorated = array();
    for i in 0..len(items) {
        push(decorated, num(i as f64));
    }
    let failure = array();
    let comparator = callback2(move |x, y| {
        if len(failure) > 0 {
            return num(0.0);
        }
        let (i, j) = (number(x), number(y));
        let order = eval(expref, at(items, i as usize)).and_then(|key_a| {
            let key_b = eval(expref, at(items, j as usize))?;
            if kind(key_a) != required || kind(key_b) != required {
                return Err("invalid-type");
            }
            Ok(match compare(key_a, key_b) {
                Some(Ordering::Greater) => 1.0,
                Some(Ordering::Less) => -1.0,
                _ => i - j,
            })
        });
        match order {
            Ok(order) => num(order),
            Err(category) => {
                push(failure, s(category));
                num(0.0)
            }
        }
    });
    sort_with(decorated, comparator);
    if len(failure) > 0 {
        return Err(text(at(failure, 0)));
    }
    let out = array();
    for i in 0..len(decorated) {
        push(out, at(items, number(at(decorated, i)) as usize));
    }
    Ok(out)
}

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
        // `_getTypeName` falls through for undefined (an empty max_by), and so does `type`.
        "type" => match kind(a) {
            "undefined" => und(),
            t => s(t),
        },
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
            let max = name == "max";
            if a_number(at(a, 0)) {
                // Math.max.apply / Math.min.apply: NaN wins, and +0 is larger than -0.
                let mut best = if max { f64::NEG_INFINITY } else { f64::INFINITY };
                for i in 0..len(a) {
                    let x = number(at(a, i));
                    best = if max { math_max(best, x) } else { math_min(best, x) };
                }
                num(best)
            } else {
                let mut best = at(a, 0);
                for i in 1..len(a) {
                    let item = at(a, i);
                    let replace = if max {
                        locale_compare(best, item) < 0
                    } else {
                        locale_compare(item, best) < 0
                    };
                    if replace {
                        best = item;
                    }
                }
                best
            }
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
        "sort_by" => sort_by(a, b)?,
        _ => return Err("unknown-function"),
    })
}
