// The reactive names this artifact adds on top of the dom-expressions client ABI.
//
// contract/fixtures/abi.json describes UPSTREAM and nothing else: it is the
// export surface of dom-expressions/src/client.js, produced by importing that
// module and reflecting over it (contract/harness/extract-abi.mjs). Re-running
// the extractor can therefore never add a name upstream does not have, so the
// additions the emitter needs are declared here, and both gen-index.mjs and
// check-exports.mjs read this one list.
//
// WHY createSignal
// ----------------
// The emitter emits `import { createSignal } from "<js-dom-module>"` for a
// signal declared in an island. client.js has no such export: it re-exports six
// names from the rxcore seam (effect, memo, untrack, getOwner, createComponent,
// mergeProps) and creating state is not one of them. A signal taken from any
// other copy of solid would sit in a different reactive graph and never notify
// the effects in this one, so it has to come out of this same artifact.
//
// WHY runWithOwner
// ----------------
// The async executor spawns tasks from event handlers, and a tracking scope
// does not survive an await: the owner must be captured before the first
// suspension and re-entered around every resume. `getOwner` is in the ABI's
// rxcore re-exports but its counterpart `runWithOwner` is not — solid-js/web
// keeps it internal while solid-js proper exports it — so, exactly like
// `createSignal`, it has to come out of this artifact's own graph.
//
// Keys are the name to export, values the name to import from "solid-js". The
// two differ where solid's public name differs from the ABI's, the way
// solid-js/web itself binds `effect` to `createRenderEffect` and `memo` to
// `createMemo`. Nothing here re-exports a name solid-js/web already provides:
// all six rxcore re-exports are in the ABI list already, and exporting one
// twice is a build error.

export const reactiveAdditions = {
  createSignal: "createSignal",
  runWithOwner: "runWithOwner"
};
