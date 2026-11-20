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
/// jmespath's `strictDeepEqual`: `===`, then the same `Object.prototype.toString` tag (expression
/// references are objects there), then by content. Objects compare every own key of `a` against
/// `b[key]`, so an undefined value equals a missing key, then require `b`'s own keys to be among
/// `a`'s.
fn equal(a: V, b: V) -> bool {
    if a.strictly_equals(b) {
        return true;
    }
    let tag = |v: V| match kind(v) {
        "expref" => "object",
        t => t,
    };
    let t = tag(a);
    if t != tag(b) {
        return false;
    }
    if t == "array" {
        return len(a) == len(b) && (0..len(a)).all(|i| equal(at(a, i), at(b, i)));
    }
    if t == "object" {
        let ka = keys(a);
        let kb = keys(b);
        return (0..len(ka)).all(|i| equal(prop(a, at(ka, i)), prop(b, at(ka, i))))
            && (0..len(kb)).all(|i| index_of(ka, at(kb, i)) >= 0);
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
        "Index" => index(n, v),
        "Slice" => slice(n, v)?,
        "Projection" => {
            let src = eval(child(n, 0), v)?;
            if !is_array(src) {
                return Ok(nil());
            }
            project(src, child(n, 1))?
        }
        "ValueProjection" => {
            let src = eval(child(n, 0), v)?;
            if !matches!(kind(src), "object" | "expref") {
                return Ok(nil());
            }
            project(values(src), child(n, 1))?
        }
        "FilterProjection" => filter_projection(n, v)?,
        "Flatten" => flatten(eval(child(n, 0), v)?),
        "Comparator" => comparator(n, v)?,
        "AndExpression" | "OrExpression" => {
            let a = eval(child(n, 0), v)?;
            if falsy(a) == (k == "OrExpression") {
                eval(child(n, 1), v)?
            } else {
                a
            }
        }
        "NotExpression" => V::erase(falsy(eval(child(n, 0), v)?)),
        "MultiSelectHash" | "MultiSelectList" => multi_select(n, v, k == "MultiSelectHash")?,
        "ExpressionReference" => {
            // jmespath tags the referenced node in place and returns the node itself.
            let c = child(n, 0);
            if c.is_undefined() || c.is_null() {
                return Err("syntax");
            }
            set(c, "jmespathType", s("Expref"));
            c
        }
        "Function" => call(n, v)?,
        _ => return Err("syntax"),
    })
}

fn index(n: V, v: V) -> V {
    if !is_array(v) {
        return nil();
    }
    let p = number(field(n, "value"));
    let idx = if p < 0.0 { p + len(v) as f64 } else { p };
    // `value[index]`, and null for undefined: off the end, or an undefined element (`[3:0:-]`).
    let x = if idx >= 0.0 && idx < len(v) as f64 { at(v, idx as usize) } else { und() };
    if x.is_undefined() { nil() } else { x }
}

/// jmespath's capSliceRange, in JS numbers.
fn cap_slice_range(length: f64, actual: f64, step: f64) -> f64 {
    if actual < 0.0 {
        let value = actual + length;
        if value < 0.0 {
            if step < 0.0 { -1.0 } else { 0.0 }
        } else {
            value
        }
    } else if actual >= length {
        if step < 0.0 { length - 1.0 } else { length }
    } else {
        actual
    }
}

/// `Slice` with jmespath's computeSliceParams, in JS numbers: the bounds are parseInt results, so
/// they may be NaN (`[0:3:-]`) or past 2^53, and the loop reads `value[i]` as it goes, which is
/// undefined one past the end.
fn slice(n: V, v: V) -> Answer {
    if !is_array(v) {
        return Ok(nil());
    }
    let step = child(n, 2);
    let step = if step.is_null() { 1.0 } else { number(step) };
    if step == 0.0 {
        return Err("invalid-value");
    }
    let length = len(v) as f64;
    let negative = step < 0.0;
    let start = child(n, 0);
    let mut i = if !start.is_null() {
        cap_slice_range(length, number(start), step)
    } else if negative {
        length - 1.0
    } else {
        0.0
    };
    let stop = child(n, 1);
    let stop = if !stop.is_null() {
        cap_slice_range(length, number(stop), step)
    } else if negative {
        -1.0
    } else {
        length
    };
    let result = array();
    while if step > 0.0 { i < stop } else { i > stop } {
        push(result, at(v, i as usize));
        i += step;
    }
    Ok(result)
}

/// A projection's right side over each element, keeping every result but null (undefined, from an
/// empty max_by, is kept).
fn project(src: V, rhs: V) -> Answer {
    let result = array();
    for i in 0..len(src) {
        let val = eval(rhs, at(src, i))?;
        if !val.is_null() {
            push(result, val);
        }
    }
    Ok(result)
}

/// Every condition first, then the projection, as jmespath does: the first error is its own.
fn filter_projection(n: V, v: V) -> Answer {
    let src = eval(child(n, 0), v)?;
    if !is_array(src) {
        return Ok(nil());
    }
    let filtered = array();
    for i in 0..len(src) {
        let item = at(src, i);
        if !falsy(eval(child(n, 2), item)?) {
            push(filtered, item);
        }
    }
    project(filtered, child(n, 1))
}

fn flatten(src: V) -> V {
    if !is_array(src) {
        return nil();
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

fn comparator(n: V, v: V) -> Answer {
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
    Ok(V::erase(ok))
}

fn multi_select(n: V, v: V, hash: bool) -> Answer {
    if v.is_null() {
        return Ok(nil());
    }
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
    Ok(result)
}

fn call(n: V, v: V) -> Answer {
    let args = array();
    let cs = field(n, "children");
    for i in 0..len(cs) {
        push(args, eval(at(cs, i), v)?);
    }
    let name = field(n, "name");
    if !name.is_string() {
        return Err("unknown-function");
    }
    functions::call(text(name), args)
}
