import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

let classToggleWrites = 0;

class FakeNode {
  constructor(ownerDocument, nodeType, nodeName, data = "") {
    this.children = [];
    this.data = data;
    this.nodeName = nodeName;
    this.nodeType = nodeType;
    this.ownerDocument = ownerDocument;
    this.parentNode = null;
  }

  get childNodes() {
    return this.children;
  }

  get firstChild() {
    return this.children[0] ?? null;
  }

  get nextSibling() {
    if (this.parentNode === null) return null;
    const siblings = this.parentNode.children;
    const index = siblings.indexOf(this);
    return index < 0 || index + 1 === siblings.length ? null : siblings[index + 1];
  }

  get textContent() {
    if (this.nodeType === 3 || this.nodeType === 8) return this.data;
    return this.children.map(child => child.textContent).join("");
  }

  set textContent(value) {
    for (const child of this.children) child.parentNode = null;
    this.children = [];
    const text = String(value);
    if (text !== "") this.appendChild(this.ownerDocument.createTextNode(text));
  }

  appendChild(node) {
    return this.insertBefore(node, null);
  }

  insertBefore(node, reference) {
    if (reference !== null && reference.parentNode !== this) {
      throw new Error("the insertion reference belongs to another parent");
    }
    if (node === reference) return node;
    if (node.parentNode !== null) node.parentNode.removeChild(node);
    const index = reference === null ? this.children.length : this.children.indexOf(reference);
    this.children.splice(index, 0, node);
    node.parentNode = this;
    return node;
  }

  removeChild(node) {
    const index = this.children.indexOf(node);
    if (index < 0) throw new Error("the removed node is not a child");
    this.children.splice(index, 1);
    node.parentNode = null;
    return node;
  }

  replaceChild(node, previous) {
    const reference = previous.nextSibling;
    this.removeChild(previous);
    return this.insertBefore(node, reference);
  }

  remove() {
    if (this.parentNode !== null) this.parentNode.removeChild(this);
  }

  contains(target) {
    return this === target || this.children.some(child => child.contains(target));
  }

  cloneNode(deep = false) {
    const clone = new FakeNode(this.ownerDocument, this.nodeType, this.nodeName, this.data);
    if (deep) {
      for (const child of this.children) clone.appendChild(child.cloneNode(true));
    }
    return clone;
  }
}

class FakeElement extends FakeNode {
  constructor(ownerDocument, name) {
    super(ownerDocument, 1, name.toUpperCase());
    this.attributes = new Map();
    this.className = "";
    const classes = new Set();
    this.classList = {
      toggle: (name, enabled) => {
        classToggleWrites += 1;
        if (enabled) classes.add(name);
        else classes.delete(name);
        this.className = [...classes].join(" ");
      },
    };
    this.disabled = false;
    this.localName = name.toLowerCase();
    this.style = { removeProperty() {}, setProperty() {} };
    this.tagName = this.nodeName;
    this.value = "";
  }

  setAttribute(name, value) {
    this.attributes.set(name, String(value));
  }

  removeAttribute(name) {
    this.attributes.delete(name);
  }

  addEventListener() {}

  removeEventListener() {}

  cloneNode(deep = false) {
    const clone = new FakeElement(this.ownerDocument, this.localName);
    clone.attributes = new Map(this.attributes);
    clone.className = this.className;
    clone.disabled = this.disabled;
    clone.value = this.value;
    if (deep) {
      for (const child of this.children) clone.appendChild(child.cloneNode(true));
    }
    return clone;
  }
}

class FakeTemplate extends FakeElement {
  constructor(ownerDocument) {
    super(ownerDocument, "template");
    this.content = new FakeNode(ownerDocument, 11, "#document-fragment");
  }

  set innerHTML(html) {
    const match = /^<([a-z][a-z0-9-]*)>(.*)<\/\1>$/.exec(html);
    if (match === null) throw new Error(`unsupported test template: ${html}`);
    this.content.textContent = "";
    const element = new FakeElement(this.ownerDocument, match[1]);
    if (match[2] === "<!>") element.appendChild(this.ownerDocument.createComment(""));
    else if (match[2] !== "" && !match[2].includes("<")) {
      element.appendChild(this.ownerDocument.createTextNode(match[2]));
    } else if (match[2] !== "") {
      throw new Error(`unsupported test template child: ${html}`);
    }
    this.content.appendChild(element);
  }
}

const document = {
  activeElement: null,
  addEventListener() {},
  removeEventListener() {},
  createComment: data => new FakeNode(document, 8, "#comment", data),
  createElement: name => name === "template"
    ? new FakeTemplate(document)
    : new FakeElement(document, name),
  createElementNS: (_namespace, name) => new FakeElement(document, name),
  createTextNode: data => new FakeNode(document, 3, "#text", String(data)),
  importNode: (node, deep) => node.cloneNode(deep),
  querySelectorAll: () => [],
};
document.head = document.createElement("head");
document.body = document.createElement("body");
globalThis.document = document;
globalThis.window = globalThis;
globalThis.navigator ??= { userAgent: "node" };

const programPath = path.resolve(process.argv[2] ?? "");
const laneOnly = process.argv[3] === "--lane-only";
if (!process.argv[2]) {
  console.error("usage: keyed-compiled.test.mjs <compiled-program.mjs> [--lane-only]");
  process.exit(2);
}

const lane = JSON.parse(readFileSync(`${programPath}.lane.json`, "utf8"));
const source = readFileSync(programPath, "utf8");
assert.equal(lane.lane, "jir");
assert.equal(lane.fallback_count, 0);
assert.equal(lane.items.every(item => item.lane === "jir"), true);

assert.match(source, /__rt\.keyed_dom_view\(/);
assert.doesNotMatch(source, /keyed\.runtime-lane-not-ready/);
if (laneOnly) {
  assert.doesNotMatch(source, /__rt\.keyed_row\(/);
  console.log(`compiled keyed-list lane: pass (${path.basename(programPath)})`);
  process.exit(0);
}
const bindFixture = path.basename(programPath).startsWith("05_bind.");
const componentFixture = path.basename(programPath).startsWith("06_component.");
const macroFixture = path.basename(programPath).startsWith("07_macro.");
const selectorFixture = path.basename(programPath).startsWith("08_selector.");
const nestedFixture = path.basename(programPath).startsWith("09_nested.");
const conditionalFixture = path.basename(programPath).startsWith("10_conditional.");
const boundedCollectFixture = path.basename(programPath).startsWith("17_bounded_collect.");
const crossCrateFixture = path.basename(programPath).startsWith("18_cross_crate.");
const hostRowsFixture = path.basename(programPath).startsWith("19_host_rows.");

const here = path.dirname(fileURLToPath(import.meta.url));
const dom = await import(pathToFileURL(path.resolve(
  here,
  "..",
  "runtime",
  "dom",
  "dist",
  "topcoat-dom.js",
)).href);
const compiled = await import(pathToFileURL(programPath).href);
const rt = globalThis.__rt;
rt._keyed_states.clear();
rt.keyed_metrics_reset();

const parent = document.createElement("ul");
let dispose;
let output;
dom.createRoot(rootDispose => {
  dispose = rootDispose;
  output = compiled.rust_entry();
});

if (macroFixture) {
  assert.equal(output.nodeName, "UL");
  parent.appendChild(output);
  const [state] = rt._keyed_states;
  const rowNodes = state.order.flatMap(row => row.nodes);
  assert.deepEqual(rowNodes.map(node => node.textContent), ["1", "2", "3"]);
  assert.deepEqual(output.children, [...rowNodes, state.dom.end]);
  assert.equal(state.rows.size, 3);
  assert.equal(state.dom.end.parentNode, output);
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, 3);
  assert.equal(rt.keyed_metrics_snapshot().liveRowRecords, 3);

  dispose();
  assert.equal(state.disposed, true);
  assert.deepEqual(output.children, []);
  assert.deepEqual(parent.children, [output]);
  assert.equal(rt.keyed_metrics_snapshot().liveRowRecords, 0);
  assert.equal(rt.keyed_metrics_snapshot().transactionStates, 0);
  console.log(`compiled keyed-list macro: pass (${path.basename(programPath)})`);
  process.exit(0);
}

assert.equal(Array.isArray(output), true);
const nodes = output.flat(Infinity);
for (const node of nodes) parent.appendChild(node);

if (nestedFixture) {
  const states = [...rt._keyed_states];
  assert.equal(states.length, 3);
  const outer = states.find(state => [...state.rows.keys()].join(",") === "1,2");
  const one = states.find(state => [...state.rows.keys()].join(",") === "11,12");
  const two = states.find(state => [...state.rows.keys()].join(",") === "21");
  assert.ok(outer);
  assert.ok(one);
  assert.ok(two);
  const outerRows = new Map(outer.rows);
  const oneRows = new Map(one.rows);
  const twoRows = new Map(two.rows);
  const elements = () => parent.children.filter(node => node.nodeType === 1);
  assert.deepEqual(elements().map(node => node.textContent), ["update", "1", "2", "3"]);
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, 5);
  const factories = rt.keyed_metrics_snapshot().rowFactories;
  assert.equal(typeof elements()[0].$$click, "function");
  elements()[0].$$click({});

  assert.deepEqual([...outer.rows.keys()], [2, 1]);
  assert.equal(outer.rows.get(1), outerRows.get(1));
  assert.equal(outer.rows.get(2), outerRows.get(2));
  assert.deepEqual([...one.rows.keys()], [12, 11, 13]);
  assert.equal(one.rows.get(11), oneRows.get(11));
  assert.equal(one.rows.get(12), oneRows.get(12));
  assert.equal(two.rows.get(21), twoRows.get(21));
  assert.deepEqual(elements().map(node => node.textContent), ["update", "3", "20", "10", "30"]);
  assert.equal(rt.keyed_metrics_snapshot().rowFactories - factories, 1);

  dispose();
  for (const state of states) assert.equal(state.disposed, true);
  assert.deepEqual(parent.children, [elements()[0]]);
  assert.equal(rt.keyed_metrics_snapshot().liveRowRecords, 0);
  assert.equal(rt.keyed_metrics_snapshot().transactionStates, 0);
  console.log(`compiled keyed-list nested: pass (${path.basename(programPath)})`);
  process.exit(0);
}

const [state] = rt._keyed_states;
const rowNodes = () => state.order.flatMap(row => row.nodes);
const assertBindRows = expected => {
  for (const [key, label, active] of expected) {
    const node = state.rows.get(key).nodes[0];
    assert.equal(node.className, active ? "on" : "off");
    assert.equal(node.attributes.get("title"), String(label));
    assert.equal(node.disabled, active);
    assert.equal(node.value, label);
  }
};
const assertComponentRows = expected => {
  for (const [key, label] of expected) {
    const component = state.rows.get(key).nodes[0].firstChild;
    assert.equal(component.nodeName, "SPAN");
    assert.equal(component.attributes.get("title"), String(label));
  }
};
assert.equal(rowNodes().length, 3);
assert.deepEqual(rowNodes().map(node => node.textContent), ["1", "2", "3"]);
if (bindFixture) assertBindRows([[1, 1, false], [2, 2, true], [3, 3, false]]);
if (componentFixture) assertComponentRows([[1, 1], [2, 2], [3, 3]]);
if (conditionalFixture) {
  assert.equal(state.rows.get(1).nodes[0].firstChild.nodeName, "SPAN");
  assert.equal(state.rows.get(2).nodes[0].firstChild.nodeName, "STRONG");
  assert.equal(state.rows.get(3).nodes[0].firstChild.nodeName, "SPAN");
}
assert.equal(state.dom.end.nodeType, 8);
assert.deepEqual(parent.children, nodes);
assert.equal(state.rows.size, 3);
assert.equal(state.dom.end.parentNode, parent);
assert.equal(rt.keyed_metrics_snapshot().rowFactories, 3);
assert.equal(rt.keyed_metrics_snapshot().liveRowRecords, 3);

const rowNodeSet = new Set([...rowNodes(), state.dom.end]);
const controls = nodes.filter(node => !rowNodeSet.has(node));
if (/live: true/.test(source)) {
  const liveIndex = /liveIndex: true/.test(source);
  const factories = rt.keyed_metrics_snapshot().rowFactories;
  const rows = new Map(state.rows);
  const handlers = new Map([...rows].map(([key, row]) => [key, row.nodes[0].$$click]));
  if (!selectorFixture) {
    for (const [key, handler] of handlers) {
      assert.equal(typeof handler, "function");
      const expected = liveIndex ? state.order.indexOf(rows.get(key)) + 100 : key;
      assert.equal(handler({}), expected);
    }
  }

  if (controls.length === 0) {
    const first = rows.get(1);
    first.item.value = { id: 1, label: 10 };
    assert.equal(first.item.value.label, 10);
    assert.equal(first.liveItem.label, 10);
    assert.equal(first.value.textContent, "10");
    assert.equal(handlers.get(1)({}), 10, "the installed handler reads the live row item");
  } else if (selectorFixture) {
    assert.equal(controls.length, 1);
    assert.equal(state.rows.get(1).nodes[0].className, "selected");
    assert.equal(state.rows.get(2).nodes[0].className, "");
    assert.equal(state.rows.get(3).nodes[0].className, "");
    const scans = rt.keyed_metrics_snapshot().collectionScans;
    const toggles = classToggleWrites;
    controls[0].$$click({});
    assert.deepEqual([...state.rows.keys()], [1, 2, 3]);
    assert.equal(state.rows.get(1).nodes[0].className, "");
    assert.equal(state.rows.get(2).nodes[0].className, "selected");
    assert.equal(state.rows.get(3).nodes[0].className, "");
    assert.equal(classToggleWrites - toggles, 2);
    assert.equal(rt.keyed_metrics_snapshot().collectionScans, scans);
  } else {
    assert.equal(controls.length, 1);
    assert.equal(typeof controls[0].$$click, "function");
    controls[0].$$click({});
    assert.deepEqual([...state.rows.keys()], [3, 1, 2]);
    for (const [key, row] of rows) assert.equal(state.rows.get(key), row);
    if (liveIndex) {
      assert.deepEqual(rowNodes().map(node => node.textContent), ["1", "2", "3"]);
      for (const [key, handler] of handlers) {
        assert.equal(handler({}), state.order.indexOf(rows.get(key)) + 100);
      }
    } else {
      assert.deepEqual(rowNodes().map(node => node.textContent), ["30", "10", "20"]);
      assert.equal(handlers.get(1)({}), 10);
      assert.equal(handlers.get(2)({}), 20);
      assert.equal(handlers.get(3)({}), 30);
      if (bindFixture) assertBindRows([[1, 10, true], [2, 20, false], [3, 30, true]]);
      if (componentFixture) assertComponentRows([[1, 10], [2, 20], [3, 30]]);
      if (conditionalFixture) {
        assert.equal(state.rows.get(1).nodes[0].firstChild.nodeName, "STRONG");
        assert.equal(state.rows.get(2).nodes[0].firstChild.nodeName, "SPAN");
        assert.equal(state.rows.get(3).nodes[0].firstChild.nodeName, "STRONG");
      }
    }
  }
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, factories);
}

if (boundedCollectFixture || crossCrateFixture || hostRowsFixture) {
  assert.equal(controls.length, 1);
  assert.equal(typeof controls[0].$$click, "function");
  const factories = rt.keyed_metrics_snapshot().rowFactories;
  const rows = new Map(state.rows);
  controls[0].$$click({});
  assert.deepEqual([...state.rows.keys()], [3, 1, 2]);
  for (const [key, row] of rows) assert.equal(state.rows.get(key), row);
  assert.deepEqual(rowNodes().map(node => node.textContent), ["3", "1", "2"]);
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, factories);
}

dispose();
assert.equal(state.disposed, true);
assert.deepEqual(parent.children, controls);
assert.equal(rt.keyed_metrics_snapshot().liveRowRecords, 0);
assert.equal(rt.keyed_metrics_snapshot().transactionStates, 0);

console.log(`compiled keyed-list test: pass (${path.basename(programPath)})`);
