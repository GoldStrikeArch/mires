// The "rxcore" seam, filled with solid-js 1.9.14's real reactive core.
//
// dom-expressions/src/client.js imports these eight names from the bare
// specifier "rxcore" (client.js:10-18). "rxcore" is not a package -- it is a
// build-time alias every host framework points at its own reactivity. Upstream
// aliases it via babel-plugin-transform-rename-import; solid-js points it at
// solid's core; upstream's own jest tests point it at an s-js shim
// (packages/dom-expressions/test/core.js in the pinned git tree).
//
// We point it at solid-js deliberately, because solid-js IS the implementation
// the vendored runtime artifact (runtime/dom/) bundles. Using the real core --
// rather than a stub -- means the arities recorded in fixtures/abi.json for the
// six re-exported rxcore names are the true ones, not stub artifacts.
//
// We import solid-js/dist/solid.js by explicit path rather than the "solid-js"
// bare specifier on purpose: under node the package's "exports" map resolves
// "solid-js" to dist/server.js (the SSR build), whose surface differs from the
// client build. The DOM contract is about the client build.

import {
  createRoot,
  createRenderEffect,
  createMemo,
  getOwner as _getOwner,
  createComponent as _createComponent,
  sharedConfig as _sharedConfig,
  untrack as _untrack,
  mergeProps as _mergeProps
} from "solid-js/dist/solid.js";

export const root = createRoot;
export const effect = createRenderEffect;
export const memo = createMemo;
export const getOwner = _getOwner;
export const createComponent = _createComponent;
export const sharedConfig = _sharedConfig;
export const untrack = _untrack;
export const mergeProps = _mergeProps;
