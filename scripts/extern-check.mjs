// Runs the compiled `#[js_extern]` fixture against the contract's fake chart library and compares
// its call trace with the contract's vectors.
//
//   node scripts/extern-check.mjs
//
// `contract/fixtures/js-extern/chart-lib.mjs` records the JavaScript-level operations performed on
// it; `vectors.json` holds, per call shape, the trace the reference emission left behind, produced
// by executing that emission rather than by transcribing it. `contract/harness/check-js-extern.mjs`
// exports `vector(id)` and `compareTrace(actual, expected)` for exactly this, and importing it
// re-runs the contract's own assertions about its drivers on the way past, which is worth having:
// a vector that stopped exercising its case fails here too.
//
// The emitted JavaScript does not have to match the reference emission's text. It has to be
// indistinguishable from it at the trace, which is what a descriptor is ABOUT: `new Chart(el, cfg)`
// and a `Chart(el, cfg)` factory both produce a chart, and only a recorder tells them apart.
//
// The fixture is examples/extern-tests/01_chart.rs; build/externtest/01_chart.js is what
// scripts/extern-test.sh leaves behind, so run that first. The last section drives
// examples/extern-tests/03_bytes_bulk.rs, the bulk byte crossings of `mires_js::JsUint8Array`.

import { fileURLToPath, pathToFileURL } from "node:url";
import fs from "node:fs";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const from = path.join(root, "build/externtest");
const harness = path.join(root, "contract/harness");
const fixtures = path.join(root, "contract/fixtures/js-extern");

// The compiled program imports the library by the specifier its declarations named. The run
// directory satisfies that specifier with a re-export of the contract's file, which is the same
// trick scripts/dom-test.sh uses for the DOM runtime: one module instance, so `__trace()` read
// from here sees what the compiled program did.
//
// `default` is re-exported by name because `export *` does not carry it, and the library exports
// its constructor twice as two DISTINCT function objects on purpose: a descriptor has to say which
// binding it imports, and identical objects would make that choice unobservable.
fs.writeFileSync(
  path.join(from, "chart-lib.mjs"),
  `export * from "../../contract/fixtures/js-extern/chart-lib.mjs";
export { default } from "../../contract/fixtures/js-extern/chart-lib.mjs";
`
);

const lib = await import(pathToFileURL(path.join(from, "chart-lib.mjs")).href);
const { env } = await import(pathToFileURL(path.join(fixtures, "drivers.mjs")).href);
const { vector, compareTrace } = await import(
  pathToFileURL(path.join(harness, "check-js-extern.mjs")).href
);
const js = await import(pathToFileURL(path.join(from, "01_chart.js")).href);

let failures = 0;
const check = (label, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"}  ${label}${detail === undefined ? "" : `: ${detail}`}`);
  if (!ok) {
    failures += 1;
  }
};

/** Build an instance the way the vectors' `setup` does, without recording it. */
const setup = () => {
  lib.__reset();
  const chart = js.v_new_named(env.canvas, env.config);
  lib.__reset();
  return chart;
};

/**
 * Drive one vector and compare.
 *
 * `run` returns what the compiled entry point answered. `returned` says how to check it: omit it
 * where the entry hands back the library's own value and the vector's `returned` is the claim, or
 * give a predicate where the Rust entry answers something of its own.
 */
const run = (id, drive, returned) => {
  const expected = vector(id);
  let answer;
  let threw = null;
  try {
    answer = drive();
  } catch (error) {
    threw = error;
  }

  const trace = lib.__trace().map((record) => JSON.parse(JSON.stringify(record)));
  const diffs = compareTrace(trace, expected.trace);
  check(`${id}: the trace is the vector's`, diffs.length === 0, diffs.join("\n      ") || undefined);

  if (expected.threw !== null && expected.threw !== undefined) {
    check(`${id}: it throws, as the vector says`, threw !== null, threw === null ? "it did not" : undefined);
    return;
  }
  check(`${id}: it does not throw`, threw === null, threw === null ? undefined : String(threw));
  if (threw !== null) {
    return;
  }

  if (typeof returned === "function") {
    check(`${id}: the value reached Rust`, returned(answer), JSON.stringify(lib.__value(answer)));
    return;
  }
  const normalized = JSON.stringify(lib.__value(answer));
  check(
    `${id}: the value handed back is the vector's`,
    normalized === JSON.stringify(expected.returned),
    normalized
  );
};

// -------------------------------------------------------------------------------- new
run("new-named", () => {
  lib.__reset();
  return js.v_new_named(env.canvas, env.config);
});
run("new-default", () => {
  lib.__reset();
  return js.v_new_default(env.canvas, env.config);
});
// The negative arm: a `new` shape that lowered to a plain call would be caught here, because the
// library refuses to be constructed without `new`.
run("new-without-new", () => {
  lib.__reset();
  return js.v_new_without_new(env.canvas, env.config);
});

// ------------------------------------------------------------------------------- send
run("send-one-arg", () => {
  const chart = setup();
  js.v_send_one_arg(chart, env.nextData);
  return undefined;
});
run("send-zero-args", () => {
  const chart = setup();
  js.v_send_zero_args(chart);
  return undefined;
});
// The two `resize` declarations differ only in arity, and `argc` in the vector is what tells them
// apart: a lowering that padded the shorter call with an explicit `undefined` fails here.
run("send-trailing-omitted", () => {
  const chart = setup();
  js.v_send_trailing_omitted(chart);
  return undefined;
});
run("send-trailing-undefined", () => {
  const chart = setup();
  js.v_send_trailing_undefined(chart, undefined);
  return undefined;
});

// --------------------------------------------------------------------------- nullable
// `getPoint(0)` answers an object and `getPoint(99)` answers `null`. The Rust entries read through
// the `Option` rather than handing it back, so what is checked is that the wrapper produced the
// right variant and that a `Some` carries the value and not just the tag.
run(
  "nullable-some",
  () => {
    const chart = setup();
    return js.v_nullable_some(chart);
  },
  (answer) => answer === env.config.data.points[0].x
);
run(
  "nullable-none",
  () => {
    const chart = setup();
    return js.v_nullable_none(chart);
  },
  (answer) => answer === true
);

// -------------------------------------------------------------------------------- get
run("get-property", () => {
  const chart = setup();
  return js.v_get_property(chart);
});
// A walk: two property reads, and the vector holds both.
run("get-scoped", () => {
  const chart = setup();
  return js.v_get_scoped(chart);
});
run("get-static", () => {
  lib.__reset();
  return js.v_get_static();
});

// -------------------------------------------------------------------------------- set
run("set-property", () => {
  const chart = setup();
  js.v_set_property(chart, "Revenue, restated");
  return undefined;
});

// ------------------------------------------------------------------------------ index
run("index-in-range", () => {
  const chart = setup();
  return js.v_index_in_range(chart);
});
run("index-length", () => {
  const chart = setup();
  return js.v_index_length(chart);
});
// Out of range answers `undefined`, which is NOT `null`: the two sit side by side in the fixture so
// a descriptor cannot conflate them, and this vector is the half that is not nullable.
run("index-out-of-range", () => {
  const chart = setup();
  return js.v_index_out_of_range(chart);
});

// ------------------------------------------------------------------------------- call
run("call-static", () => {
  lib.__reset();
  js.v_call_static(env.plugin);
  return undefined;
});

// ------------------------------------------------------------------------- byte buffers
// examples/extern-tests/03_bytes_bulk.rs: `mires_js::JsUint8Array` copies a whole byte range across
// the boundary with one host operation each way. Exact values and lengths, a slice that does not
// start at its buffer's first element, `i8` storage, and copy semantics in both directions.
const bytes = await import(pathToFileURL(path.join(from, "03_bytes_bulk.js")).href);
const rt = await import(pathToFileURL(path.join(from, "shim.js")).href);
const pattern = (length) => Uint8Array.from({ length }, (_, index) => (index * 7 + 3) & 255);
const same = (actual, expected) => actual instanceof Uint8Array
  && actual.length === expected.length
  && actual.every((byte, index) => byte === expected[index]);
const shown = (value) => value instanceof Uint8Array
  ? `Uint8Array(${value.length}) [${Array.from(value.subarray(0, 12)).join(", ")}${value.length > 12 ? ", ..." : ""}]`
  : String(value);
const exact = (label, run, expected) => {
  let actual;
  try {
    actual = run();
  } catch (error) {
    check(label, false, `threw ${String(error)}`);
    return;
  }
  check(label, same(actual, expected), same(actual, expected) ? undefined : shown(actual));
};

exact("bytes: an empty Vec crosses as an empty Uint8Array", () => bytes.bytes_pattern(0), []);
exact("bytes: one byte", () => bytes.bytes_pattern(1), [3]);
exact("bytes: 65,536 bytes, every value 0..=255", () => bytes.bytes_pattern(65536), pattern(65536));
exact("bytes: 0 and 255 and their neighbours", () => bytes.bytes_extremes(), [0, 255, 1, 254, 127, 128]);
exact("bytes: an empty literal", () => bytes.bytes_empty_literal(), []);
exact("bytes: a Vec that never allocated", () => bytes.bytes_empty_vec(), []);
exact("bytes: a slice at offset 100 of a larger buffer", () => bytes.bytes_middle(100, 164), pattern(300).subarray(100, 164));
exact("bytes: a slice ending at its buffer's last element", () => bytes.bytes_middle(299, 300), pattern(300).subarray(299));
exact("bytes: an empty slice inside a buffer", () => bytes.bytes_middle(150, 150), []);
exact("bytes: a Vec after push past capacity and truncate", () => bytes.bytes_push_truncate(), [25, 50, 75, 100, 125, 255]);
exact("bytes: i8 storage crosses as its bytes", () => bytes.bytes_signed(), [128, 255, 0, 1, 127]);
exact("bytes: a static byte string", () => bytes.bytes_static_and_str(0), [0, 255, ...new TextEncoder().encode("static")]);
exact("bytes: a &str's UTF-8 bytes", () => bytes.bytes_static_and_str(1), new TextEncoder().encode("héllo \u{1f600}"));
{
  const first = bytes.bytes_pattern(8);
  const second = bytes.bytes_pattern(8);
  check("bytes: every export is a fresh Uint8Array over its own buffer",
    first !== second && first.buffer !== second.buffer && first.byteOffset === 0
      && first.buffer.byteLength === 8);
}
for (const length of [0, 1, 7, 65536]) {
  let answer;
  try {
    answer = bytes.bytes_round_trip(length);
  } catch (error) {
    answer = `threw ${String(error)}`;
  }
  check(`bytes: Rust -> JS -> Rust round trip of ${length} bytes is equal`, answer === true, String(answer));
}
{
  const input = Uint8Array.from([0, 1, 128, 254, 255, 42]);
  const output = bytes.bytes_echo(input);
  check("bytes: JS -> Rust -> JS round trip is equal", same(output, input), shown(output));
  check("bytes: the round trip does not alias its input", output !== input && output.buffer !== input.buffer);
  const big = pattern(65536).reverse();
  exact("bytes: JS -> Rust -> JS round trip of 65,536 bytes", () => bytes.bytes_echo(big), big);
  exact("bytes: an empty Uint8Array imports as an empty Vec", () => bytes.bytes_echo(new Uint8Array(0)), []);
  const window = pattern(64).subarray(10, 20);
  exact("bytes: a subarray view imports only its own range", () => bytes.bytes_echo(window), window);
}
{
  const input = Uint8Array.from([9, 8, 7, 0, 255]);
  exact("bytes: copy_to_slice into the middle of a larger buffer",
    () => bytes.bytes_copy_into_middle(input, 3),
    [0xee, 0xee, 0xee, 9, 8, 7, 0, 255, 0xee, 0xee, 0xee]);
  let threw = null;
  try {
    bytes.bytes_copy_mismatch(Uint8Array.from([1, 2]));
  } catch (error) {
    threw = error;
  }
  check("bytes: copy_to_slice into a slice of another length panics", threw === "rust panic", String(threw));
  let accepted = null;
  try {
    bytes.bytes_echo([1, 2, 3]);
    accepted = "a plain array was accepted";
  } catch (error) {
    accepted = error instanceof TypeError ? null : String(error);
  }
  check("bytes: only a Uint8Array is accepted as one", accepted === null, accepted ?? undefined);
}
{
  let calls = 0;
  const failures = bytes.bytes_copy_isolation((view) => {
    calls += 1;
    for (let index = 0; index < view.length; index += 1) view[index] = (calls === 1 ? 200 : 144) + index;
  });
  check("bytes: writes after a crossing are never visible on the other side",
    failures === 0 && calls === 2, `failure bits ${failures}, ${calls} host call(s)`);
}
// The runtime helper on the carriers a compiled program can hand it, directly: typed backing
// storage (a typed `NativeVec` buffer), `i8` values in a plain array, an absent or frozen buffer
// behind a zero length, and an offset into a typed buffer.
{
  exact("bytes: runtime copy of a plain array at an offset",
    () => rt.u8array_from_bytes({ $b: [5, 6, 7, 8, 9], $o: 1, $l: 3 }), [6, 7, 8]);
  exact("bytes: runtime copy of i8 values",
    () => rt.u8array_from_bytes({ $b: [-128, -1, 0, 127], $o: 0, $l: 4 }), [128, 255, 0, 127]);
  exact("bytes: runtime copy of Uint8Array storage at an offset",
    () => rt.u8array_from_bytes({ $b: Uint8Array.from([1, 2, 3, 4]), $o: 2, $l: 2 }), [3, 4]);
  exact("bytes: runtime copy of Int8Array storage",
    () => rt.u8array_from_bytes({ $b: Int8Array.from([-1, -128, 5]), $o: 0, $l: 3 }), [255, 128, 5]);
  exact("bytes: runtime copy of an absent zero-length buffer",
    () => rt.u8array_from_bytes({ $b: undefined, $o: undefined, $l: 0 }), []);
  exact("bytes: runtime copy of the frozen dangling buffer",
    () => rt.u8array_from_bytes(rt.slice_from_parts(1, 0)), []);
  // Typed storage is sliced with the captured `subarray`, never the instance's own property.
  const ownSubarray = (answer) => {
    const view = Uint8Array.from([1, 2, 3, 4]);
    Object.defineProperty(view, "subarray", { value: answer });
    return view;
  };
  exact("bytes: runtime copy ignores a typed buffer's own subarray that answers other bytes",
    () => rt.u8array_from_bytes({ $b: ownSubarray(() => Uint8Array.from([9, 9, 9])), $o: 1, $l: 3 }), [2, 3, 4]);
  exact("bytes: runtime copy ignores a typed buffer's own subarray that throws",
    () => rt.u8array_from_bytes({ $b: ownSubarray(() => { throw new Error("own subarray ran"); }), $o: 0, $l: 2 }), [1, 2]);
  const storage = Uint8Array.from([1, 2, 3]);
  const copied = rt.u8array_from_bytes({ $b: storage, $o: 0, $l: 3 });
  copied[0] = 99;
  check("bytes: runtime copy of typed storage does not alias it", storage[0] === 1 && copied.buffer !== storage.buffer);
  const destination = { $b: [0, 0, 0, 0, 0], $o: 1, $l: 3 };
  rt.u8array_into_bytes(Uint8Array.from([255, 0, 128]), destination);
  check("bytes: runtime import writes exactly dst.$l bytes at dst.$o",
    JSON.stringify(destination.$b) === JSON.stringify([0, 255, 0, 128, 0]), JSON.stringify(destination.$b));
}
// The export and the length read are classified as unable to throw (`HostOperation::may_throw`),
// so whatever they are handed, the only exception allowed out is a registered compiler abort.
const aborts = (label, run) => {
  let threw = null;
  let answer;
  try {
    answer = run();
  } catch (error) {
    threw = { error };
  }
  const ok = threw !== null && rt.is_abort(threw.error);
  check(label, ok, ok ? undefined : threw === null
    ? `returned ${shown(answer)}`
    : `threw an ordinary ${threw.error instanceof Error ? threw.error.constructor.name : typeof threw.error}`);
};
{
  const hostile = new Proxy({}, { get() { throw new Error("host getter ran"); } });
  aborts("bytes: a host record with a negative length aborts", () => bytes.bytes_from_host_record({ $l: -1 }));
  aborts("bytes: a host record with a fractional length aborts", () => bytes.bytes_from_host_record({ $b: [1, 2], $o: 0, $l: 1.5 }));
  aborts("bytes: a host record whose getter throws aborts", () => bytes.bytes_from_host_record(hostile));
  aborts("bytes: a host record whose range overruns its buffer aborts", () => bytes.bytes_from_host_record({ $b: [1, 2], $o: 1, $l: 2 }));
  aborts("bytes: a host record with a field-key offset aborts", () => bytes.bytes_from_host_record({ $b: { b: 1 }, $o: "b", $l: 1 }));
  // The offset is checked for itself, over an array buffer whose range check alone would pass.
  aborts("bytes: an array record with a fractional offset aborts", () => bytes.bytes_from_host_record({ $b: [1, 2, 3], $o: 0.5, $l: 2 }));
  aborts("bytes: an array record with a negative offset aborts", () => bytes.bytes_from_host_record({ $b: [1, 2, 3], $o: -1, $l: 2 }));
  aborts("bytes: an array record with a string offset aborts", () => bytes.bytes_from_host_record({ $b: [5, 6, 7], $o: "0", $l: 1 }));
  // Typed storage is range-checked by its built-in length: `subarray` would clamp silently.
  aborts("bytes: a typed record whose range passes the end of its buffer aborts",
    () => bytes.bytes_from_host_record({ $b: Uint8Array.from([1, 2, 3]), $o: 2, $l: 4 }));
  aborts("bytes: a host record over a DataView aborts", () => bytes.bytes_from_host_record({ $b: new DataView(new ArrayBuffer(4)), $o: 0, $l: 2 }));
  aborts("bytes: a host record over BigInt storage aborts", () => bytes.bytes_from_host_record({ $b: [1n, 2n], $o: 0, $l: 2 }));
  aborts("bytes: a host record over a BigInt64Array aborts", () => bytes.bytes_from_host_record({ $b: new BigInt64Array(2), $o: 0, $l: 2 }));
  aborts("bytes: a length no Uint8Array can have aborts", () => bytes.bytes_from_host_record({ $b: [], $o: 0, $l: 2 ** 53 }));
  exact("bytes: a well-formed host record is copied", () => bytes.bytes_from_host_record({ $b: [1, 2, 3], $o: 1, $l: 2 }), [2, 3]);
  aborts("bytes: u64 storage viewed as bytes aborts", () => bytes.bytes_from_words());
  aborts("bytes: host values viewed as bytes abort when valueOf throws",
    () => bytes.bytes_from_values({ valueOf() { throw new Error("host valueOf ran"); } }));
  aborts("bytes: runtime export of undefined aborts", () => rt.u8array_from_bytes(undefined));
  const target = { $b: [7, 7, 7], $o: 0, $l: 3 };
  aborts("bytes: runtime import into a destination of another length aborts", () => rt.u8array_into_bytes(Uint8Array.from([1, 2]), target));
  check("bytes: the aborted import wrote nothing", target.$b.every((byte) => byte === 7), JSON.stringify(target.$b));
}
// The length read is classified as able to throw: a view that is not a `Uint8Array` is refused with
// the ordinary `TypeError` a mismatched host read raises, so cleanup runs, and no host code runs.
{
  const refuses = (label, value, run) => {
    let traps = 0;
    // Every trap kind is counted (`get`, `getPrototypeOf`, `has`, ...): reading the tag through
    // the captured accessor reaches none of them, where `instanceof` would walk the prototype.
    const counted = new Proxy(value, new Proxy({}, {
      get: (_, trap) => (...args) => { traps += 1; return Reflect[trap](...args); },
    }));
    let threw = null;
    try {
      run(counted);
    } catch (error) {
      threw = { error };
    }
    const ok = threw !== null && threw.error instanceof TypeError && !rt.is_abort(threw.error) && traps === 0;
    check(label, ok, ok ? undefined : threw === null ? "it did not throw"
      : `${rt.is_abort(threw.error) ? "abort" : String(threw.error)}, ${traps} trap(s)`);
  };
  refuses("bytes: runtime length of a Proxy over a Uint8Array is a TypeError, no trap run",
    Uint8Array.from([1, 2, 3]), (value) => rt.u8array_length(value));
  refuses("bytes: runtime length of an array-like is a TypeError, no trap run",
    { length: 3 }, (value) => rt.u8array_length(value));
  refuses("bytes: runtime length of an Int8Array is a TypeError",
    new Int8Array(3), (value) => rt.u8array_length(new Int8Array(3)));
  refuses("bytes: to_vec of a mistyped Proxy is a TypeError, no trap run",
    Uint8Array.from([1, 2, 3]), (value) => bytes.bytes_unchecked_to_vec(value));
  let walked = 0;
  const prototypeWalk = new Proxy(Uint8Array.from([1, 2, 3]), {
    getPrototypeOf() { walked += 1; throw new Error("getPrototypeOf trap ran"); },
  });
  let refused = null;
  try {
    rt.u8array_length(prototypeWalk);
  } catch (error) {
    refused = error;
  }
  check("bytes: runtime length never walks a Proxy's prototype (its getPrototypeOf trap throws)",
    refused instanceof TypeError && !rt.is_abort(refused) && walked === 0,
    `${String(refused)}, getPrototypeOf ran ${walked} time(s)`);
  exact("bytes: to_vec of a genuine Uint8Array through the unchecked declaration",
    () => bytes.bytes_unchecked_to_vec(Uint8Array.from([5, 6])), [5, 6]);
}
// An own `length` property on a genuine Uint8Array is never consulted: sizes come from the
// internal slots, and the copy checks them again.
{
  const shadowed = (real, fake) => {
    const view = Uint8Array.from(real);
    Object.defineProperty(view, "length", { get() { return fake; }, configurable: true });
    return view;
  };
  exact("bytes: an own length longer than the view does not lengthen the import", () => bytes.bytes_echo(shadowed([1, 2, 3], 5)), [1, 2, 3]);
  exact("bytes: an own length shorter than the view does not truncate the import", () => bytes.bytes_echo(shadowed([1, 2, 3], 1)), [1, 2, 3]);
  exact("bytes: copy_to_slice sizes from the view, not its own length",
    () => bytes.bytes_copy_into_middle(shadowed([4, 5, 6], 9), 1), [0xee, 4, 5, 6, 0xee]);
  let threw = null;
  try {
    bytes.bytes_copy_mismatch(shadowed([1, 2], 3));
  } catch (error) {
    threw = error;
  }
  check("bytes: an own length equal to the destination does not let a short view through", threw === "rust panic", String(threw));
  check("bytes: runtime length reads the internal slot", rt.u8array_length(shadowed([1, 2, 3], 5)) === 3);
}
// Detach: a view whose buffer is transferred after the conversion has length zero.
{
  let answer;
  try {
    answer = bytes.bytes_after_detach(Uint8Array.from([1, 2, 3]), (view) => {
      structuredClone(view.buffer, { transfer: [view.buffer] });
    });
  } catch (error) {
    answer = `threw ${String(error)}`;
  }
  check("bytes: a detached view has length zero and imports as an empty Vec", answer === 3000, String(answer));
}

console.log(failures === 0 ? "\nall checks passed" : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
