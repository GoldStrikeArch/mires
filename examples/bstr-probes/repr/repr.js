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
}
