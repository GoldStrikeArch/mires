// The host half of the representation probes (`repr.rs`). Every value the host is handed is
// printed as JSON; `js-str=bytes` must print exactly what `js-str=utf16` prints.
{
  const q = (value) => JSON.stringify(value);
  const kept = new Map();
  const wm = new WeakMap();
  const seenSet = new Set();

  const walkBtree = (map) => {
    const out = [];
    const seen = new Set();
    const visit = (nodePtr, height) => {
      if (!nodePtr || seen.has(nodePtr)) return;
      seen.add(nodePtr);
      const leaf = nodePtr.data ? nodePtr.data : nodePtr;
      for (let i = 0; i < leaf.len; i++) {
        out.push([leaf.keys[i], leaf.vals[i]]);
      }
      if (height > 0 && nodePtr.edges) {
        for (let i = 0; i <= leaf.len; i++) {
          visit(nodePtr.edges[i], height - 1);
        }
      }
    };
    if (map.root && map.root.TAG === "Some") {
      visit(map.root._0.node, map.root._0.height);
    }
    return out;
  };

  globalThis.bstrSee = (label, value) => {
    if (label === "btm-multi") {
      const pairs = walkBtree(value);
      console.log(`see ${label}: count=${pairs.length} first=${q(pairs[0])} last=${q(pairs[pairs.length - 1])}`);
      return value;
    }
    console.log(`see ${label}: ${q(value)}`);
    return value;
  };
  globalThis.bstrKeep = (label, value) => {
    const prev = kept.get(label);
    if (prev === undefined) {
      kept.set(label, value);
      wm.set(value, `tag:${label}`);
      seenSet.add(value);
      console.log(`keep-shape ${label}: isArr=${Array.isArray(value)} keys=${q(Object.keys(value))} val=${q(value)}`);
      return true;
    }
    const same = prev === value && wm.get(value) === `tag:${label}` && seenSet.has(value);
    return same;
  };
  globalThis.bstrReadKept = (label) => {
    const v = kept.get(label);
    console.log(`read-kept ${label}: ${q(v)}`);
    return v;
  };
  globalThis.bstrMutPair = (v) => {
    v[0] = "p\uD800é";
    v[1] += 10;
  };
  globalThis.bstrMutNamed = (v) => {
    v.name = "nm\uDC00ü";
    v.n += 5;
  };
  globalThis.bstrMutOpt = (v) => {
    v._0 = "op\uD800😀";
  };
  globalThis.bstrMutCell = (v) => {
    v.buf[v.off] = "cl\uD800ñ";
  };
  globalThis.bstrCall = (label, cb, arg) => {
    console.log(`call-cb ${label}: ${q(cb(arg))}`);
  };
  globalThis.bstrCall0 = (label, cb) => {
    console.log(`call-cb0 ${label}: ${q(cb())}`);
  };
  globalThis.bstrCall2 = (label, cb, a, b) => {
    console.log(`call-cb2 ${label}: ${q(cb(a, b))}`);
  };
  globalThis.bstrCheckCyc = (label, v) => {
    console.log(`check-cyc ${label}: label=${v.label} selfSame=${v.next._0 === v}`);
  };
  let leakedHetBuf;
  let leakedHetBufAfter;
  globalThis.bstrSeeSlot = (label, v) => {
    if (label === "het_slot") leakedHetBuf = v.buf;
    if (label === "het_slot_after_tup") leakedHetBufAfter = v.buf;
    console.log(`see-slot ${label}: s=${q(v.buf[v.off])} js=${q(v.buf[2])} n=${v.buf[3]}`);
    return v.buf;
  };
  globalThis.bstrMutHetTup = (v) => {
    v[0] = "new_é";
    v[1] = { TAG: "Some", _0: "mut_€" };
    v[3] = 99;
  };
  globalThis.bstrGetOpt = (which) => (which === 0 ? { TAG: "None" } : { TAG: "Some", _0: "ëins" });
  globalThis.bstrGetPair = () => ["zwëi", 2];
  globalThis.bstrGetNamed = () => ({ name: "drëi", n: 3 });
  globalThis.bstrMaybe = (which) => (which === 0 ? null : "vïer");
  globalThis.bstrTry = (which) => {
    if (which === 0) throw new Error("no");
    return "fünf";
  };
  globalThis.__rt.bstr_ext_pair = (value) => console.log(`ext_pair: ${q(value)}`);
  globalThis.__rt.bstr_ext_opt = () => ({ TAG: "Some", _0: "sëchs" });
  const runReflect = (label, mode, v, key) => {
    if (mode === 0) {
      const d = Object.getOwnPropertyDescriptor(v, key);
      console.log(`reflect ${label} desc: ${q(d && d.value)}`);
    } else if (mode === 1) {
      v[key] = "€";
    } else if (mode === 2) {
      Object.defineProperty(v, key, { value: "€", writable: true, configurable: true, enumerable: true });
    } else if (mode === 3) {
      Object.freeze(v);
      const val = v[key];
      let threw = false;
      try {
        (function () { "use strict"; v[key] = "x"; })();
      } catch (e) {
        threw = e instanceof TypeError;
      }
      console.log(`reflect ${label} freeze: val=${q(val)} threw=${threw}`);
    } else if (mode === 4) {
      Object.defineProperty(v, key, { writable: false });
      const d = Object.getOwnPropertyDescriptor(v, key);
      let threw = false;
      try {
        (function () { "use strict"; v[key] = "x"; })();
      } catch (e) {
        threw = e instanceof TypeError;
      }
      console.log(`reflect ${label} ro_cfg: w=${d && d.writable} e=${d && d.enumerable} c=${d && d.configurable} val=${q(d && d.value)} threw=${threw}`);
    } else if (mode === 5) {
      Object.defineProperty(v, key, { value: "€", writable: false, configurable: false });
      const d = Object.getOwnPropertyDescriptor(v, key);
      console.log(`reflect ${label} def_val_lock: e=${d && d.enumerable} w=${d && d.writable} c=${d && d.configurable} keys=${q(Object.keys(v))} json=${JSON.stringify(v)}`);
    } else if (mode === 6) {
      Object.defineProperty(v, key, { value: "€", configurable: false });
      (function () { "use strict"; v[key] = "ü"; })();
      const d = Object.getOwnPropertyDescriptor(v, key);
      console.log(`reflect ${label} def_val_keep_rw: w=${d && d.writable} e=${d && d.enumerable} val=${q(v[key])} keys=${q(Object.keys(v))} json=${JSON.stringify(v)}`);
    } else if (mode === 7) {
      let backing = "€";
      let gThis = false;
      let sThis = false;
      const g = function () { gThis = (this === v); return backing; };
      const s = function (next) { sThis = (this === v); backing = next; };
      Object.defineProperty(v, key, { get: g, set: s, enumerable: true, configurable: true });
      const r1 = v[key];
      v[key] = "😀";
      const r2 = v[key];
      const d = Object.getOwnPropertyDescriptor(v, key);
      globalThis.__r2LastBacking = () => backing;
      console.log(`reflect ${label} acc: r1=${q(r1)} r2=${q(r2)} back=${q(backing)} sameGet=${Boolean(d && d.get === g)} sameSet=${Boolean(d && d.set === s)} gThis=${gThis} sThis=${sThis}`);
    } else if (mode === 8) {
      const child = Object.create(v);
      child[key] = "€";
      console.log(`reflect ${label} proto_child: parent=${q(v[key])} child=${q(child[key])} own=${Object.prototype.hasOwnProperty.call(child, key)}`);
    }
  };
  globalThis.bstrReflectMut = (mode, v) => runReflect("bind", mode, v, "s");
  globalThis.bstrReflectArr = (mode, v) => runReflect("arr", mode, v, "0");
  globalThis.__rt.bstr_ext_reflect_mut = (mode, v) => runReflect("ext", mode, v, "s");
  rust_entry();
  console.log(`exp_opt: ${q(bstr_exp_opt({ TAG: "Some", _0: "ééé" }))} ${q(bstr_exp_opt({ TAG: "None" }))}`);
  console.log(`exp_pair: ${q(bstr_exp_pair(["日本", 1]))}`);
  console.log(`exp_named: ${q(bstr_exp_named({ name: "naïve", n: 0 }))}`);
  console.log(`exp_shape: ${bstr_exp_shape({ TAG: "Text", _0: "😀" })} ${bstr_exp_shape({ TAG: "Pair", left: "é", right: "中" })} ${bstr_exp_shape({ TAG: "Count", _0: 4 })} ${bstr_exp_shape({ TAG: "Empty" })}`);
  console.log(`exp_owned: ${bstr_exp_owned("äöü")}`);
  console.log(`exp_vec: ${q(bstr_exp_vec(1))}`);
  const jsObj = { name: "roundé", n: 99 };
  const retObj = bstr_exp_id(jsObj);
  console.log(`exp_id_same: ${retObj === jsObj} keys=${q(Object.keys(retObj))} val=${q(retObj)}`);
  const jsMutArr = ["beforeé", 5];
  const oldLen = bstr_exp_mut_pair(jsMutArr);
  console.log(`exp_mut_pair: oldLen=${oldLen} arr=${q(jsMutArr)}`);
  const fakeLeaf = {
    parent: { TAG: "None" },
    parent_idx: 0,
    len: 1,
    keys: ["k1é"],
    vals: ["v1😀"],
  };
  const fakeBtree = {
    length: 1,
    root: { TAG: "Some", _0: { height: 0, node: { buf: [fakeLeaf], off: 0 } } },
  };
  console.log(`exp_btree: ${bstr_exp_btree(fakeBtree)}`);
  const jsCyc = { label: "hostCyc😀", next: { TAG: "None" } };
  jsCyc.next = { TAG: "Some", _0: jsCyc };
  const retCyc = bstr_exp_cyc(jsCyc);
  console.log(`exp_cyc_same: ${retCyc === jsCyc} label=${retCyc.label}`);
  const jsHet = ["hé", { TAG: "Some", _0: "😀" }, 10];
  console.log(`exp_in_slot_tup: ${bstr_exp_in_slot({ buf: jsHet, off: 0 })} ${bstr_exp_in_tup(jsHet)}`);
  const jsHetRev = ["hé", { TAG: "Some", _0: "😀" }, 10];
  console.log(`exp_in_tup_slot: ${bstr_exp_in_tup(jsHetRev)} ${bstr_exp_in_slot({ buf: jsHetRev, off: 0 })}`);
  const frozenSlotBuf = Object.freeze(["hé", 10]);
  console.log(`exp_in_frozen_slot: ${bstr_exp_in_slot({ buf: frozenSlotBuf, off: 0 })}`);
  console.log(`exp_in_leaked_het: ${bstr_exp_in_het4(leakedHetBuf)} ${bstr_exp_in_het4(leakedHetBufAfter)}`);

  // Blocker 2 schema aliasing, readonly/frozen inputs, identity, reflection via exports, `#[track_caller]` export.
  console.log(`schema_fresh_B: ${bstr_exp_b_len({ s: "é", n: 7, t: "€" })}`);
  const ab1 = { s: "é", n: 7, t: "€" };
  console.log(`schema_A_then_B: ${bstr_exp_a_len(ab1)} ${bstr_exp_b_len(ab1)}`);
  const ab2 = { s: "é", n: 7, t: "€" };
  console.log(`schema_B_then_A: ${bstr_exp_b_len(ab2)} ${bstr_exp_a_len(ab2)}`);
  const roField = { n: 7 };
  Object.defineProperty(roField, "s", { value: "é", writable: false, enumerable: true, configurable: false });
  console.log(`schema_readonly: ${bstr_exp_a_len(roField)}`);
  console.log(`schema_frozen: ${bstr_exp_a_len(Object.freeze({ s: "é", n: 7 }))}`);
  const idObj = { s: "é", n: 7 };
  console.log(`schema_id_same: ${bstr_exp_a_identity(idObj) === idObj} ${bstr_exp_a_same(idObj, idObj)}`);
  for (let mode = 0; mode < 9; mode++) {
    globalThis.__r2LastBacking = null;
    const obj = { s: "é", n: 7 };
    const res = bstr_exp_reflect_export(mode, obj);
    const tail = mode === 7 && globalThis.__r2LastBacking ? ` afterRust=${q(globalThis.__r2LastBacking())} objS=${q(obj.s)}` : "";
    console.log(`exp_reflect mode=${mode} len=${res}${tail}`);
  }
  const hostLoc = { filename: "hôst.rs", line: 12, col: 4 };
  console.log(`tc_with_loc: ${tc_exported("é", hostLoc)} ${q(tc_exported_file("é", hostLoc))}`);
  console.log(`tc_ambig_with_loc: ${q(tc_ambig_exported({ name: "é", x: 3, y: 4 }, hostLoc))}`);
}
