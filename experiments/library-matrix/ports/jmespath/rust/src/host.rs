use mires_js::{JsValue, js_bindings};
pub type V = JsValue;
pub type Answer = Result<V, &'static str>;
pub fn s(x: &str) -> V {
    V::erase(x)
}
pub fn num(n: f64) -> V {
    V::erase(n)
}
pub fn nil() -> V {
    V::null()
}
pub fn und() -> V {
    V::undefined()
}
pub fn text(v: V) -> &'static str {
    v.read::<&str>()
}
pub fn field(v: V, k: &str) -> V {
    prop(v, s(k))
}
pub fn set(v: V, k: &str, x: V) {
    put(v, s(k), x);
}
pub fn list(xs: &[V]) -> V {
    let a = array();
    for &x in xs {
        push(a, x);
    }
    a
}
pub fn node(t: &str) -> V {
    let n = obj();
    set(n, "type", s(t));
    n
}
pub fn named(t: &str, name: V) -> V {
    let n = node(t);
    set(n, "name", name);
    n
}
pub fn leaf(t: &str, v: V) -> V {
    let n = node(t);
    set(n, "value", v);
    n
}
pub fn children(t: &str, a: &[V]) -> V {
    let n = node(t);
    set(n, "children", list(a));
    n
}
pub fn child(n: V, i: usize) -> V {
    at(field(n, "children"), i)
}

js_bindings! {
    #[js(new = "Object")] pub fn obj() -> V;
    #[js(new = "Array")] pub fn array() -> V;
    #[js(call = "Reflect.get")] pub fn prop(v: V,k: V) -> V;
    #[js(call = "Reflect.set")] pub fn put(v: V,k: V,x: V) -> bool;
    #[js(get = "length")] pub fn len(v: V) -> usize;
    #[js(index)] pub fn at(v: V,i: usize) -> V;
    #[js(method = "push")] pub fn push(v: V,x: V) -> usize;
    #[js(call = "Array.isArray")] pub fn is_array(v: V) -> bool;
    #[js(call = "Object.keys")] pub fn keys(v: V) -> V;
    #[js(call = "Object.values")] pub fn values(v: V) -> V;
    #[js(call = "JSON.parse")] pub fn parse_json(v: V) -> V;
    #[js(call = "JSON.stringify")] pub fn stringify(v: V) -> V;
    #[js(call = "Object")] pub fn boxed(v: V) -> V;
    #[js(call = "String")] pub fn string(v: V) -> V;
    #[js(call = "Number")] pub fn number(v: V) -> f64;
    #[js(call = "parseInt")] pub fn parse_int(v: V) -> f64;
    #[js(call = "Math.floor")] pub fn floor(v: f64) -> f64;
    #[js(call = "Math.ceil")] pub fn ceil(v: f64) -> f64;
    #[js(method = "charCodeAt")] pub fn unit(v: V,i: usize) -> u32;
    #[js(method = "slice")] pub fn slice(v: V,start: usize,end: usize) -> V;
    #[js(method = "replace")] pub fn replace(v: V,from: V,to: V) -> V;
    #[js(method = "trimStart")] pub fn trim_start(v: V) -> V;
    #[js(method = "indexOf")] pub fn index_of(v: V,x: V) -> i32;
    #[js(method = "lastIndexOf")] pub fn last_index_of(v: V,x: V) -> i32;
    #[js(method = "indexOf")] pub fn index_from(v: V,x: V,i: i32) -> i32;
    #[js(method = "localeCompare")] pub fn locale_compare(v: V,b: V) -> i32;
    #[js(method = "join")] pub fn join(v: V,sep: V) -> V;
    #[js(method = "sort")] pub fn sort(v: V) -> V;
    #[js(method = "reverse")] pub fn reverse(v: V) -> V;
}
