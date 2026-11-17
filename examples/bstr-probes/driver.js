// The host half of the `js-str` differential probes (`scripts/bstr-probes.sh`): concatenated
// after the runtime shim and the compiled `probes.rs`, exactly as `scripts/test.sh` runs a core
// test. It prints the lines `native.rs` prints.
{
  const text = (units) => String.fromCharCode(...units);
  const INPUTS = [
    [0x70, 0x6c, 0x61, 0x69, 0x6e],
    [0xe9],
    [0xd83d, 0xde00, 0x78],
    [0x61, 0xd800, 0x62],
    [0xdc00],
    [0x78, 0xd83d],
    [0x4e2d, 0x6587],
    [0x3a9, 0x2248, 0xe7, 0x221a],
    [0xfeff, 0x62],
  ];
  const HOST_TEXT = [0x78, 0xd800, 0x79, 0xd83d, 0xde00];
  const CMP = [
    [[0xe000], [0xd800, 0xdc00]],
    [[0x61], [0xe9]],
    [[0xd83d, 0xde00], [0xfffd]],
    [[0xffff], [0xdbff, 0xdfff]],
  ];
  const DISTINCT = [
    [[0x78, 0xd800], [0x78, 0xdbff]],
    [[0x78, 0xd800], [0x78, 0xfffd]],
    [[0xe9], [0x65, 0x301]],
    [[0xd83d, 0xde00], [0xd83d, 0xde00]],
  ];
  const SLICES = [
    [[0x61, 0x62], 0, 2],
    [[0xe9], 0, 1],
    [[0x61, 0xd83d, 0xde00], 1, 3],
    [[0x61, 0xd83d, 0xde00], 1, 5],
  ];
  const NAMES = ["len", "chars", "tail", "upper", "first"];
  // A string the host was handed, as JSON: a lone surrogate shows as `\udXXX`, which
  // `console.log` would otherwise print as U+FFFD and hide.
  const q = (value) => JSON.stringify(value);

  rust_entry();
  INPUTS.forEach((units, n) => {
    const s = text(units);
    console.log(
      `host[${n}] len ${bstr_len(s)} chars ${bstr_chars(s)} echo ${q(bstr_echo(s))} tail ${q(bstr_tail(s))}`,
    );
    console.log(`host[${n}] echo_value ${q(bstr_echo_value(s))}`);
    NAMES.forEach((name, k) => console.log(`host[${n}] ${name} ${q(bstr_answer(s, k))}`));
  });
  CMP.forEach(([a, b], n) => console.log(`host_cmp[${n}] ${bstr_cmp(text(a), text(b))}`));
  DISTINCT.forEach(([a, b], n) => console.log(`distinct[${n}] ${bstr_distinct(text(a), text(b))}`));
  globalThis.bstrHostText = () => text(HOST_TEXT);
  globalThis.bstrHostSink = (s) => console.log(`sink ${s}`);
  console.log(`via_host ${bstr_via_host()}`);
  SLICES.forEach(([units, a, b], n) => {
    try {
      console.log(`slice[${n}] ${q(bstr_slice(text(units), a, b))}`);
    } catch (error) {
      if (!globalThis.__rt.is_abort(error)) throw error;
      console.log(`slice[${n}] aborted`);
    }
  });
}
