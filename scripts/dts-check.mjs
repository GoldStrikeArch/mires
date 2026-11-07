// Asserts the properties of the emitted declaration files that a carelessly rewritten golden
// would break.
//
//   node scripts/dts-check.mjs
//
// The fixtures are examples/dts-tests/*.rs and build/dtstest/*.d.ts is what scripts/dts-test.sh
// leaves behind, so run that first.
//
// There is no TypeScript compiler in this tree, so nothing here can say the text type checks. What
// it can say is that each declaration means what the value model says the value is, which is the
// half a golden diff cannot: a golden accepts whatever was produced, and these say why.

import { fileURLToPath } from "node:url";
import fs from "node:fs";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const from = path.join(root, "build/dtstest");

let failures = 0;
const check = (label, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"}  ${label}${detail === undefined ? "" : `: ${detail}`}`);
  if (!ok) {
    failures += 1;
  }
};

const text = fs.readFileSync(path.join(from, "01_shapes.d.ts"), "utf8");

/** The declared body of `type Name = ...;`. */
const alias = (name) => text.match(new RegExp(`^type ${name} = (.*);$`, "m"))?.[1];
/** The declared signature of an exported function. */
const fn = (name) => text.match(new RegExp(`^export declare function ${name}\\((.*)\\): (.*);$`, "m"));

// ------------------------------------------------------------------ the file

check("every brace is balanced", (text.match(/{/g) ?? []).length === (text.match(/}/g) ?? []).length);
check("every declaration line ends in a semicolon", text
  .split("\n")
  .filter((line) => line.startsWith("export declare") || line.startsWith("type "))
  .every((line) => line.endsWith(";")));
// `any` would type check and mean nothing. `unknown` is the honest answer for a value the model
// keeps as its own bookkeeping, and it forces a caller to narrow.
check("nothing is `any`", !/\bany\b/.test(text), text.match(/.*\bany\b.*/)?.[0]);
check("the file says how it was generated", text.includes("js-dts=on"));

// ------------------------------------------------------------- the primitives

{
  const [, params, ret] = fn("primitives") ?? [];
  check("a u32 is a number", /count: number/.test(params), params);
  // The one primitive mistake that survives testing: a small u64 reads fine as a number.
  check("a u64 is a bigint, not a number", /big: bigint/.test(params), params);
  check("an f64 is a number", /ratio: number/.test(params), params);
  check("a bool is a boolean", /flag: boolean/.test(params), params);
  // A `char` is a code POINT in the value model, not a one-character string.
  check("a char is a number", /letter: number/.test(params), params);
  check("an i64 return is a bigint", ret === "bigint", ret);
}

check("a &str is a string", fn("greet")?.[1] === "name: string", fn("greet")?.[1]);
check("and so is a &'static str answer", fn("greet")?.[2] === "string", fn("greet")?.[2]);

// ---------------------------------------------------------------- the shapes

{
  const point = alias(fn("shift")[1].match(/point: (\w+)/)[1]);
  check("a struct is an object keyed by field name", point === "{ x: number; y: number }", point);
}

{
  const direction = alias(fn("turn")[2]);
  // The whole argument for the string-tag representation, in one line: a fieldless enum is
  // nameable, so TypeScript narrows a switch on it. As an integer it would have been `number`.
  check(
    "a fieldless enum is a union of string literals",
    direction === '"North" | "South"',
    direction
  );
}

check(
  "a repr(int) fieldless enum is a number, because the value IS the discriminant",
  fn("raise")?.[2] === "number",
  fn("raise")?.[2]
);

{
  const reply = alias(fn("describe")[2]);
  check("a payload enum is a discriminated union on TAG", /^\{ TAG: "Empty" \}/.test(reply), reply);
  check("a tuple variant carries its fields positionally", /TAG: "Text"; 0: number/.test(reply), reply);
  check("a struct variant carries them by name", /TAG: "Point"; x: number; y: number/.test(reply), reply);
  // The empty variant keeps the object form: a value with no identity cannot be written through,
  // and every `&mut Option<T>` in `core` writes exactly that value through exactly such a reference.
  check("and its empty variant is an object, not a bare string", !/\| "Empty"/.test(reply), reply);
}

{
  const option = alias(fn("maybe")[2]);
  check(
    "an Option is total: None is a value, never an absent one",
    /TAG: "None"/.test(option) && !/undefined|null/.test(option),
    option
  );
  check("and Some carries its payload", /TAG: "Some"; 0: number/.test(option), option);
}

// --------------------------------------------------------------- the handles

{
  const sig = fn("bump")?.[2];
  const node = fn("wrap")?.[2];
  check("a Sig is opaque, not the number its handle field says", sig === "Sig", sig);
  check("a Node is opaque too", node === "Node", node);
  check("and the two are distinct types", sig !== node);
  // A branded object type: TypeScript refuses to swap one for the other and refuses arithmetic on
  // either, which is the point of not spelling them `number`.
  check("each is branded rather than aliased to unknown", /unique symbol/.test(alias("Sig")), alias("Sig"));
}

// ------------------------------------------------------- the imported surface

check(
  "a declared interface's module is declared",
  /declare module "chart\.js" \{/.test(text),
  text.match(/declare module .*/)?.[0]
);
check(
  "with the local binding the program actually uses",
  /export const ext\$Chart\$\w+: unknown;/.test(text),
  text.match(/export const ext.*/)?.[0]
);
// A global imports nothing, so it is not part of the module's type surface: a `declare global`
// would claim the page has a name this module cannot supply.
check("and nothing claims a global", !/declare global/.test(text));

check("a tuple is an array", fn("split")?.[2] === "[number, number]", fn("split")?.[2]);

console.log(`${failures === 0 ? "PASS" : "FAIL"}: ${failures} failed`);
process.exit(failures === 0 ? 0 : 1);
