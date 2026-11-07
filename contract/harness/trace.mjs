// The recording stub runtime.
//
// This module implements every name in fixtures/abi.json as a RECORDER rather
// than a real implementation. A compiled module imported against this stub runs
// to completion without a DOM and leaves behind a normalized, order-sensitive
// trace of exactly which runtime calls it made and with what shape.
//
// WHY A STUB AND NOT jsdom
// ------------------------
// The point is to pin the CALL SEQUENCE the compiler emits, not to test that
// solid works. A real DOM would let genuinely different emitted code produce
// identical observable results (e.g. setAttribute vs a property assignment that
// happen to agree), hiding exactly the divergence we care about. The stub makes
// the emitted ABI usage itself the observable.
//
// NORMALIZATION
// -------------
// Traces must be stable across runs and across identifier renaming, so:
//
//   * every node object is renamed #0, #1, ... in FIRST-APPEARANCE order
//   * every function value becomes fn#0, fn#1, ... likewise
//   * template cloners become tmpl#0, tmpl#1, ... keyed by their html
//   * everything else is JSON, with undefined recorded as the string "undefined"
//
// so the trace records structure and order, never identity or address.
//
// The synthetic tree: template() returns a cloner producing a fake node whose
// firstChild / nextSibling / etc. are themselves synthetic nodes, created
// lazily and memoised, so `el.firstChild.nextSibling` is stable and its
// identity is recorded as a path like "#0.firstChild.nextSibling".

// ------------------------------------------------------------------ registry
let records = [];
let nodeSeq = 0;
let fnSeq = 0;
let tmplSeq = 0;

const nodeNames = new WeakMap();
const fnNames = new WeakMap();

/** Reset all recording state. Call between fixtures. */
export function __reset() {
  records = [];
  nodeSeq = 0;
  fnSeq = 0;
  tmplSeq = 0;
}

/** The recorded trace, as an array of normalized record objects. */
export function __trace() {
  return records;
}

function record(op, args) {
  records.push({ op, ...args });
}

// --------------------------------------------------------------- synthetic DOM
const NODE = Symbol("syntheticNode");

/**
 * A synthetic node. Relations (firstChild, nextSibling, ...) are lazily created
 * and memoised so repeated navigation yields the same object, which is what
 * makes the emitted walk expressions comparable.
 */
function makeNode(label) {
  const relations = new Map();
  const attrs = {};
  const props = {};

  const node = {
    [NODE]: true,
    __label: label,
    // properties compiled code commonly touches directly
    $$events: {},
    style: makeStyle(),
    classList: {},
    attrs,
    props
  };

  const relation = (name) => {
    if (!relations.has(name)) relations.set(name, makeNode(`${label}.${name}`));
    return relations.get(name);
  };

  for (const name of [
    "firstChild",
    "nextSibling",
    "previousSibling",
    "lastChild",
    "parentNode",
    "content",
    "host"
  ]) {
    Object.defineProperty(node, name, { get: () => relation(name), enumerable: false });
  }

  // Methods a runtime might call on a node during a trace.
  node.appendChild = (child) => {
    record("node.appendChild", { node: nodeName(node), child: value(child) });
    return child;
  };
  node.insertBefore = (child, ref) => {
    record("node.insertBefore", {
      node: nodeName(node),
      child: value(child),
      ref: value(ref)
    });
    return child;
  };
  node.removeChild = (child) => {
    record("node.removeChild", { node: nodeName(node), child: value(child) });
    return child;
  };
  node.remove = () => record("node.remove", { node: nodeName(node) });
  node.setAttribute = (name, v) => {
    attrs[name] = v;
    record("node.setAttribute", { node: nodeName(node), name, value: value(v) });
  };
  node.removeAttribute = (name) => {
    delete attrs[name];
    record("node.removeAttribute", { node: nodeName(node), name });
  };
  node.getAttribute = (name) => attrs[name] ?? null;
  node.hasAttribute = (name) => name in attrs;
  node.addEventListener = (name, handler, opts) =>
    record("node.addEventListener", {
      node: nodeName(node),
      name,
      handler: value(handler),
      options: value(opts)
    });
  node.cloneNode = (deep) => {
    const clone = makeNode(`${label}#clone`);
    record("node.cloneNode", { node: nodeName(node), deep: Boolean(deep), result: nodeName(clone) });
    return clone;
  };

  nodeNames.set(node, label ?? `#${nodeSeq++}`);
  return node;
}

function makeStyle() {
  const store = {};
  return {
    setProperty: (k, v) => {
      store[k] = v;
    },
    removeProperty: (k) => {
      delete store[k];
    },
    __store: store
  };
}

const isNode = (v) => typeof v === "object" && v !== null && v[NODE] === true;

function nodeName(node) {
  if (!nodeNames.has(node) || nodeNames.get(node) === undefined) {
    nodeNames.set(node, `#${nodeSeq++}`);
  }
  let name = nodeNames.get(node);
  // Anonymous nodes get a first-appearance number; labelled ones (walk results)
  // keep their structural path, which is far more useful in a diff.
  if (name === undefined) {
    name = `#${nodeSeq++}`;
    nodeNames.set(node, name);
  }
  return name;
}

function fnName(fn) {
  if (!fnNames.has(fn)) fnNames.set(fn, `fn#${fnSeq++}`);
  return fnNames.get(fn);
}

/** Normalize any runtime value into something JSON-stable. */
function value(v) {
  if (v === undefined) return "undefined";
  if (v === null) return null;
  if (isNode(v)) return nodeName(v);
  if (typeof v === "function") return v.__tmpl ? v.__tmpl : fnName(v);
  if (Array.isArray(v)) return v.map(value);
  if (typeof v === "symbol") return `symbol(${String(v.description ?? "")})`;
  if (typeof v === "object") {
    const out = {};
    for (const k of Object.keys(v).sort()) out[k] = value(v[k]);
    return out;
  }
  return v;
}

// --------------------------------------------------------- template + cloning
export function template(html, isImportNode, isSVG, isMathML) {
  const name = `tmpl#${tmplSeq++}`;
  record("template", {
    tmpl: name,
    html,
    // Only recorded when set, mirroring the plugin: the three booleans are
    // appended to the call only when at least one is true.
    ...(isImportNode !== undefined ? { isImportNode: Boolean(isImportNode) } : {}),
    ...(isSVG !== undefined ? { isSVG: Boolean(isSVG) } : {}),
    ...(isMathML !== undefined ? { isMathML: Boolean(isMathML) } : {})
  });

  let root = null;
  const clone = () => {
    if (!root) root = makeNode(name);
    const inst = makeNode(`${name}:root`);
    record("template.clone", { tmpl: name, result: nodeName(inst) });
    return inst;
  };
  clone.__tmpl = name;
  clone.cloneNode = clone;
  return clone;
}

// -------------------------------------------------------------------- inserts
export function insert(parent, accessor, marker, initial) {
  record("insert", {
    parent: value(parent),
    accessor: value(accessor),
    marker: value(marker),
    initial: value(initial)
  });
}

export function insertExpression(parent, value_, current, marker) {
  record("insertExpression", {
    parent: value(parent),
    value: value(value_),
    current: value(current),
    marker: value(marker)
  });
}

// ------------------------------------------------------------------ reactivity
export function effect(fn, init, options) {
  record("effect", { fn: value(fn), init: value(init), options: value(options) });
  // Run it once so the inner attribute/property calls are recorded too -- that
  // inner sequence is a large part of what the contract pins.
  try {
    const r = fn(init);
    record("effect.ran", { fn: value(fn), returned: value(r) });
    return r;
  } catch (err) {
    record("effect.threw", { fn: value(fn), error: String(err && err.message) });
    return undefined;
  }
}

export function memo(fn, equal) {
  record("memo", { fn: value(fn), equal: value(equal) });
  const m = () => fn();
  return m;
}

export function untrack(fn) {
  record("untrack", { fn: value(fn) });
  return fn();
}

export function getOwner() {
  record("getOwner", {});
  return null;
}

export function createComponent(Comp, props) {
  record("createComponent", { comp: value(Comp), props: value(props) });
  try {
    return Comp(props);
  } catch (err) {
    record("createComponent.threw", { error: String(err && err.message) });
    return undefined;
  }
}

export function mergeProps(...sources) {
  record("mergeProps", { sources: sources.map(value) });
  return Object.assign({}, ...sources.map((s) => (typeof s === "function" ? s() : s)));
}

export function dynamicProperty(props, key) {
  record("dynamicProperty", { props: value(props), key });
  return props;
}

// ------------------------------------------------------------------ attributes
export function setAttribute(node, name, v) {
  record("setAttribute", { node: value(node), name, value: value(v) });
}

export function setAttributeNS(node, ns, name, v) {
  record("setAttributeNS", { node: value(node), ns, name, value: value(v) });
}

export function setBoolAttribute(node, name, v) {
  record("setBoolAttribute", { node: value(node), name, value: value(v) });
}

export function setProperty(node, name, v) {
  record("setProperty", { node: value(node), name, value: value(v) });
}

export function setStyleProperty(node, name, v) {
  record("setStyleProperty", { node: value(node), name, value: value(v) });
}

export function className(node, v) {
  record("className", { node: value(node), value: value(v) });
}

export function classList(node, v, prev) {
  record("classList", { node: value(node), value: value(v), prev: value(prev) });
  return v;
}

export function style(node, v, prev) {
  record("style", { node: value(node), value: value(v), prev: value(prev) });
  return v;
}

export function spread(node, props, isSVG, skipChildren) {
  record("spread", {
    node: value(node),
    props: value(props),
    isSVG: Boolean(isSVG),
    skipChildren: Boolean(skipChildren)
  });
}

export function assign(node, props, isSVG, skipChildren, prevProps, skipRef) {
  record("assign", {
    node: value(node),
    props: value(props),
    isSVG: Boolean(isSVG),
    skipChildren: Boolean(skipChildren),
    prevProps: value(prevProps),
    skipRef: Boolean(skipRef)
  });
}

export function use(fn, element, arg) {
  record("use", { fn: value(fn), element: value(element), arg: value(arg) });
}

export function innerHTML(parent, content) {
  record("innerHTML", { parent: value(parent), content: value(content) });
}

// ---------------------------------------------------------------------- events
export function addEventListener(node, name, handler, delegate) {
  record("addEventListener", {
    node: value(node),
    name,
    handler: value(handler),
    delegate: Boolean(delegate)
  });
}

export function delegateEvents(eventNames, doc) {
  record("delegateEvents", { events: [...eventNames].sort(), document: value(doc) });
}

export function clearDelegatedEvents(doc) {
  record("clearDelegatedEvents", { document: value(doc) });
}

// ------------------------------------------------------------------- hydration
let hydrationCounter = 0;

export function getNextElement(tmpl) {
  const node = makeNode(`hk#${hydrationCounter++}`);
  record("getNextElement", { tmpl: value(tmpl), result: nodeName(node) });
  return node;
}

export function getNextMatch(el, nodeName_) {
  record("getNextMatch", { el: value(el), nodeName: nodeName_ });
  return el;
}

export function getNextMarker(start) {
  const end = makeNode(`marker#${hydrationCounter++}`);
  record("getNextMarker", { start: value(start), end: nodeName(end) });
  return [end, []];
}

export function runHydrationEvents() {
  record("runHydrationEvents", {});
}

export function getHydrationKey() {
  const key = `hk${hydrationCounter++}`;
  record("getHydrationKey", { key });
  return key;
}

export function hydrate(code, element, options) {
  record("hydrate", { code: value(code), element: value(element), options: value(options) });
  return code();
}

export function render(code, element, init, options) {
  record("render", {
    code: value(code),
    element: value(element),
    init: value(init),
    options: value(options)
  });
  return code();
}

export function NoHydration(props) {
  record("NoHydration", { props: value(props) });
  return props && props.children;
}

export function Hydration(props) {
  record("Hydration", { props: value(props) });
  return props && props.children;
}

// ------------------------------------------- built-in components (NOT ABI)
//
// These two are NOT among abi.json's 48 client exports, and that is the point.
// The plugin's `builtIns: ["For", "Show"]` option makes it emit
//
//   import { Show as _$Show } from "r-dom";
//
// for any JSX element with a matching name -- so compiled output can import from
// moduleName a name the dom-expressions client runtime does not export. In a
// real app the seam closes because solid-js/web re-exports solid-js's control
// flow alongside the DOM runtime; against a bare stub it does not, and the
// module fails to load before a single record is written.
//
// Added for the corpus (families 11-topcoat-if and 13-topcoat-for), which are
// the first fixtures here to use a builtIn. Adding an export cannot change an
// existing trace: none of run-trace.mjs's RECORD_TARGETS import either name.
//
// They record and return undefined rather than implementing control flow.
// Reading `props` through value() is not inert -- the plugin compiles props to
// getters, so enumerating them evaluates `when`, `children` and `fallback`, and
// the templates in those branches get cloned and recorded. That evaluation IS
// the contract-relevant part; what Show does with the result afterwards is
// solid's business, not the emitter's.
export function Show(props) {
  record("Show", { props: value(props) });
  return undefined;
}

export function For(props) {
  record("For", { props: value(props) });
  return undefined;
}

// -------------------------------------------------------- server-only voidFns
// Present on the client ABI but bound to a no-op there. Recorded so a compiled
// module that reaches for one is visible in the trace rather than crashing.
const voidFn = (name) => (...args) => {
  record(`voidFn:${name}`, { args: args.map(value) });
  return undefined;
};

export const useAssets = voidFn("useAssets");
export const getAssets = voidFn("getAssets");
export const Assets = voidFn("Assets");
export const generateHydrationScript = voidFn("generateHydrationScript");
export const HydrationScript = voidFn("HydrationScript");
export const getRequestEvent = voidFn("getRequestEvent");

export const RequestContext = Symbol("RequestContext");

// ---------------------------------------------------- constants (re-exported)
// Re-exported from the pinned constants module so the stub's surface matches
// abi.json exactly. These are data, not behavior, so they pass through as-is.
export {
  Properties,
  ChildProperties,
  getPropAlias,
  Aliases,
  DOMElements,
  SVGElements,
  SVGNamespace,
  DelegatedEvents
} from "../vendor/upstream/dom-expressions/src/constants.js";
