import assert from "node:assert/strict";

import "../runtime/shim.js";

const rt = globalThis.__rt;
rt._keyed.clear();
rt._keyed_states.clear();
rt.keyed_metrics_reset();

// K0 instrumentation describes the legacy eager cache without changing it.
const first = {};
const rebuilt = {};
assert.equal(rt.keyed_row("legacy", "1", first), first);
assert.equal(rt.keyed_row("legacy", "1", rebuilt), first);
assert.deepEqual(rt.keyed_metrics_snapshot(), {
  collectionScans: 0,
  createdRows: 0,
  disposals: 0,
  domMoves: 0,
  effects: 0,
  handlers: 0,
  hydrationFallbacks: 0,
  hydratedRows: 0,
  liveRowRecords: 0,
  owners: 0,
  removedRows: 0,
  rangeReplacements: 0,
  retainedRows: 0,
  rowFactories: 0,
  selectionEffects: 0,
  transactionStates: 0,
});

rt._keyed.clear();
rt.keyed_metrics_reset();

const cleanup = [];
const factory = (item, index, key) => rt.keyed_owner(owner => {
  owner.effect(() => cleanup.push(`effect:${key}`));
  owner.handler(() => cleanup.push(`handler:${key}`));
  owner.onCleanup(() => cleanup.push(`row:${key}`));
  return { createdFrom: item.label, key };
}, item, index, key);

const state = rt.keyed_state();
const initial = rt.keyed_reconcile(
  state,
  [{ id: 1, label: "one" }, { id: 2, label: "two" }],
  item => item.id,
  factory,
  () => 0,
);
assert.deepEqual(initial.map(row => row.key), [1, 2]);
const row1 = state.rows.get(1);
const row2 = state.rows.get(2);
assert.equal(row1.item.value.label, "one");
assert.equal(row2.index.value, 1);
assert.deepEqual(
  rt.keyed_values({ buf: ["outside", "a", "b", "outside"], off: 1, len: 2 }),
  ["a", "b"],
);
const borrowedBuffer = ["outside", "a", "b", "outside"];
assert.deepEqual(
  rt.keyed_references({ buf: borrowedBuffer, off: 1, len: 2 }),
  [{ buf: borrowedBuffer, off: 1 }, { buf: borrowedBuffer, off: 2 }],
);
assert.throws(
  () => rt.keyed_values({ buf: ["a"], off: 1, len: 1 }),
  /invalid keyed-list slice source/,
);
assert.throws(
  () => rt.keyed_references({ buf: ["a"], off: 1, len: 1 }),
  /invalid keyed-list borrowed source/,
);

const reordered = rt.keyed_reconcile(
  state,
  [{ id: 2, label: "two updated" }, { id: 1, label: "one updated" }],
  item => item.id,
  factory,
  values => {
    assert.deepEqual(values.map(row => row.key), [2, 1]);
    return 2;
  },
);
assert.equal(reordered[0], row2.value);
assert.equal(reordered[1], row1.value);
assert.equal(state.rows.get(2), row2);
assert.equal(row2.item.value.label, "two updated");
assert.equal(row2.index.value, 0);
assert.equal(row1.item.value.label, "one updated");
assert.equal(row1.index.value, 1);

let duplicateFactories = 0;
assert.throws(
  () => rt.keyed_reconcile(
    state,
    [{ id: 2 }, { id: 2 }],
    item => item.id,
    () => {
      duplicateFactories += 1;
      return {};
    },
  ),
  /duplicate keyed-list key/,
);
assert.equal(duplicateFactories, 0);
assert.deepEqual(state.order, [row2, row1]);

rt.keyed_reconcile(
  state,
  [{ id: 2, label: "only" }],
  item => item.id,
  factory,
  () => 0,
);
assert.deepEqual(cleanup, ["row:1", "handler:1", "effect:1"]);
assert.equal(state.rows.size, 1);

const beforeFailure = state.rows.get(2);
assert.throws(
  () => rt.keyed_reconcile(
    state,
    [{ id: 2, label: "still live" }, { id: 3, label: "temporary" }, { id: 4 }],
    item => item.id,
    (item, index, key) => rt.keyed_owner(owner => {
      owner.onCleanup(() => cleanup.push(`failed:${key}`));
      if (key === 4) throw new Error("row factory failed");
      return { key };
    }, item, index, key),
  ),
  /row factory failed/,
);
assert.equal(state.rows.size, 1);
assert.equal(state.rows.get(2), beforeFailure);
assert.deepEqual(cleanup.slice(-2), ["failed:4", "failed:3"]);

assert.throws(
  () => rt.keyed_reconcile(state, [{ id: "2" }], item => item.id, factory),
  /key type changed/,
);
assert.equal(state.rows.get(2), beforeFailure);

const other = rt.keyed_state();
rt.keyed_reconcile(other, [{ id: 2, label: "other" }], item => item.id, factory);
assert.notEqual(other.rows.get(2), state.rows.get(2));

rt.keyed_dispose(state);
rt.keyed_dispose(state);
rt.keyed_dispose(other);
assert.equal(state.disposed, true);
assert.equal(other.disposed, true);
assert.equal(rt.keyed_metrics_snapshot().liveRowRecords, 0);
assert.equal(rt.keyed_metrics_snapshot().transactionStates, 0);
assert.throws(
  () => rt.keyed_reconcile(state, [], item => item.id, factory),
  /disposed keyed list/,
);

const metrics = rt.keyed_metrics_snapshot();
assert.equal(metrics.rowFactories, 5);
assert.equal(metrics.createdRows, 3);
assert.equal(metrics.retainedRows, 3);
assert.equal(metrics.domMoves, 2);
assert.equal(metrics.effects, 3);
assert.equal(metrics.handlers, 3);
assert.equal(metrics.owners, 5);
assert.equal(metrics.disposals, 5);

// A commit records inverse mutations before touching its host state. A failure
// runs those inverses in reverse order and leaves the live row transaction intact.
const journalState = rt.keyed_state();
const journalCleanup = [];
const journalFactory = (item, index, key) => rt.keyed_owner(owner => {
  owner.onCleanup(() => journalCleanup.push(key));
  return { key };
}, item, index, key);
let domOrder = [];
const replaceDom = values => {
  domOrder = values.map(value => value.key);
  return domOrder.length;
};
rt.keyed_reconcile(
  journalState,
  [{ id: 1 }, { id: 2 }],
  item => item.id,
  journalFactory,
  replaceDom,
);
const journalRow1 = journalState.rows.get(1);
const journalRow2 = journalState.rows.get(2);
const rollbackOrder = [];
assert.throws(
  () => rt.keyed_reconcile(
    journalState,
    [{ id: 2 }, { id: 3 }, { id: 1 }],
    item => item.id,
    journalFactory,
    (values, _rows, journal) => {
      const before = domOrder;
      journal(() => {
        domOrder = before;
        rollbackOrder.push("dom");
      });
      journal(() => rollbackOrder.push("last"));
      domOrder = values.map(value => value.key);
      throw new Error("DOM commit failed");
    },
  ),
  /DOM commit failed/,
);
assert.deepEqual(rollbackOrder, ["last", "dom"]);
assert.deepEqual(domOrder, [1, 2]);
assert.deepEqual([...journalState.rows.keys()], [1, 2]);
assert.equal(journalState.rows.get(1), journalRow1);
assert.equal(journalState.rows.get(2), journalRow2);
assert.deepEqual(journalCleanup, [3]);

// Reconciliation requested from inside a commit runs after that commit. The
// newest request wins and retained rows keep their identity across both steps.
let reentrant = true;
const reentrantCommit = (values) => {
  domOrder = values.map(value => value.key);
  if (reentrant) {
    reentrant = false;
    rt.keyed_reconcile(
      journalState,
      [{ id: 5 }],
      item => item.id,
      journalFactory,
      reentrantCommit,
    );
    const latest = rt.keyed_reconcile(
      journalState,
      [{ id: 4 }, { id: 2 }],
      item => item.id,
      journalFactory,
      reentrantCommit,
    );
    assert.deepEqual(latest.map(value => value.key), [1, 2]);
  }
  return values.length;
};
rt.keyed_reconcile(
  journalState,
  [{ id: 2 }, { id: 1 }],
  item => item.id,
  journalFactory,
  reentrantCommit,
);
assert.deepEqual(domOrder, [4, 2]);
assert.deepEqual([...journalState.rows.keys()], [4, 2]);
assert.equal(journalState.rows.get(2), journalRow2);
assert.deepEqual(journalCleanup, [3, 1]);
rt.keyed_dispose(journalState);

// Cleanup happens after publication and every removed row is attempted. The
// thrown error explicitly describes a committed, internally consistent state.
const cleanupState = rt.keyed_state();
const cleanupAttempts = [];
const throwingCleanupFactory = (item, index, key) => rt.keyed_owner(owner => {
  owner.onCleanup(() => {
    cleanupAttempts.push(key);
    if (key !== 3) throw new Error(`cleanup ${key}`);
  });
  return { key };
}, item, index, key);
rt.keyed_reconcile(
  cleanupState,
  [{ id: 1 }, { id: 2 }, { id: 3 }],
  item => item.id,
  throwingCleanupFactory,
);
let cleanupFailure;
try {
  rt.keyed_reconcile(
    cleanupState,
    [{ id: 3 }],
    item => item.id,
    throwingCleanupFactory,
  );
} catch (error) {
  cleanupFailure = error;
}
assert.equal(cleanupFailure?.message, "keyed row cleanup failed");
assert.equal(cleanupFailure?.keyedCommitted, true);
assert.equal(cleanupFailure?.errors.length, 2);
assert.deepEqual(cleanupAttempts, [1, 2]);
assert.deepEqual([...cleanupState.rows.keys()], [3]);
rt.keyed_dispose(cleanupState);
assert.deepEqual(cleanupAttempts, [1, 2, 3]);

// Reactive hooks make item/index slots live, batch every retained update with
// publication, and put each row factory under one disposable root.
let reactiveState;
let reactiveBatches = 0;
let reactiveRoots = 0;
let reactiveRootDisposals = 0;
let reactiveSignalWrites = 0;
let disposeReactiveState;
const publishedInBatch = [];
const reactiveHooks = {
  batch: transaction => {
    reactiveBatches += 1;
    const result = transaction();
    publishedInBatch.push([...reactiveState.rows.keys()]);
    return result;
  },
  root: build => {
    reactiveRoots += 1;
    let disposed = false;
    return build(() => {
      if (disposed) return;
      disposed = true;
      reactiveRootDisposals += 1;
    });
  },
  signal: initial => {
    let value = initial;
    return [
      () => value,
      next => {
        reactiveSignalWrites += 1;
        value = next;
      },
    ];
  },
  cleanup: cleanup => {
    disposeReactiveState = cleanup;
  },
};
reactiveState = rt.keyed_state(reactiveHooks);
const reactiveFactory = (item, index, key) => rt.keyed_owner(owner => {
  const handler = () => `${owner.item.label}:${owner.index}`;
  owner.handler();
  return { handler, key };
}, item, index, key, reactiveHooks);
rt.keyed_reconcile(
  reactiveState,
  [{ id: 1, label: "one" }, { id: 2, label: "two" }],
  item => item.id,
  reactiveFactory,
);
const reactiveRow1 = reactiveState.rows.get(1);
const reactiveRow2 = reactiveState.rows.get(2);
const reactiveHandler1 = reactiveRow1.value.handler;
rt.keyed_reconcile(
  reactiveState,
  [{ id: 2, label: "two+" }, { id: 1, label: "one+" }],
  item => item.id,
  reactiveFactory,
);
assert.equal(reactiveState.rows.get(1), reactiveRow1);
assert.equal(reactiveState.rows.get(2), reactiveRow2);
assert.equal(reactiveRow1.value.handler, reactiveHandler1);
assert.equal(reactiveHandler1(), "one+:1");
assert.equal(reactiveRow2.value.handler(), "two+:0");
assert.deepEqual(publishedInBatch, [[1, 2], [2, 1]]);
assert.equal(reactiveBatches, 2);
assert.equal(reactiveRoots, 2);
assert.equal(reactiveSignalWrites, 4);
rt.keyed_reconcile(reactiveState, [{ id: 2, label: "two only" }], item => item.id, reactiveFactory);
assert.equal(reactiveRootDisposals, 1);
disposeReactiveState();
assert.equal(reactiveRootDisposals, 2);
assert.equal(reactiveState.disposed, true);

// Compiler-known selection uses one shared slot accessor and writes only the
// previous and current rows. Reintroduced selected keys initialize correctly.
const selectionState = rt.keyed_state();
let selector;
const selectionFactory = (item, index, key) => rt.keyed_owner(
  () => ({ key, selected: { value: selector.is(key) } }),
  item,
  index,
  key,
);
selector = rt.keyed_selector(selectionState, 1, row => row.value.selected);
rt.keyed_reconcile(
  selectionState,
  [{ id: 1 }, { id: 2 }, { id: 3 }],
  item => item.id,
  selectionFactory,
);
const selectionBefore = rt.keyed_metrics_snapshot().selectionEffects;
selector.select(2);
assert.equal(selectionState.rows.get(1).value.selected.value, false);
assert.equal(selectionState.rows.get(2).value.selected.value, true);
assert.equal(selectionState.rows.get(3).value.selected.value, false);
assert.equal(rt.keyed_metrics_snapshot().selectionEffects - selectionBefore, 2);
selector.select(3);
selector.select(3);
assert.equal(rt.keyed_metrics_snapshot().selectionEffects - selectionBefore, 4);
rt.keyed_reconcile(selectionState, [{ id: 1 }], item => item.id, selectionFactory);
assert.equal(selector.key, 3);
rt.keyed_reconcile(
  selectionState,
  [{ id: 3 }, { id: 1 }],
  item => item.id,
  selectionFactory,
);
assert.equal(selectionState.rows.get(3).value.selected.value, true);
rt.keyed_dispose(selectionState);

// The K2 static DOM carrier keeps one node or a nonempty node array per row.
// This small DOM model makes focus loss observable whenever insertBefore moves
// the active row, and can inject a one-shot mutation failure for rollback.
class FakeNode {
  constructor(document, name, nodeType = 1) {
    this.children = [];
    this.name = name;
    this.nodeType = nodeType;
    this.ownerDocument = document;
    this.parentNode = null;
  }

  contains(target) {
    return this === target || this.children.some(child => child.contains(target));
  }

  get nextSibling() {
    if (this.parentNode === null) return null;
    const siblings = this.parentNode.children;
    const at = siblings.indexOf(this);
    return at < 0 || at + 1 === siblings.length ? null : siblings[at + 1];
  }
}

class FakeParent extends FakeNode {
  constructor(document, atomic = false) {
    super(document, "parent");
    this.blurEvents = 0;
    this.failAfter = null;
    this.mutationCount = 0;
    if (atomic) this.moveBefore = (node, reference) => this.insert(node, reference, true);
  }

  insert(node, reference, preserve) {
    if (reference !== null && reference.parentNode !== this) throw new Error("invalid reference");
    if (node === reference) return;
    if (this.failAfter !== null) {
      if (this.failAfter === 0) {
        this.failAfter = null;
        throw new Error("injected DOM mutation failure");
      }
      this.failAfter -= 1;
    }
    if (node.parentNode !== null && node.parentNode !== this) node.parentNode.removeChild(node);
    if (node.parentNode === this) {
      if (!preserve && node.contains(this.ownerDocument.activeElement)) {
        this.blurEvents += 1;
        this.ownerDocument.activeElement = null;
      }
      this.children.splice(this.children.indexOf(node), 1);
    }
    const at = reference === null ? this.children.length : this.children.indexOf(reference);
    this.children.splice(at, 0, node);
    node.parentNode = this;
    this.mutationCount += 1;
  }

  insertBefore(node, reference) {
    this.insert(node, reference, false);
  }

  removeChild(node) {
    const at = this.children.indexOf(node);
    if (at < 0) throw new Error("node is not a child");
    if (node.contains(this.ownerDocument.activeElement)) {
      this.blurEvents += 1;
      this.ownerDocument.activeElement = null;
    }
    this.children.splice(at, 1);
    node.parentNode = null;
    this.mutationCount += 1;
  }
}

const fakeDocument = {
  activeElement: null,
  createComment: text => {
    const node = new FakeNode(fakeDocument, `end:${text}`, 8);
    node.data = text;
    node.nodeValue = text;
    return node;
  },
};
globalThis.document = fakeDocument;

// The compiler-facing adapter returns detached initial carriers for `insert`,
// then owns later DOM commits after its permanent boundary has a parent.
let viewEffect;
let disposeView;
let viewFactoryCalls = 0;
let viewRootDisposals = 0;
const viewHooks = {
  batch: transaction => transaction(),
  cleanup: cleanup => {
    disposeView = cleanup;
  },
  effect: effect => {
    viewEffect = effect;
    return effect();
  },
  root: build => build(() => {
    viewRootDisposals += 1;
  }),
  signal: initial => {
    let value = initial;
    return [() => value, next => {
      value = next;
    }];
  },
  untrack: run => run(),
};
let viewItems = [{ id: 1, label: "one" }, { id: 2, label: "two" }];
const viewInitial = rt.keyed_dom_view(
  () => viewItems,
  item => item.id,
  item => {
    viewFactoryCalls += 1;
    return [
      new FakeNode(fakeDocument, `view-label:${item.id}`),
      new FakeNode(fakeDocument, `view-input:${item.id}`),
    ];
  },
  viewHooks,
);
assert.deepEqual(viewInitial.map(node => node.name), [
  "view-label:1", "view-input:1", "view-label:2", "view-input:2", "end:",
]);
assert.equal(viewInitial.every(node => node.parentNode === null), true);
const viewParent = new FakeParent(fakeDocument);
for (const node of viewInitial) viewParent.insertBefore(node, null);
viewItems = [{ id: 2, label: "two+" }, { id: 1, label: "one+" }];
viewEffect();
assert.equal(viewFactoryCalls, 2);
assert.deepEqual(viewParent.children.map(node => node.name), [
  "view-label:2", "view-input:2", "view-label:1", "view-input:1", "end:",
]);
viewItems = [{ id: 2, label: "two++" }, { id: 3, label: "three" }];
viewEffect();
assert.equal(viewFactoryCalls, 3);
assert.equal(viewRootDisposals, 1);
disposeView();
assert.deepEqual(viewParent.children, []);
assert.equal(viewRootDisposals, 3);

// Matching SSR rows have already been adopted by their ordinary template
// markers. The keyed adapter borrows the existing closing marker, performs no
// initial DOM mutation, and owns the first post-hydration update.
const hydrationParent = new FakeParent(fakeDocument);
const hydrationNodes = new Map([
  [1, new FakeNode(fakeDocument, "hydrated:1")],
  [2, new FakeNode(fakeDocument, "hydrated:2")],
]);
const hydrationEnd = fakeDocument.createComment("/");
for (const node of [...hydrationNodes.values(), hydrationEnd]) {
  hydrationParent.insertBefore(node, null);
}
const hydrationMutations = hydrationParent.mutationCount;
const hydratedBefore = rt.keyed_metrics_snapshot().hydratedRows;
let hydrationEffect;
let disposeHydration;
let hydrationItems = [{ id: 1 }, { id: 2 }];
const hydrationInitial = rt.keyed_dom_view(
  () => hydrationItems,
  item => item.id,
  item => hydrationNodes.get(item.id) ?? new FakeNode(fakeDocument, `hydrated:${item.id}`),
  {
    ...viewHooks,
    cleanup: cleanup => {
      disposeHydration = cleanup;
    },
    effect: effect => {
      hydrationEffect = effect;
      return effect();
    },
  },
);
const [hydrationState] = rt._keyed_states;
assert.deepEqual(hydrationInitial, [...hydrationNodes.values(), hydrationEnd]);
assert.equal(hydrationState.dom.end, hydrationEnd);
assert.equal(hydrationState.dom.ownsEnd, false);
assert.equal(hydrationParent.mutationCount, hydrationMutations);
assert.equal(rt.keyed_metrics_snapshot().hydratedRows - hydratedBefore, 2);

hydrationItems = [{ id: 2 }, { id: 3 }];
hydrationEffect();
assert.deepEqual(hydrationParent.children.map(node => node.name), [
  "hydrated:2", "hydrated:3", "end:/",
]);
disposeHydration();
assert.deepEqual(hydrationParent.children, [hydrationEnd]);
assert.equal(hydrationState.disposed, true);

const domParent = new FakeParent(fakeDocument);
const domState = rt.keyed_dom_state(domParent);
const domCleanup = [];
const domFactory = (owner, item) => {
  owner.onCleanup(() => domCleanup.push(item.id));
  const label = new FakeNode(fakeDocument, `label:${item.id}`);
  const input = new FakeNode(fakeDocument, `input:${item.id}`);
  input.selectionStart = item.id;
  input.value = item.label;
  return [label, input];
};
const readDomOrder = () => domParent.children.map(node => node.name);

// With no live focus inside the list, use the DOM runtime's two-ended
// reconciliation shape. A distant retained-row swap is two moves, not one move
// for every row between the endpoints.
const swapParent = new FakeParent(fakeDocument);
const swapState = rt.keyed_dom_state(swapParent);
const swapItems = [1, 2, 3, 4, 5].map(id => ({ id }));
rt.keyed_dom_reconcile(
  swapState,
  swapItems,
  item => item.id,
  (_owner, item) => new FakeNode(fakeDocument, `swap:${item.id}`),
);
const swapRows = new Map(swapState.rows);
const movesBeforeSwap = rt.keyed_metrics_snapshot().domMoves;
rt.keyed_dom_reconcile(
  swapState,
  [swapItems[0], swapItems[4], swapItems[2], swapItems[3], swapItems[1]],
  item => item.id,
  (_owner, item) => new FakeNode(fakeDocument, `swap:${item.id}`),
);
assert.equal(rt.keyed_metrics_snapshot().domMoves - movesBeforeSwap, 2);
assert.deepEqual(swapParent.children.map(node => node.name), [
  "swap:1", "swap:5", "swap:3", "swap:4", "swap:2", "end:",
]);
for (const [key, row] of swapRows) assert.equal(swapState.rows.get(key), row);
rt.keyed_dom_dispose(swapState);

rt.keyed_dom_reconcile(
  domState,
  [{ id: 1, label: "one" }, { id: 2, label: "two" }, { id: 3, label: "three" }],
  item => item.id,
  domFactory,
);
const domRow1 = domState.rows.get(1);
const domRow2 = domState.rows.get(2);
const domRow3 = domState.rows.get(3);
const activeInput = domRow2.nodes[1];
activeInput.selectionStart = 7;
fakeDocument.activeElement = activeInput;
assert.deepEqual(readDomOrder(), [
  "label:1", "input:1", "label:2", "input:2", "label:3", "input:3", "end:",
]);

rt.keyed_dom_reconcile(
  domState,
  [{ id: 3, label: "three+" }, { id: 1, label: "one+" }, { id: 2, label: "two+" }],
  item => item.id,
  domFactory,
);
assert.equal(domState.rows.get(1), domRow1);
assert.equal(domState.rows.get(2), domRow2);
assert.equal(domState.rows.get(3), domRow3);
assert.equal(fakeDocument.activeElement, activeInput);
assert.equal(activeInput.selectionStart, 7);
assert.equal(domParent.blurEvents, 0);
assert.equal(domRow2.item.value.label, "two+");
assert.equal(domRow2.index.value, 2);
assert.deepEqual(readDomOrder(), [
  "label:3", "input:3", "label:1", "input:1", "label:2", "input:2", "end:",
]);

const beforeDomFailure = readDomOrder();
const beforeDomItems = [domRow3.item.value, domRow1.item.value, domRow2.item.value];
domParent.failAfter = 1;
assert.throws(
  () => rt.keyed_dom_reconcile(
    domState,
    [{ id: 1, label: "failed-1" }, { id: 2, label: "failed-2" }, { id: 3, label: "failed-3" }],
    item => item.id,
    domFactory,
  ),
  /injected DOM mutation failure/,
);
assert.deepEqual(readDomOrder(), beforeDomFailure);
assert.deepEqual([...domState.rows.keys()], [3, 1, 2]);
assert.deepEqual(
  [domRow3.item.value, domRow1.item.value, domRow2.item.value],
  beforeDomItems,
  "row slots roll back with DOM order",
);
assert.equal(fakeDocument.activeElement, activeInput);
assert.equal(domParent.blurEvents, 0);

rt.keyed_dom_reconcile(
  domState,
  [{ id: 2, label: "two only" }, { id: 4, label: "four" }],
  item => item.id,
  domFactory,
);
assert.equal(domState.rows.get(2), domRow2);
assert.equal(domRow1.nodes.every(node => node.parentNode === null), true);
assert.equal(domRow3.nodes.every(node => node.parentNode === null), true);
assert.deepEqual(domCleanup, [3, 1]);
assert.deepEqual(readDomOrder(), ["label:2", "input:2", "label:4", "input:4", "end:"]);

rt.keyed_dom_dispose(domState);
assert.deepEqual(domParent.children, []);
assert.deepEqual(domCleanup, [3, 1, 2, 4]);

// A deterministic model catches transaction/order/identity interactions that
// individual examples tend to miss. The seed is part of the failure output.
const seed = 0x5eed1234;
let randomState = seed;
const random = limit => {
  randomState = (Math.imul(randomState, 1664525) + 1013904223) >>> 0;
  return randomState % limit;
};
const oracleState = rt.keyed_state();
const disposed = new Set();
const created = new Set();
let model = [];
let nextId = 1;
for (let step = 0; step < 300; ++step) {
  const operation = random(6);
  if (operation === 0 || model.length === 0) {
    model.splice(random(model.length + 1), 0, { id: nextId++, label: `new-${step}` });
  } else if (operation === 1) {
    const at = random(model.length);
    model[at] = { ...model[at], label: `updated-${step}` };
  } else if (operation === 2 && model.length > 1) {
    const from = random(model.length);
    const [item] = model.splice(from, 1);
    model.splice(random(model.length + 1), 0, item);
  } else if (operation === 3) {
    model.splice(random(model.length), 1);
  } else if (operation === 4 && step % 31 === 0) {
    model = [];
  } else if (operation === 5 && model.length > 1) {
    const left = random(model.length);
    const right = random(model.length);
    [model[left], model[right]] = [model[right], model[left]];
  }

  const previous = new Map(oracleState.rows);
  rt.keyed_reconcile(
    oracleState,
    model,
    item => item.id,
    (item, index, key) => rt.keyed_owner(owner => {
      const value = { key };
      created.add(value);
      owner.onCleanup(() => disposed.add(value));
      return value;
    }, item, index, key),
    () => 0,
  );
  assert.deepEqual([...oracleState.rows.keys()], model.map(item => item.id), `seed ${seed}, step ${step}`);
  model.forEach((item, index) => {
    const row = oracleState.rows.get(item.id);
    assert.equal(row.item.value, item, `seed ${seed}, item slot at step ${step}`);
    assert.equal(row.index.value, index, `seed ${seed}, index slot at step ${step}`);
    if (previous.has(item.id)) {
      assert.equal(row, previous.get(item.id), `seed ${seed}, retained identity at step ${step}`);
    }
  });
  assert.equal(created.size - disposed.size, model.length, `seed ${seed}, live rows at step ${step}`);
}
rt.keyed_dispose(oracleState);
assert.equal(disposed.size, created.size, `seed ${seed}, final disposal`);

// JavaScript can throw any value. No thrown value doubles as an empty error slot.
const throwsValue = (run, expected) => {
  let caught = false;
  try { run(); }
  catch (actual) {
    caught = true;
    assert.equal(actual, expected);
  }
  assert.ok(caught, "the operation must propagate its thrown value");
};
for (const failure of [null, undefined, false, 0, "", Symbol("failure"), Object.freeze(new Error("frozen"))]) {
  for (const phase of ["key", "factory", "commit"]) {
    const state = rt.keyed_state();
    const factory = (item, index, key) => rt.keyed_owner(() => ({ key }), item, index, key);
    rt.keyed_reconcile(state, [1], key => key, factory);
    const original = state.rows.get(1);
    throwsValue(() => rt.keyed_reconcile(state, [1, 2], key => {
      if (phase === "key" && key === 2) throw failure;
      return key;
    }, (item, index, key) => {
      if (phase === "factory") throw failure;
      return factory(item, index, key);
    }, () => { if (phase === "commit") throw failure; return 0; }), failure);
    assert.deepEqual([...state.rows.keys()], [1]);
    assert.equal(state.rows.get(1), original);
    rt.keyed_dispose(state);
  }
  const attempts = [];
  const factory = (item, index, key) => rt.keyed_owner(owner => {
    owner.onCleanup(() => attempts.push(key));
    owner.onCleanup(() => { throw failure; });
    return {};
  }, item, index, key);
  const row = factory(1, 0, 1);
  throwsValue(() => row.dispose(), failure);
  row.dispose();
  assert.deepEqual(attempts, [1]);
  const state = rt.keyed_state();
  rt.keyed_reconcile(state, [2, 3], key => key, factory);
  throwsValue(() => rt.keyed_dispose(state), failure);
  assert.deepEqual(attempts, [1, 2, 3]);
  assert.equal(state.rows.size, 0);
  assert.ok(!rt._keyed_states.has(state));
  const parent = new FakeParent(fakeDocument);
  const dom = rt.keyed_dom_state(parent);
  rt.keyed_dom_reconcile(dom, [4, 5], key => key, owner => {
    owner.onCleanup(() => { attempts.push(owner.key); throw failure; });
    return new FakeNode(fakeDocument, "row");
  });
  throwsValue(() => rt.keyed_dom_dispose(dom), failure);
  assert.deepEqual(attempts, [1, 2, 3, 4, 5]);
  assert.deepEqual(parent.children, []);
}

// A list owner can be torn down by a row factory, commit, or removed-row cleanup.
// Finish the current transaction, then dispose its published rows before returning.
for (const phase of ["key", "factory", "commit", "cleanup"]) {
  const state = rt.keyed_state();
  const cleaned = [];
  const factory = (item, index, key) => rt.keyed_owner(owner => {
    owner.onCleanup(() => {
      cleaned.push(key);
      if (phase === "cleanup") rt.keyed_dispose(state);
    });
    if (key === 2 && phase === "factory") rt.keyed_dispose(state);
    return { key };
  }, item, index, key);
  rt.keyed_reconcile(state, [1], key => key, factory);
  rt.keyed_reconcile(state, [2], key => {
    if (phase === "key") rt.keyed_dispose(state);
    return key;
  }, factory, () => {
    if (phase === "commit") {
      rt.keyed_reconcile(state, [99], key => key, factory);
      rt.keyed_dispose(state);
    }
    return 0;
  });
  assert.ok(state.disposed, phase);
  assert.equal(state.rows.size, 0, phase);
  assert.deepEqual(state.order, [], phase);
  assert.deepEqual(cleaned, [1, 2], phase);
  assert.ok(!rt._keyed_states.has(state), phase);
}

for (const phase of ["key", "factory", "commit", "cleanup"]) {
  const before = rt.keyed_metrics_snapshot().liveRowRecords;
  const parent = new FakeParent(fakeDocument);
  const cleaned = [];
  let teardown = false;
  const state = rt.keyed_dom_state(parent, undefined, {
    batch: publish => {
      publish();
      if (teardown && phase === "commit") rt.keyed_dom_dispose(state);
    },
  });
  const factory = (owner, key) => {
    owner.onCleanup(() => {
      cleaned.push(key);
      if (teardown && phase === "cleanup") rt.keyed_dom_dispose(state);
    });
    if (teardown && phase === "factory") rt.keyed_dom_dispose(state);
    return new FakeNode(fakeDocument, `row:${key}`);
  };
  rt.keyed_dom_reconcile(state, [1], key => key, factory);
  teardown = true;
  rt.keyed_dom_reconcile(state, [2], key => {
    if (phase === "key") rt.keyed_dom_dispose(state);
    return key;
  }, factory);
  assert.deepEqual(cleaned, [1, 2], phase);
  assert.deepEqual(parent.children, [], phase);
  assert.equal(state.rows.size, 0, phase);
  assert.equal(rt.keyed_metrics_snapshot().liveRowRecords, before, phase);
}

// Failure and teardown can coincide. Roll back first, then dispose every owner,
// preserving the original exception even if rollback diagnostics cannot attach.
const frozenFailure = Object.freeze(new Error("original commit failure"));
const failingState = rt.keyed_state();
const failureCleanups = [];
const failingFactory = (item, index, key) => rt.keyed_owner(owner => {
  owner.onCleanup(() => failureCleanups.push(key));
  return {};
}, item, index, key);
rt.keyed_reconcile(failingState, [1], key => key, failingFactory);
throwsValue(() => rt.keyed_reconcile(failingState, [2], key => key, failingFactory,
  (_values, _rows, journal) => {
    journal(() => { throw new Error("rollback failed"); });
    rt.keyed_dispose(failingState);
    throw frozenFailure;
  }), frozenFailure);
assert.deepEqual(failureCleanups, [2, 1]);
assert.equal(failingState.rows.size, 0);
assert.ok(failingState.disposed);

// Materializing a source belongs to the transaction too. Reentrant sources
// queue the newest request and snapshot it immediately, without nested commits.
for (const nestedSource of [false, true]) {
  const state = rt.keyed_state();
  const trace = [];
  const create = (item, index, key) => rt.keyed_owner(() => item, item, index, key);
  const commit = values => { trace.push(values.slice()); return 0; };
  const latest = [3];
  const source = {
    *[Symbol.iterator]() {
      rt.keyed_reconcile(state, nestedSource ? {
        *[Symbol.iterator]() {
          rt.keyed_reconcile(state, latest, key => key, create, commit);
          latest[0] = 4;
          yield 2;
        },
      } : [2], key => key, create, commit);
      assert.deepEqual(trace, [], 'source iteration must not run a nested commit');
      yield 1;
    },
  };
  const result = rt.keyed_reconcile(state, source, key => key, create, commit);
  assert.deepEqual(trace, [[1], [nestedSource ? 3 : 2]]);
  assert.deepEqual(result, [nestedSource ? 3 : 2]);
  assert.equal(state.running, false);
  rt.keyed_dispose(state);
}

for (const failure of [undefined, null, false, Object.freeze({ source: 'failed' })]) {
  const state = rt.keyed_state();
  const trace = [];
  const create = (item, index, key) => rt.keyed_owner(() => item, item, index, key);
  const commit = values => { trace.push(values.slice()); return 0; };
  rt.keyed_reconcile(state, [0], key => key, create, commit);
  throwsValue(() => rt.keyed_reconcile(state, {
    *[Symbol.iterator]() {
      rt.keyed_reconcile(state, [2], key => key, create, commit);
      throw failure;
    },
  }, key => key, create, commit), failure);
  assert.deepEqual(trace, [[0]], 'a failed source must not publish queued work');
  assert.deepEqual([...state.rows.keys()], [0]);
  assert.equal(state.pending, null);
  assert.equal(state.running, false);
  rt.keyed_dispose(state);
}

// A caller can catch a failed queued source. Restore an earlier valid request
// unless source iteration already queued a newer valid request of its own.
for (const newer of [false, true]) {
  const state = rt.keyed_state();
  const trace = [];
  const create = (item, index, key) => rt.keyed_owner(() => item, item, index, key);
  const commit = values => { trace.push(values.slice()); return 0; };
  const failure = {};
  rt.keyed_reconcile(state, {
    *[Symbol.iterator]() {
      rt.keyed_reconcile(state, [2], key => key, create, commit);
      throwsValue(() => rt.keyed_reconcile(state, {
        *[Symbol.iterator]() {
          if (newer) rt.keyed_reconcile(state, [3], key => key, create, commit);
          throw failure;
        },
      }, key => key, create, commit), failure);
      yield 1;
    },
  }, key => key, create, commit);
  assert.deepEqual(trace, [[1], [newer ? 3 : 2]]);
  rt.keyed_dispose(state);
}

const materializedState = rt.keyed_state();
assert.throws(() => rt.keyed_reconcile(materializedState, {
  *[Symbol.iterator]() {
    rt.keyed_dispose(materializedState);
    yield 1;
  },
}, key => key, failingFactory), /disposed keyed list/);
assert.equal(materializedState.rows.size, 0);

// Carrier copying is part of row construction. A failing slice lookup, call,
// or element read must dispose the root even before node validation begins.
for (const failure of [null, undefined, false, 0, "", Symbol("copy"), Object.freeze(new Error("copy"))]) {
  for (const phase of ["lookup", "call", "element"]) {
    for (const cleanupThrows of [false, true]) {
      const nodes = [];
      if (phase === "lookup") {
        Object.defineProperty(nodes, "slice", { get() { throw failure; } });
      } else if (phase === "call") {
        nodes.slice = () => { throw failure; };
      } else {
        Object.defineProperty(nodes, 0, { get() { throw failure; } });
      }
      const trace = [];
      let roots = 0;
      const before = rt.keyed_metrics_snapshot();
      throwsValue(() => rt.keyed_dom_owner(owner => {
        owner.onCleanup(() => trace.push("row"));
        owner.onCleanup(() => {
          trace.push("last");
          if (cleanupThrows) throw new Error("row cleanup");
        });
        return nodes;
      }, 1, 0, 1, {
        root: build => {
          roots += 1;
          return build(() => {
            roots -= 1;
            trace.push("root");
            if (cleanupThrows) throw new Error("root cleanup");
          });
        },
      }), failure);
      assert.equal(roots, 0, `${phase}: a failed carrier must dispose its root`);
      assert.deepEqual(trace, ["last", "row", "root"]);
      const after = rt.keyed_metrics_snapshot();
      assert.equal(after.owners - before.owners, 1);
      assert.equal(after.disposals - before.disposals, 1);
    }
  }
}

console.log("keyed transaction tests: pass");
