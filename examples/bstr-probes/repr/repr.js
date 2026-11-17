// The host half of the representation probes (`repr.rs`). Every value the host is handed is
// printed as JSON; `js-str=bytes` must print exactly what `js-str=utf16` prints.
{
  const q = (value) => JSON.stringify(value);
  globalThis.bstrSee = (label, value) => {
    console.log(`see ${label}: ${q(value)}`);
    return value;
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
}
