use crate::functions;
use crate::host::*;
use core::cmp::Ordering;

pub fn kind(v: V) -> &'static str {
    if v.is_null() {
        "null"
    } else if v.is_string() {
        "string"
    } else if v.is_number() {
        "number"
    } else if v.is_boolean() {
        "boolean"
    } else if is_array(v) {
        "array"
    } else if v.is_object() {
        if field(v, "jmespathType").strictly_equals(s("Expref")) {
            "expref"
        } else {
            "object"
        }
    } else {
        "undefined"
    }
}
fn falsy(v: V) -> bool {
    match kind(v) {
        "null" => true,
        "boolean" => !v.read::<bool>(),
        "array" | "string" => len(v) == 0,
        "object" | "expref" => len(keys(v)) == 0,
        _ => false,
    }
}
fn equal(a: V, b: V) -> bool {
    if a.strictly_equals(b) {
        return true;
    }
    let t = kind(a);
    if t != kind(b) {
        return false;
    }
    if t == "array" {
        return len(a) == len(b) && (0..len(a)).all(|i| equal(at(a, i), at(b, i)));
    }
    if t == "object" {
        let ka = keys(a);
        if len(ka) != len(keys(b)) {
            return false;
        }
        for i in 0..len(ka) {
            let k = at(ka, i);
            if !equal(prop(a, k), prop(b, k)) {
                return false;
            }
        }
        return true;
    }
    false
}
pub fn compare(a: V, b: V) -> Option<Ordering> {
    let a = if a.is_object() && !a.is_null() {
        string(a)
    } else {
        a
    };
    let b = if b.is_object() && !b.is_null() {
        string(b)
    } else {
        b
    };
    if a.is_string() && b.is_string() {
        for i in 0..len(a).min(len(b)) {
            let ord = unit(a, i).cmp(&unit(b, i));
            if ord != Ordering::Equal {
                return Some(ord);
            }
        }
        Some(len(a).cmp(&len(b)))
    } else {
        number(a).partial_cmp(&number(b))
    }
}

pub fn eval(n: V, v: V) -> Answer {
    if n.is_undefined() || n.is_null() {
        return Err("syntax");
    }
    let k = text(field(n, "type"));
    Ok(match k {
        "Field" => {
            if !matches!(kind(v), "object" | "expref") {
                nil()
            } else {
                let x = prop(v, field(n, "name"));
                if x.is_undefined() { nil() } else { x }
            }
        }
        "Identity" | "Current" => v,
        "Literal" => field(n, "value"),
        "Pipe" | "IndexExpression" | "Subexpression" => eval(child(n, 1), eval(child(n, 0), v)?)?,
        "Index" => {
            if !is_array(v) {
                nil()
            } else {
                let p = number(field(n, "value"));
                let idx = if p < 0.0 { p + len(v) as f64 } else { p };
                if idx >= 0.0 && idx < len(v) as f64 {
                    at(v, idx as usize)
                } else {
                    nil()
                }
            }
        }
        "Slice" => {
            if !is_array(v) {
                return Ok(nil());
            }
            let p = child(n, 2);
            let step = if p.is_null() { 1 } else { number(p) as i64 };
            if step == 0 {
                return Err("invalid-value");
            }
            let length = len(v) as i64;
            let cap = |x: V, default: i64| -> i64 {
                if x.is_null() {
                    return default;
                }
                let mut y = number(x) as i64;
                if y < 0 {
                    y += length;
                }
                if step < 0 {
                    y.clamp(-1, length - 1)
                } else {
                    y.clamp(0, length)
                }
            };
            let mut i = cap(child(n, 0), if step < 0 { length - 1 } else { 0 });
            let stop = cap(child(n, 1), if step < 0 { -1 } else { length });
            let result = array();
            while if step < 0 { i > stop } else { i < stop } {
                push(result, at(v, i as usize));
                i = i.saturating_add(step);
            }
            result
        }
        "Projection" | "FilterProjection" | "ValueProjection" => {
            let mut src = eval(child(n, 0), v)?;
            if k == "ValueProjection" {
                if !matches!(kind(src), "object" | "expref") {
                    return Ok(nil());
                }
                src = values(src);
            }
            if !is_array(src) {
                return Ok(nil());
            }
            let result = array();
            for i in 0..len(src) {
                let item = at(src, i);
                if k == "FilterProjection" && falsy(eval(child(n, 2), item)?) {
                    continue;
                }
                let val = eval(child(n, 1), item)?;
                if !val.is_null() {
                    push(result, val);
                }
            }
            result
        }
        "Flatten" => {
            let src = eval(child(n, 0), v)?;
            if !is_array(src) {
                return Ok(nil());
            }
            let result = array();
            for i in 0..len(src) {
                let x = at(src, i);
                if is_array(x) {
                    for j in 0..len(x) {
                        push(result, at(x, j));
                    }
                } else {
                    push(result, x);
                }
            }
            result
        }
        "Comparator" => {
            let a = eval(child(n, 0), v)?;
            let b = eval(child(n, 1), v)?;
            let ok = match text(field(n, "name")) {
                "EQ" => equal(a, b),
                "NE" => !equal(a, b),
                "LT" => compare(a, b) == Some(Ordering::Less),
                "GT" => compare(a, b) == Some(Ordering::Greater),
                "LTE" => compare(a, b).is_some_and(|x| x != Ordering::Greater),
                _ => compare(a, b).is_some_and(|x| x != Ordering::Less),
            };
            V::erase(ok)
        }
        "AndExpression" | "OrExpression" => {
            let a = eval(child(n, 0), v)?;
            if falsy(a) == (k == "OrExpression") {
                eval(child(n, 1), v)?
            } else {
                a
            }
        }
        "NotExpression" => V::erase(falsy(eval(child(n, 0), v)?)),
        "MultiSelectHash" | "MultiSelectList" => {
            if v.is_null() {
                return Ok(nil());
            }
            let hash = k == "MultiSelectHash";
            let result = if hash { obj() } else { array() };
            let cs = field(n, "children");
            for i in 0..len(cs) {
                let c = at(cs, i);
                if hash {
                    put(result, field(c, "name"), eval(field(c, "value"), v)?);
                } else {
                    push(result, eval(c, v)?);
                }
            }
            result
        }
        "ExpressionReference" => {
            let c = child(n, 0);
            set(c, "jmespathType", s("Expref"));
            c
        }
        "Function" => {
            let args = array();
            let cs = field(n, "children");
            for i in 0..len(cs) {
                push(args, eval(at(cs, i), v)?);
            }
            let name = field(n, "name");
            if !name.is_string() {
                return Err("unknown-function");
            }
            functions::call(text(name), args)?
        }
        _ => return Err("syntax"),
    })
}
