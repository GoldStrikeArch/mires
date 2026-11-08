// Asserts what `Event::target_value` and `Event::prevent_default` actually DO.
//
//   node scripts/event-accessor-check.mjs
//
// The dom suite cannot show this. Its trace records that a handler was installed -- as a `$$input`
// property or an `addEventListener` call -- and never invokes one, so the accessors inside a handler
// body are never reached. `contract/harness/trace.mjs` is the contract's, so this check goes around
// it rather than changing it, exactly as scripts/keyed-identity-check.mjs does for keyed rows.
//
// What it does instead: examples/dom-tests/21_event_accessors.rs exports two functions that take an
// `&Event` and nothing else, so a fake event can be handed straight to them. That is the whole
// mechanism -- no instrumentation, because there is nothing to intercept: the accessors compile to
// `event.target.value || ""` and `event.preventDefault()`, and a plain object with those members is
// as good an event as a real one for the purpose of proving the property read is the right one.
//
// build/domtest/21_event_accessors.js is what scripts/dom-test.sh leaves behind, so run that first.

import { fileURLToPath } from "node:url";
import fs from "node:fs";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const from = path.join(root, "build/domtest");
const run = path.join(root, "build/eventcheck");

const program = path.join(from, "21_event_accessors.js");
if (!fs.existsSync(program)) {
  console.log("FAIL  build/domtest/21_event_accessors.js is missing; run scripts/dom-test.sh first");
  process.exit(1);
}

// A run directory of its own, at the same depth below the root as build/domtest, so that the dom
// stub's path to the contract harness still resolves.
fs.mkdirSync(run, { recursive: true });
fs.writeFileSync(path.join(run, "package.json"), '{ "type": "module" }\n');
for (const name of ["shim.js", "topcoat-dom.js", "21_event_accessors.js"]) {
  fs.copyFileSync(path.join(from, name), path.join(run, name));
}

const compiled = await import(path.join(run, "21_event_accessors.js"));

let failures = 0;
const check = (label, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"}  ${label}${detail === undefined ? "" : `: ${detail}`}`);
  if (!ok) {
    failures += 1;
  }
};

// The value the user typed, which is the whole reason the accessor exists.
check(
  "the text the user typed comes back",
  compiled.read_target_value({ target: { value: "hello" } }) === "hello",
  JSON.stringify(compiled.read_target_value({ target: { value: "hello" } }))
);
check(
  "an empty field is the empty string",
  compiled.read_target_value({ target: { value: "" } }) === ""
);

// A `&str` return promises a string. An element with no `value` -- a `<div>` the event bubbled
// from -- must not hand Rust `undefined`, which would break the moment anything read its length.
const missing = compiled.read_target_value({ target: {} });
check("an element with no `value` is the empty string, not undefined", missing === "", JSON.stringify(missing));
check("and the answer is a string, which is what `&str` is", typeof missing === "string", typeof missing);
check(
  "so the length of what comes back is readable",
  compiled.read_target_value({ target: { value: "abcd" } }).length === 4
);

// The property read is `target`, not `currentTarget`: the value wanted is the element the event
// came FROM, which for a delegated handler is not the element the listener sits on.
check(
  "the value is read off `target`, not off the event itself",
  compiled.read_target_value({ value: "wrong", target: { value: "right" } }) === "right"
);

// prevent_default calls the method, once, on the event it was given.
let calls = 0;
let receiver = null;
const event = {
  target: { value: "" },
  preventDefault() {
    calls += 1;
    receiver = this;
  },
};
compiled.cancel(event);
check("prevent_default calls preventDefault exactly once", calls === 1, String(calls));
check("on the event it was handed", receiver === event);

console.log();
console.log(failures === 0 ? "all checks passed" : `${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
