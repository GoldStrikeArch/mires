// A vendored, trace-recording FAKE of a chart library.
//
// This is the `#[js_extern]` counterpart of `harness/trace.mjs`: it implements a
// plausible npm chart library as a RECORDER rather than as a real
// implementation, so a compiled Rust module that declares the library through
// `#[js_extern]` and drives it leaves behind a normalized, order-sensitive trace
// of exactly which JS-level operations it performed.
//
// WHY A FAKE AND NOT THE REAL LIBRARY
// -----------------------------------
// The observable we want is the CALL SHAPE the FFI descriptor lowered to, not
// whether a chart renders. Against a real library, two genuinely different
// lowerings agree on the picture: `new Chart(el, cfg)` and a hypothetical
// `Chart(el, cfg)` factory both produce a chart, `chart.resize(800)` and
// `chart.resize(800, undefined)` both resize, and a wrapper that turns
// `undefined` into `None` looks identical to one that only handles `null` until
// the day the library returns the other one. Recording makes the distinction
// that the descriptor is ABOUT into the thing being measured.
//
// It is also why this file is checked in rather than fetched: there is no
// upstream to pin. The surface below is modelled on Chart.js's actual usage
// (`Chart.register(...)`, `new Chart(canvas, config)`, `chart.update()`,
// `chart.data.datasets[0]`) because the dashboard is what this has to carry, but
// nothing here is derived from Chart.js and nothing should be read as a claim
// about it.
//
// NORMALIZATION
// -------------
// Same discipline as `trace.mjs`, for the same reason -- a trace has to be
// stable across runs and across identifier renaming:
//
//   * instances are labelled chart#0, chart#1, ... in FIRST-APPEARANCE order
//   * a property view is labelled by its PATH from its instance, so
//     `chart.data.datasets` records as `chart#0.data.datasets` and a walk is
//     legible in a diff instead of being three anonymous numbers
//   * functions become fn#0, fn#1, ...
//   * anything with a string `nodeName` becomes `node(<tag>)`, so a real canvas
//     element and this file's stand-in for one record identically
//   * everything else is JSON with object keys sorted, and `undefined` recorded
//     as the string "undefined" so it is distinguishable from `null` -- which is
//     the whole point of the nullable vectors
//
// Labels are computed AT RECORD TIME from a role, never stored at construction
// time, so `__reset()` between a setup phase and a measured phase renumbers
// cleanly and a vector's trace contains only the operation it is about.
//
// SYMBOL KEYS ARE NEVER RECORDED. The indexed collection is a Proxy, and a
// Proxy sees every symbol-keyed probe anything makes -- `Symbol.toPrimitive`,
// node's inspector, an `await` looking for `then`. A trace that changes because
// something logged the object would be worthless, so the get trap passes symbols
// straight through without a record.

// ------------------------------------------------------------------ registry
const ROLE = Symbol("chartRole");

const VERSION = "0.0.0-fake";

let records = [];
let instanceSeq = 0;
let fnSeq = 0;
let instanceNames = new Map();
let fnNames = new WeakMap();
let registered = [];

/** Reset all recording state. Call between vectors. */
export function __reset() {
  records = [];
  instanceSeq = 0;
  fnSeq = 0;
  instanceNames = new Map();
  fnNames = new WeakMap();
  registered = [];
}

/** The recorded trace, as an array of normalized record objects. */
export function __trace() {
  return records;
}

/** Normalize an arbitrary value the way a record's fields are normalized. */
export function __value(v) {
  return value(v);
}

/** Plugins handed to `Chart.register`, in call order. Not part of any trace. */
export function __registered() {
  return registered;
}

function record(op, args) {
  records.push({ op, ...args });
}

// --------------------------------------------------------------- normalization

/**
 * The display name of a recorded object. Instances get a first-appearance
 * number; views get their path, which is far more useful in a diff than a
 * number would be.
 */
function label(obj) {
  const role = obj === null || obj === undefined ? undefined : obj[ROLE];
  if (role === undefined) return null;
  if (role.kind === "instance") {
    if (!instanceNames.has(obj)) instanceNames.set(obj, `chart#${instanceSeq++}`);
    return instanceNames.get(obj);
  }
  return `${label(role.parent)}.${role.prop}`;
}

function fnName(fn) {
  if (!fnNames.has(fn)) fnNames.set(fn, `fn#${fnSeq++}`);
  return fnNames.get(fn);
}

/** Normalize any value into something JSON-stable and identity-free. */
function value(v) {
  if (v === undefined) return "undefined";
  if (v === null) return null;
  if (typeof v === "function") return fnName(v);
  if (typeof v === "symbol") return `symbol(${String(v.description ?? "")})`;
  if (typeof v === "object") {
    const named = label(v);
    if (named !== null) return named;
    if (typeof v.nodeName === "string") return `node(${v.nodeName.toLowerCase()})`;
    if (Array.isArray(v)) return v.map(value);
    const out = {};
    for (const k of Object.keys(v).sort()) out[k] = value(v[k]);
    return out;
  }
  return v;
}

const args = (list) => [...list].map(value);

// ------------------------------------------------------------------ the views
//
// A view is a plain object with defined getters, NOT a Proxy, everywhere the
// property set is known. A Proxy would also trap the properties nobody asked
// about, and a recorder that reacts to being inspected is a recorder that
// records the observer.

function makeView(parent, prop, getters) {
  const view = { [ROLE]: { kind: "view", parent, prop } };
  for (const [name, get] of Object.entries(getters)) {
    Object.defineProperty(view, name, {
      get() {
        const v = get();
        record("get", { target: label(view), prop: name, value: value(v) });
        return v;
      },
      enumerable: true,
      configurable: true
    });
  }
  return view;
}

const INDEX = /^(?:0|[1-9]\d*)$/;

/**
 * An indexed collection. This one IS a Proxy, because the set of valid indices
 * is not knowable up front -- which is exactly the thing the `index` call shape
 * exists for. Out of range reads return `undefined` and are still recorded: a
 * descriptor that wraps a fallible read has to say what it does with
 * `undefined`, and it is NOT the same answer as `null` (see `getPoint`).
 */
function makeCollection(parent, prop, read) {
  const base = { [ROLE]: { kind: "view", parent, prop } };
  return new Proxy(base, {
    get(target, key) {
      if (typeof key === "symbol") return Reflect.get(target, key);
      if (key === "length") {
        const n = read().length;
        record("get", { target: label(target), prop: "length", value: n });
        return n;
      }
      if (INDEX.test(key)) {
        const index = Number(key);
        const item = read()[index];
        record("index", { target: label(target), index, value: value(item) });
        return item;
      }
      return Reflect.get(target, key);
    }
  });
}

// -------------------------------------------------------------- the instance

function makeInstance(config) {
  const state = {
    data: config === null || config === undefined ? undefined : config.data,
    title: config === null || config === undefined ? null : (config.title ?? null)
  };

  const instance = { [ROLE]: { kind: "instance" } };

  const dataset = () => (state.data && state.data.datasets) || [];
  const points = () => (state.data && state.data.points) || [];

  const dataView = makeView(instance, "data", {
    labels: () => (state.data && state.data.labels) || [],
    datasets: () => datasets
  });
  const datasets = makeCollection(dataView, "datasets", dataset);

  // send: one argument, no return value.
  instance.update = function update(data) {
    record("send", {
      target: label(instance),
      method: "update",
      argc: arguments.length,
      args: args(arguments),
      returned: "undefined"
    });
    if (arguments.length > 0) state.data = data;
  };

  // send: an OPTIONAL trailing argument, so a lowering that pads the call with
  // an explicit `undefined` is visible in `argc` rather than invisible because
  // the library ignores it. Real libraries branch on `arguments.length`.
  instance.resize = function resize(width, height) {
    record("send", {
      target: label(instance),
      method: "resize",
      argc: arguments.length,
      args: args(arguments),
      returned: "undefined"
    });
  };

  // send: zero arguments.
  instance.destroy = function destroy() {
    record("send", {
      target: label(instance),
      method: "destroy",
      argc: arguments.length,
      args: args(arguments),
      returned: "undefined"
    });
  };

  // send with a NULLABLE return: an object in range, `null` out of range. Note
  // `null` and not `undefined`, deliberately -- the collection above returns
  // `undefined` for the same mistake, so the two sit side by side and a
  // descriptor cannot conflate them by accident.
  instance.getPoint = function getPoint(index) {
    const all = points();
    const point = Number.isInteger(index) && index >= 0 && index < all.length ? all[index] : null;
    record("send", {
      target: label(instance),
      method: "getPoint",
      argc: arguments.length,
      args: args(arguments),
      returned: value(point)
    });
    return point;
  };

  // get: a plain readable property whose value is a further view.
  Object.defineProperty(instance, "data", {
    get() {
      record("get", { target: label(instance), prop: "data", value: value(dataView) });
      return dataView;
    },
    enumerable: true,
    configurable: true
  });

  // get + set on the same property.
  Object.defineProperty(instance, "title", {
    get() {
      record("get", { target: label(instance), prop: "title", value: value(state.title) });
      return state.title;
    },
    set(v) {
      record("set", { target: label(instance), prop: "title", value: value(v) });
      state.title = v;
    },
    enumerable: true,
    configurable: true
  });

  return instance;
}

// ----------------------------------------------------------- the two bindings
//
// The module exports the constructor TWICE, as `default` and as the named
// `Chart`, which is what a library shipping both an ESM default and a named
// export looks like from the importer's side. They are distinct function
// objects rather than one object exported twice, on purpose: a descriptor has
// to say which binding it imports, and if both bindings were the same object
// that choice would be unobservable and the trace would agree with either
// answer. Every record made through a binding carries `via`.

function makeBinding(via) {
  const Chart = function Chart(target, config) {
    const recorded = args(arguments);
    if (new.target === undefined) {
      record("new.withoutNew", { ctor: "Chart", via, argc: arguments.length, args: recorded });
      throw new TypeError("Chart is a constructor and cannot be invoked without 'new'");
    }
    const instance = makeInstance(config);
    record("new", {
      ctor: "Chart",
      via,
      argc: arguments.length,
      args: recorded,
      result: label(instance)
    });
    return instance;
  };

  // A static method: the `call` shape reached through a scope path rather than
  // through a bare module export.
  Chart.register = function register(...plugins) {
    record("call", {
      target: "Chart",
      via,
      method: "register",
      argc: plugins.length,
      args: plugins.map(value),
      returned: "undefined"
    });
    registered.push(...plugins);
  };

  // A static property: the `get` shape at the same scope.
  Object.defineProperty(Chart, "version", {
    get() {
      record("get", { target: "Chart", via, prop: "version", value: VERSION });
      return VERSION;
    },
    enumerable: false,
    configurable: true
  });

  return Chart;
}

export const Chart = makeBinding("named");

export default makeBinding("default");
