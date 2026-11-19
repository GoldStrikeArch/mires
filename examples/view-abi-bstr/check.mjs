import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const mode = process.argv[2];
if (mode === "--compare") {
  let ignoreMetrics = false;
  const paths = [];
  for (const arg of process.argv.slice(3)) {
    if (arg === "--ignore-metrics") ignoreMetrics = true;
    else paths.push(arg);
  }
  if (paths.length < 2) {
    console.error("usage: check.mjs --compare [--ignore-metrics] <a.json> <b.json> ...");
    process.exit(2);
  }
  const normalize = (snap) => {
    if (!ignoreMetrics) return snap;
    const copy = structuredClone(snap);
    for (const key of ["keyedSlice", "keyedVecLive", "keyedVecStatic"]) {
      if (copy[key]) delete copy[key].metrics;
    }
    return copy;
  };
  const snapshots = paths.map((p) => normalize(JSON.parse(readFileSync(p, "utf8"))));
  for (let i = 1; i < snapshots.length; i += 1) {
    assert.deepEqual(snapshots[i], snapshots[0], `snapshot ${paths[i]} differs from ${paths[0]}`);
  }
  console.log(`view-abi-bstr compare: pass (${paths.length} modes identical)`);
  process.exit(0);
}

if (mode !== "--run" || !process.argv[3]) {
  console.error("usage: check.mjs --run <program.mjs> [<out.json>] | --compare <a.json> <b.json>...");
  process.exit(2);
}

const programPath = path.resolve(process.argv[3]);
let outPath = null;
let expectedStrMode = null;
for (let i = 4; i < process.argv.length; i += 1) {
  if (process.argv[i] === "--str" && process.argv[i + 1]) {
    expectedStrMode = process.argv[i + 1];
    i += 1;
  } else if (!outPath) {
    outPath = path.resolve(process.argv[i]);
  }
}
const programDir = path.dirname(programPath);

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
    return this.children.map((child) => child.textContent).join("");
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
    return this === target || this.children.some((child) => child.contains(target));
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
    this.styles = new Map();
    this.classes = new Set();
    this.className = "";
    this.id = "";
    this.title = "";
    this.value = "";
    this.disabled = false;
    this.localName = name.toLowerCase();
    this.tagName = this.nodeName;
    this._listeners = new Map();
    this.classList = {
      toggle: (name, enabled) => {
        if (enabled) this.classes.add(name);
        else this.classes.delete(name);
        this.className = [...this.classes].sort().join(" ");
      },
    };
    this.style = {
      setProperty: (k, v) => {
        this.styles.set(k, String(v));
      },
      removeProperty: (k) => {
        this.styles.delete(k);
      },
    };
  }

  setAttribute(name, value) {
    this.attributes.set(name, String(value));
  }

  removeAttribute(name) {
    this.attributes.delete(name);
  }

  getAttribute(name) {
    return this.attributes.get(name) ?? null;
  }

  addEventListener(type, fn) {
    let list = this._listeners.get(type);
    if (!list) {
      list = [];
      this._listeners.set(type, list);
    }
    list.push(fn);
  }

  removeEventListener(type, fn) {
    const list = this._listeners.get(type);
    if (!list) return;
    const idx = list.indexOf(fn);
    if (idx >= 0) list.splice(idx, 1);
  }

  dispatchEvent(ev) {
    if (!ev.target) ev.target = this;
    if (!ev.preventDefault) {
      ev.defaultPrevented = false;
      ev.preventDefault = () => {
        ev.defaultPrevented = true;
      };
    }
    for (const fn of this._listeners.get(ev.type) ?? []) {
      fn(ev);
    }
    if (!ev.cancelBubble) {
      this.ownerDocument.dispatchEvent(ev);
    }
  }

  cloneNode(deep = false) {
    const clone = new FakeElement(this.ownerDocument, this.localName);
    clone.attributes = new Map(this.attributes);
    clone.styles = new Map(this.styles);
    clone.classes = new Set(this.classes);
    clone.className = this.className;
    clone.id = this.id;
    clone.title = this.title;
    clone.value = this.value;
    clone.disabled = this.disabled;
    if (deep) {
      for (const child of this.children) clone.appendChild(child.cloneNode(true));
    }
    return clone;
  }
}

function parseHtmlInto(ownerDocument, rootParent, html) {
  const stack = [rootParent];
  let i = 0;
  while (i < html.length) {
    if (html[i] !== "<") {
      const next = html.indexOf("<", i);
      const end = next < 0 ? html.length : next;
      stack[stack.length - 1].appendChild(ownerDocument.createTextNode(html.slice(i, end)));
      i = end;
      continue;
    }
    if (html.startsWith("<!", i)) {
      const isLong = html.startsWith("<!--", i);
      const close = isLong ? html.indexOf("-->", i + 4) : html.indexOf(">", i + 2);
      const text = isLong ? html.slice(i + 4, close) : "";
      stack[stack.length - 1].appendChild(ownerDocument.createComment(text));
      i = isLong ? close + 3 : close + 1;
      continue;
    }
    if (html.startsWith("</", i)) {
      const close = html.indexOf(">", i + 2);
      stack.pop();
      i = close + 1;
      continue;
    }
    const close = html.indexOf(">", i + 1);
    const inside = html.slice(i + 1, close).trim();
    const selfClosing = inside.endsWith("/");
    const body = selfClosing ? inside.slice(0, -1).trim() : inside;
    const space = body.search(/\s/);
    const tag = space < 0 ? body : body.slice(0, space);
    const el = ownerDocument.createElement(tag);
    if (space >= 0) {
      const attrStr = body.slice(space + 1);
      const attrRe = /([a-zA-Z0-9_-]+)="([^"]*)"/g;
      for (const m of attrStr.matchAll(attrRe)) {
        el.setAttribute(m[1], m[2]);
      }
    }
    stack[stack.length - 1].appendChild(el);
    if (!selfClosing) stack.push(el);
    i = close + 1;
  }
}

class FakeTemplate extends FakeElement {
  constructor(ownerDocument) {
    super(ownerDocument, "template");
    this.content = new FakeNode(ownerDocument, 11, "#document-fragment");
  }

  set innerHTML(html) {
    this.content.textContent = "";
    parseHtmlInto(this.ownerDocument, this.content, html);
  }
}

const docListeners = new Map();
const document = {
  activeElement: null,
  addEventListener(type, fn) {
    let list = docListeners.get(type);
    if (!list) {
      list = [];
      docListeners.set(type, list);
    }
    list.push(fn);
  },
  removeEventListener(type, fn) {
    const list = docListeners.get(type);
    if (!list) return;
    const idx = list.indexOf(fn);
    if (idx >= 0) list.splice(idx, 1);
  },
  dispatchEvent(ev) {
    for (const fn of docListeners.get(ev.type) ?? []) {
      fn(ev);
    }
  },
  createComment: (data) => new FakeNode(document, 8, "#comment", data),
  createElement: (name) =>
    name === "template" ? new FakeTemplate(document) : new FakeElement(document, name),
  createElementNS: (_ns, name) => new FakeElement(document, name),
  createTextNode: (data) => new FakeNode(document, 3, "#text", String(data)),
  importNode: (node, deep) => node.cloneNode(deep),
  querySelectorAll: () => [],
};
document.head = document.createElement("head");
document.body = document.createElement("body");
globalThis.document = document;
globalThis.window = globalThis;
globalThis.navigator ??= { userAgent: "node" };

const rustNotes = [];
const cleanupNotes = [];
globalThis.noteRust = (tag, byteLen, charLen, text) => {
  rustNotes.push({ tag, byteLen, charLen, text });
};
globalThis.noteCleanup = (tag) => {
  cleanupNotes.push(tag);
};
globalThis.resolvedEmoji = () => Promise.resolve("é😀");
globalThis.resolvedSurrogate = () => Promise.resolve("x\uD800y");

const dom = await import(pathToFileURL(path.join(programDir, "topcoat-dom.js")).href);
const compiled = await import(pathToFileURL(programPath).href);
const rt = globalThis.__rt;

function serializeNode(node) {
  if (node.nodeType === 3) return { text: node.data };
  if (node.nodeType === 8) return { comment: node.data };
  return {
    tag: node.localName,
    id: node.id || undefined,
    title: node.title || undefined,
    className: node.className || undefined,
    attrs: Object.fromEntries([...node.attributes.entries()].sort()),
    styles: Object.fromEntries([...node.styles.entries()].sort()),
    children: node.children.map(serializeNode),
  };
}

function mountList(out) {
  const host = document.createElement("div");
  const flat = Array.isArray(out) ? out.flat(Infinity) : [out];
  for (const node of flat) host.appendChild(node);
  return host;
}

const snapshot = {};

// 1. build_main: template, hole, effect_group, handler + Event::target_value, component, text, spread, style, classList
{
  let dispose;
  let mainNode;
  dom.createRoot((d) => {
    dispose = d;
    mainNode = compiled.build_main();
  });
  const span = mainNode.children[0];
  const p = mainNode.children[1];
  const input = mainNode.children[2];
  const section = mainNode.children[3];
  const aEl = mainNode.children[4];
  const bEl = mainNode.children[5];
  const aside = mainNode.children[6];

  assert.equal(mainNode.getAttribute("data-static"), "é😀");
  assert.equal(span.getAttribute("data-info"), "info-é😀-€");
  assert.equal(span.getAttribute("data-dyn"), "init-é😀");
  assert.equal(span.title, "init-é😀");
  assert.equal(span.styles.get("content"), '"é😀"');
  assert.equal(span.className, "active");
  assert.equal(p.textContent, "init-é😀");
  assert.equal(aEl.textContent, "é😀");
  assert.equal(bEl.textContent, "ascii-only");
  assert.equal(aside.getAttribute("id"), "spread-id");
  assert.equal(aside.getAttribute("title"), "spread-é😀");

  const article = section.children[0];
  assert.deepEqual(
    article.children.map((c) => c.textContent),
    ["init-é😀", "10", "€-7", "😀", "é😀"],
  );

  // Trigger delegated input event with non-ASCII and lone surrogate
  input.value = "new-€-😀";
  input.dispatchEvent({ type: "input", target: input });
  assert.equal(span.getAttribute("data-dyn"), "new-€-😀");
  assert.equal(span.title, "new-€-😀");
  assert.equal(p.textContent, "new-€-😀");
  assert.equal(span.className, "active");
  assert.equal(article.children[0].textContent, "new-€-😀");

  // Lone surrogate input normalizes to U+FFFD under `js-str=bytes`, while `utf16` passes the
  // host string verbatim (both report the same UTF-8 byteLen=5 and charLen=3 inside Rust!).
  input.value = "a\uD800b";
  input.dispatchEvent({ type: "input", target: input });
  const expectedSurrogateText = expectedStrMode === "utf16" ? "a\uD800b" : "a\uFFFDb";
  assert.equal(p.textContent, expectedSurrogateText);
  assert.equal(span.className, "");

  // Direct Event listener (`custom`) calls preventDefault + target_value
  const customEv = { type: "custom", target: input, defaultPrevented: false };
  customEv.preventDefault = () => {
    customEv.defaultPrevented = true;
  };
  input.dispatchEvent(customEv);
  assert.equal(customEv.defaultPrevented, true);
  // "a\uFFFDb" in UTF-8 is 5 bytes
  assert.equal(article.children[1].textContent, "5");

  // Re-dispatch valid non-ASCII Unicode so the DOM snapshot compares identically across utf16 & bytes
  input.value = "final-€-😀";
  input.dispatchEvent({ type: "input", target: input });
  assert.equal(p.textContent, "final-€-😀");
  assert.equal(span.className, "active");

  snapshot.mainInitialNotes = rustNotes.splice(0).map((note) => ({
    ...note,
    text: note.text.replace(/[\uD800-\uDFFF]/g, "\uFFFD"),
  }));
  snapshot.mainAfter = serializeNode(mainNode);
  dispose();
}

// 2. build_cond: cond + content(&str) + content(()) + content(text)
{
  let dispose;
  let condOut;
  dom.createRoot((d) => {
    dispose = d;
    condOut = compiled.build_cond();
  });
  const host = mountList(condOut);
  const btn = host.children[0];
  const box1 = host.children[1];
  const box2 = host.children[2];
  assert.equal(box1.textContent, "cond-é😀");
  assert.equal(box2.textContent, "fallback-π");
  snapshot.condBefore = serializeNode(host);

  btn.dispatchEvent({ type: "click", target: btn });
  assert.equal(box1.textContent, "");
  assert.equal(box2.textContent, "€");
  snapshot.condAfter = serializeNode(host);
  dispose();
}

// 3. render_push_keyed: &str keys + Display struct keys preserve cached node identity
{
  let r0;
  let r1;
  let str0Nodes;
  let disp0Nodes;
  let str0Snap;
  let disp0Snap;
  dom.createRoot((d) => {
    r0 = compiled.render_push_keyed(0);
    const [strUl0, dispUl0] = r0;
    str0Nodes = strUl0.children.slice();
    disp0Nodes = dispUl0.children.slice();
    str0Snap = serializeNode(strUl0);
    disp0Snap = serializeNode(dispUl0);
    r1 = compiled.render_push_keyed(1);
    d();
  });
  const [strUl1, dispUl1] = r1;
  assert.deepEqual(
    str0Nodes.map((n) => n.textContent),
    ["é😀", "€", "plain"],
  );
  assert.deepEqual(
    strUl1.children.map((n) => n.textContent),
    ["plain", "€", "é😀"],
  );
  assert.equal(strUl1.children[0], str0Nodes[2]);
  assert.equal(strUl1.children[1], str0Nodes[1]);
  assert.equal(strUl1.children[2], str0Nodes[0]);

  assert.equal(dispUl1.children[0], disp0Nodes[1]);
  assert.equal(dispUl1.children[1], disp0Nodes[0]);
  snapshot.pushKeyed = {
    str0: str0Snap,
    str1: serializeNode(strUl1),
    disp0: disp0Snap,
    disp1: serializeNode(dispUl1),
  };
}

// 4. build_selector: Selector<&str> updates only old & new keys
{
  rustNotes.length = 0;
  let dispose;
  let selOut;
  dom.createRoot((d) => {
    dispose = d;
    selOut = compiled.build_selector();
  });
  const host = mountList(selOut);
  const btn = host.children[0];
  const spans = host.children.slice(1);
  assert.deepEqual(
    spans.map((s) => s.getAttribute("data-selected")),
    ["yes", "no", "no"],
  );
  const initialSelNotes = rustNotes.splice(0);

  btn.value = "€";
  btn.dispatchEvent({ type: "click", target: btn });
  assert.deepEqual(
    spans.map((s) => s.getAttribute("data-selected")),
    ["no", "yes", "no"],
  );
  const updateSelNotes = rustNotes.splice(0);
  // Selector fan-out must only re-run the previously selected ("é😀") and newly selected ("€") rows!
  assert.deepEqual(
    updateSelNotes.map((n) => n.text).sort(),
    ["é😀", "€"].sort(),
  );
  snapshot.selector = { initialSelNotes, updateSelNotes, dom: serializeNode(host) };
  dispose();
}

// 5. build_settled: microtask + owner + with_owner + on_settled(&str) + on_cleanup
{
  rustNotes.length = 0;
  cleanupNotes.length = 0;
  let dispose;
  let pEl;
  dom.createRoot((d) => {
    dispose = d;
    pEl = compiled.build_settled();
  });
  assert.equal(pEl.textContent, "pending");
  for (let turn = 0; turn < 8; turn += 1) {
    await Promise.resolve();
  }
  assert.equal(pEl.textContent, "é😀");
  const settledNotes = rustNotes.splice(0);
  assert.equal(settledNotes.length, 2);
  const expectedSettledSurrogate = expectedStrMode === "utf16" ? "x\uD800y" : "x\uFFFDy";
  assert.equal(settledNotes[1].text, expectedSettledSurrogate);
  assert.equal(settledNotes[1].byteLen, 5);
  assert.equal(settledNotes[1].charLen, 3);
  dispose();
  const settledCleanups = cleanupNotes.splice(0);
  assert.deepEqual(settledCleanups, ["microtask-ran", "settled-owner"]);
  snapshot.settled = {
    text: pEl.textContent,
    notes: settledNotes.map((note) => ({
      ...note,
      text: note.text.replace(/[\uD800-\uDFFF]/g, "\uFFFD"),
    })),
    cleanups: settledCleanups,
  };
}

const hasMetrics = typeof rt.keyed_metrics_reset === "function";

// 6. keyed_list_v1 over &[(&str, u32)] with &str keys and live row updates
{
  rt._keyed_states.clear();
  if (hasMetrics) rt.keyed_metrics_reset();
  rustNotes.length = 0;
  let dispose;
  let out;
  dom.createRoot((d) => {
    dispose = d;
    out = compiled.build_keyed_slice();
  });
  const host = mountList(out);
  const btn = host.children[0];
  const [state] = rt._keyed_states;
  const rowsBefore = state.order.map((r) => r.nodes[0]);
  assert.deepEqual(
    rowsBefore.map((n) => [n.getAttribute("data-key"), n.children[0].textContent, n.children[1].textContent]),
    [
      ["é😀", "é😀", "1"],
      ["€", "€", "2"],
      ["ascii", "ascii", "3"],
    ],
  );
  const emojiLi = rowsBefore[0];
  const euroLi = rowsBefore[1];

  btn.dispatchEvent({ type: "click", target: btn });
  const rowsAfter = state.order.map((r) => r.nodes[0]);
  assert.equal(rowsAfter[0], euroLi);
  assert.equal(rowsAfter[2], emojiLi);
  assert.deepEqual(
    rowsAfter.map((n) => [n.getAttribute("data-key"), n.children[0].textContent, n.children[1].textContent]),
    [
      ["€", "€", "20"],
      ["π", "π", "4"],
      ["é😀", "é😀", "10"],
    ],
  );
  rowsAfter[2].dispatchEvent({ type: "click", target: rowsAfter[2] });
  const sliceClickNotes = rustNotes.splice(0);
  assert.deepEqual(sliceClickNotes, [{ tag: "keyed-slice-click", byteLen: 10, charLen: 3, text: "é😀" }]);
  const metrics = hasMetrics ? rt.keyed_metrics_snapshot() : undefined;
  dispose();
  snapshot.keyedSlice = { dom: serializeNode(host), sliceClickNotes, ...(metrics ? { metrics } : {}) };
}

// 7. keyed_list_v1 over &Vec<(&str, &str)> with &str keys and live row updates
{
  rt._keyed_states.clear();
  if (hasMetrics) rt.keyed_metrics_reset();
  rustNotes.length = 0;
  let dispose;
  let out;
  dom.createRoot((d) => {
    dispose = d;
    out = compiled.build_keyed_vec_live();
  });
  const host = mountList(out);
  const btn = host.children[0];
  const [state] = rt._keyed_states;
  const rowsBefore = state.order.map((r) => r.nodes[0]);
  assert.deepEqual(
    rowsBefore.map((n) => [n.getAttribute("data-key"), n.children[0].textContent, n.children[1].textContent]),
    [
      ["é😀", "é😀", "α"],
      ["€", "€", "β"],
    ],
  );
  const emojiLi = rowsBefore[0];
  const euroLi = rowsBefore[1];

  btn.dispatchEvent({ type: "click", target: btn });
  const rowsAfter = state.order.map((r) => r.nodes[0]);
  assert.equal(rowsAfter[0], euroLi);
  assert.equal(rowsAfter[1], emojiLi);
  assert.deepEqual(
    rowsAfter.map((n) => [n.getAttribute("data-key"), n.children[0].textContent, n.children[1].textContent]),
    [
      ["€", "€", "β-updated-€"],
      ["é😀", "é😀", "α-updated-😀"],
      ["γ", "γ", "new-γ"],
    ],
  );
  rowsAfter[1].dispatchEvent({ type: "click", target: rowsAfter[1] });
  const vecClickNotes = rustNotes.splice(0);
  const metrics = hasMetrics ? rt.keyed_metrics_snapshot() : undefined;
  dispose();
  snapshot.keyedVecLive = { dom: serializeNode(host), vecClickNotes, ...(metrics ? { metrics } : {}) };
}

// 8. keyed_list_v1 over owned Vec<(&str, &str)> with &str keys (static row lane)
{
  rt._keyed_states.clear();
  if (hasMetrics) rt.keyed_metrics_reset();
  let dispose;
  let out;
  dom.createRoot((d) => {
    dispose = d;
    out = compiled.build_keyed_vec_static();
  });
  const host = mountList(out);
  const btn = host.children[0];
  const [state] = rt._keyed_states;
  const rowsBefore = state.order.map((r) => r.nodes[0]);
  assert.deepEqual(
    rowsBefore.map((n) => [n.getAttribute("data-key"), n.children[0].textContent, n.children[1].textContent]),
    [
      ["é😀", "é😀", "one-😀"],
      ["€", "€", "two-€"],
    ],
  );
  const emojiLi = rowsBefore[0];
  const euroLi = rowsBefore[1];

  btn.dispatchEvent({ type: "click", target: btn });
  const rowsAfter = state.order.map((r) => r.nodes[0]);
  assert.equal(rowsAfter[0], euroLi);
  assert.equal(rowsAfter[1], emojiLi);
  assert.deepEqual(
    rowsAfter.map((n) => [n.getAttribute("data-key"), n.children[0].textContent, n.children[1].textContent]),
    [
      ["€", "€", "two-€"],
      ["é😀", "é😀", "one-😀"],
      ["ω", "ω", "three-ω"],
    ],
  );
  const metrics = hasMetrics ? rt.keyed_metrics_snapshot() : undefined;
  dispose();
  snapshot.keyedVecStatic = { dom: serializeNode(host), ...(metrics ? { metrics } : {}) };
}

const json = JSON.stringify(snapshot, null, 2);
if (outPath) {
  writeFileSync(outPath, `${json}\n`);
}
console.log(`view-abi-bstr run: pass (${path.basename(programPath)})`);
