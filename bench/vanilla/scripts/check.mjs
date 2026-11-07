// The node smoke test for the compiled benchmark app.
//
//   node scripts/check.mjs [dist/main.js]
//
// A fake DOM records every operation the program performs, `Math.random` is replaced by a seeded
// stream, and the six buttons and the delegated click are driven through the listeners
// bootstrap.js registered. That makes two things checkable without a browser: the semantics the
// benchmark specifies (ids, labels, the every-tenth update, the 1 <-> 998 swap) and the recycling
// invariants the harness's own `isKeyed` check looks for (a second run clones nothing, a delete
// shifts text up and drops the LAST element, a swap moves no nodes). It is the local rehearsal
// for that gate.

import { readFileSync } from "node:fs";
import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const bundle = process.argv[2] ?? path.resolve(here, "..", "dist", "main.js");

// ---------------------------------------------------------------------------
// The recorder
// ---------------------------------------------------------------------------

const ops = { cloneNode: 0, appendChild: 0, removeChild: 0, textContent: 0 };
const reset = () => {
  for (const key of Object.keys(ops)) ops[key] = 0;
};

class El {
  constructor(tag) {
    this.tag = tag;
    this.children = [];
    this.parent = null;
    this.className = "";
    this.attrs = {};
    this.text = "";
  }
  get firstChild() {
    return this.children.length > 0 ? this.children[0] : null;
  }
  get nextSibling() {
    if (this.parent === null) return null;
    const at = this.parent.children.indexOf(this);
    return at + 1 < this.parent.children.length ? this.parent.children[at + 1] : null;
  }
  get sectionRowIndex() {
    return this.parent === null ? -1 : this.parent.children.indexOf(this);
  }
  set textContent(value) {
    ops.textContent += 1;
    this.text = String(value);
    for (const child of this.children) child.parent = null;
    this.children.length = 0;
  }
  get textContent() {
    return this.children.length > 0 ? this.children.map((c) => c.textContent).join("") : this.text;
  }
  set innerHTML(html) {
    this.children = parse(html, this);
  }
  appendChild(child) {
    ops.appendChild += 1;
    // A fragment empties into its new parent, which is the whole reason to have one.
    if (child.tag === "#fragment") {
      for (const grandchild of child.children) {
        grandchild.parent = this;
        this.children.push(grandchild);
      }
      child.children = [];
      return child;
    }
    child.parent = this;
    this.children.push(child);
    return child;
  }
  removeChild(child) {
    ops.removeChild += 1;
    const at = this.children.indexOf(child);
    assert.notEqual(at, -1, "removeChild was handed a node that is not a child");
    this.children.splice(at, 1);
    child.parent = null;
    return child;
  }
  cloneNode(deep) {
    assert.equal(typeof deep, "boolean", "cloneNode takes a boolean");
    // Counted once for the call, not once per descendant: what is being measured is how many
    // rows the program built, and a row is one call however deep the template is.
    ops.cloneNode += 1;
    return this.copyTree(deep);
  }
  copyTree(deep) {
    const copy = new El(this.tag);
    copy.className = this.className;
    copy.attrs = { ...this.attrs };
    copy.text = this.text;
    if (deep) {
      for (const child of this.children) {
        const clone = child.copyTree(true);
        clone.parent = copy;
        copy.children.push(clone);
      }
    }
    return copy;
  }
  closest(selector) {
    for (let at = this; at !== null; at = at.parent) if (at.tag === selector) return at;
    return null;
  }
  matches(selector) {
    return selector.split(",").some((part) => {
      const [own, descendant] = part.trim().split(/\s+/);
      const wanted = own.replace(".", "");
      if (descendant === undefined) return this.classes().includes(wanted);
      for (let at = this.parent; at !== null; at = at.parent) {
        if (at.classes().includes(wanted)) return true;
      }
      return false;
    });
  }
  classes() {
    return this.className.split(" ").filter((name) => name.length > 0);
  }
  addEventListener(name, handler) {
    (this.listeners ??= {})[name] = handler;
  }
}

// The markup one row is cloned from is written by the program, so this only has to read tags,
// attributes and closing tags: no text nodes, no entities, no void elements.
function parse(html, parent) {
  const out = [];
  let at = parent;
  const stack = [];
  for (const [, closing, tag, rest] of html.matchAll(/<(\/?)([a-z]+)([^>]*)>/g)) {
    if (closing === "/") {
      at = stack.pop();
      continue;
    }
    const element = new El(tag);
    for (const [, name, value] of rest.matchAll(/([a-zA-Z-]+)="([^"]*)"/g)) {
      if (name === "class") element.className = value;
      else element.attrs[name] = value;
    }
    element.parent = at;
    if (at === parent) out.push(element);
    else at.children.push(element);
    stack.push(at);
    at = element;
  }
  return out;
}

const byId = new Map();
for (const id of ["run", "runlots", "add", "update", "clear", "swaprows"]) {
  byId.set(id, new El("button"));
}
const tbody = new El("tbody");
byId.set("tbody", tbody);

globalThis.document = {
  getElementById: (id) => byId.get(id) ?? null,
  createElement: (tag) => new El(tag),
  createDocumentFragment: () => new El("#fragment"),
};

// ---------------------------------------------------------------------------
// The seeded stream
// ---------------------------------------------------------------------------

let state = 0;
const seed = (value) => {
  state = value >>> 0;
};
// mulberry32: short, and the only thing asked of it is that the same seed gives the same run.
globalThis.Math.random = () => {
  state = (state + 0x6d2b79f5) >>> 0;
  let t = state;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};

const ADJECTIVES = ["pretty", "large", "big", "small", "tall", "short", "long", "handsome",
  "plain", "quaint", "clean", "elegant", "easy", "angry", "crazy", "helpful", "mushy", "odd",
  "unsightly", "adorable", "important", "inexpensive", "cheap", "expensive", "fancy"];
const COLOURS = ["red", "yellow", "blue", "green", "pink", "brown", "purple", "brown", "white",
  "black", "orange"];
const NOUNS = ["table", "chair", "house", "bbq", "desk", "car", "pony", "cookie", "sandwich",
  "burger", "pizza", "mouse", "keyboard"];

const draw = (max) => Math.round(Math.random() * 1000) % max;
// The reference label stream: the same three draws in the same order the program makes them.
const labels = (count) => {
  const out = [];
  for (let i = 0; i < count; i++) {
    out.push(`${ADJECTIVES[draw(25)]} ${COLOURS[draw(11)]} ${NOUNS[draw(13)]}`);
  }
  return out;
};

// ---------------------------------------------------------------------------
// Driving it
// ---------------------------------------------------------------------------

new Function(readFileSync(bundle, "utf8"))();

const press = (id) => {
  reset();
  byId.get(id).listeners.click({ preventDefault() {}, target: byId.get(id) });
};
const clickRow = (index, what) => {
  reset();
  const row = tbody.children[index];
  const target = what === "remove"
    ? row.children[2].children[0].children[0]
    : row.children[1].children[0];
  tbody.listeners.click({ preventDefault() {}, target });
};

const ids = () => tbody.children.map((row) => row.children[0].text);
const texts = () => tbody.children.map((row) => row.children[1].children[0].text);
const danger = () => tbody.children.filter((row) => row.className === "danger");

let checks = 0;
const ok = (what) => {
  checks += 1;
  console.log(`ok  ${what}`);
};

// -- run ---------------------------------------------------------------------

seed(7);
press("run");
seed(7);
const firstRun = labels(1000);

assert.equal(tbody.children.length, 1000, "a thousand rows");
assert.deepEqual(ids(), Array.from({ length: 1000 }, (_, i) => String(i + 1)), "ids 1..1000");
assert.deepEqual(texts(), firstRun, "labels off the seeded stream");
assert.equal(ops.cloneNode, 1000, "a clone per row, into an empty table");
assert.equal(ops.appendChild, 1001, "a thousand into the fragment, the fragment into the table");
ok(`run: 1000 rows, ids 1..1000, labels from the stream ("${firstRun[0]}")`);

const nodes = [...tbody.children];

// -- run again: the recycling invariant ---------------------------------------

seed(11);
press("run");
seed(11);
const secondRun = labels(1000);

assert.equal(ops.cloneNode, 0, "a replacement clones NOTHING");
assert.equal(ops.appendChild, 0, "and appends nothing");
assert.equal(ops.removeChild, 0, "and removes nothing");
assert.equal(ops.textContent, 2000, "it writes two cells a row and no more");
assert.deepEqual(tbody.children, nodes, "the very same elements, in the very same order");
assert.deepEqual(ids(), Array.from({ length: 1000 }, (_, i) => String(i + 1001)), "ids continue");
assert.deepEqual(texts(), secondRun, "the new labels");
ok("run again: zero clones, same 1000 elements reused, ids 1001..2000");

// -- add ----------------------------------------------------------------------

seed(13);
press("add");
seed(13);
const added = labels(1000);

assert.equal(tbody.children.length, 2000, "two thousand rows");
assert.equal(ops.cloneNode, 1000, "a clone for each appended row and no others");
assert.deepEqual(texts().slice(0, 1000), secondRun, "the rows already there are untouched");
assert.deepEqual(texts().slice(1000), added, "the appended rows");
assert.deepEqual(ids().slice(1000, 1002), ["2001", "2002"], "ids continue across the append");
assert.equal(ops.textContent, 2000, "only the new rows were written");
ok("add: 2000 rows, 1000 clones, the first 1000 untouched");

// -- update --------------------------------------------------------------------

const beforeUpdate = texts();
press("update");
const afterUpdate = texts();

assert.equal(ops.cloneNode, 0, "an update clones nothing");
assert.equal(ops.textContent, 400, "two cells for each of the 200 rows it touched");
for (let i = 0; i < afterUpdate.length; i++) {
  if (i % 10 === 0) {
    assert.equal(afterUpdate[i], `${beforeUpdate[i]} !!!`, `row ${i} got its bang`);
  } else {
    assert.equal(afterUpdate[i], beforeUpdate[i], `row ${i} did not`);
  }
}
ok("update: every 10th row from index 0 gained \" !!!\", and only those");

// -- select ----------------------------------------------------------------------

clickRow(4, "label");
assert.equal(ops.textContent, 0, "selecting syncs nothing");
assert.equal(ops.cloneNode + ops.appendChild + ops.removeChild, 0, "and moves no nodes");
assert.equal(danger().length, 1, "exactly one highlighted row");
assert.equal(danger()[0], tbody.children[4], "the one that was clicked");

clickRow(9, "label");
assert.equal(danger().length, 1, "still exactly one");
assert.equal(danger()[0], tbody.children[9], "the newly clicked one");
ok("select: exactly one .danger, no sync, no node moved");

// -- swap ---------------------------------------------------------------------

const beforeSwap = texts();
press("swaprows");
const afterSwap = texts();

assert.equal(ops.textContent, 4, "two cells on each of two rows");
assert.equal(ops.cloneNode + ops.appendChild + ops.removeChild, 0, "no node moved");
assert.equal(afterSwap[1], beforeSwap[998], "row 1 shows what row 998 showed");
assert.equal(afterSwap[998], beforeSwap[1], "and the other way round");
for (let i = 0; i < afterSwap.length; i++) {
  if (i !== 1 && i !== 998) assert.equal(afterSwap[i], beforeSwap[i], `row ${i} is unchanged`);
}
ok("swaprows: rows 1 and 998 exchanged by text, four writes, no node moved");

// -- remove ------------------------------------------------------------------

const beforeRemove = texts();
const beforeIds = ids();
const last = tbody.children[tbody.children.length - 1];
const kept = tbody.children.slice(0, tbody.children.length - 1);
clickRow(2, "remove");

assert.equal(tbody.children.length, beforeRemove.length - 1, "one row fewer");
assert.equal(ops.removeChild, 1, "one element left the table");
assert.equal(ops.cloneNode, 0, "and none was made");
assert.deepEqual(tbody.children, kept, "the elements that stayed are the LEADING ones");
assert.equal(last.parent, null, "the element that went is the LAST one");
assert.deepEqual(texts().slice(2), beforeRemove.slice(3), "every later row's text shifted up");
assert.deepEqual(ids().slice(2), beforeIds.slice(3), "and so did every later id");
assert.deepEqual(texts().slice(0, 2), beforeRemove.slice(0, 2), "the rows before it did not move");
ok("remove: text shifts up from the deleted row, the LAST element is the one dropped");

// -- clear -------------------------------------------------------------------

press("clear");
assert.equal(tbody.children.length, 0, "the table is empty");
assert.equal(danger().length, 0, "and nothing is highlighted");
ok("clear: the table is empty");

// -- runlots -----------------------------------------------------------------

press("runlots");
assert.equal(tbody.children.length, 10000, "ten thousand rows");
assert.equal(ops.cloneNode, 10000, "a clone each, the table having been emptied");
assert.equal(new Set(ids()).size, 10000, "ten thousand distinct ids");
ok("runlots: 10000 rows");

// -- and once more, to prove nothing above left the store confused -------------

press("run");
assert.equal(tbody.children.length, 1000, "back to a thousand");
assert.equal(ops.cloneNode, 0, "recycling the leading thousand");
assert.equal(ops.removeChild, 9000, "and dropping the other nine thousand");
ok("run after runlots: 1000 rows, zero clones, 9000 elements dropped from the end");

console.log(`\n${checks} checks passed`);
