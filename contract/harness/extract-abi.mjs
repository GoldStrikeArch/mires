// Extract the dom-expressions client runtime ABI by IMPORTING client.js and
// reflecting over the live module namespace.
//
//   node --import ./register-loader.mjs extract-abi.mjs
//
// Writes fixtures/abi.json.
//
// Three things are recorded:
//
//   exports      every export of dom-expressions/src/client.js: name, kind,
//                and arity (Function.length -- note this UNDERCOUNTS, since
//                parameters with defaults and rest params are excluded; the
//                declared parameter list is recorded separately where it
//                matters).
//   rxcore       the six names client.js re-exports straight from the "rxcore"
//                seam (client.js:33-47), plus the two it consumes internally
//                without re-exporting (root, sharedConfig). These are the names
//                the vendored runtime artifact must source from solid, not
//                from dom-expressions.
//   solidWebParity   whether solid-js/web (browser build) exposes each name.
//                This is the reconciliation the runtime/dom artifact depends
//                on: it re-exports from solid-js/web, so any ABI name missing
//                there needs an alias or another source.

import {
  requireUpstream,
  writeJson,
  provenance,
  DOM_SRC,
  SOLID,
  assertPinnedVersion
} from "./paths.mjs";
import { readFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

requireUpstream();

assertPinnedVersion(
  "dom-expressions",
  JSON.parse(readFileSync(path.join(DOM_SRC, "..", "package.json"), "utf8")).version
);
assertPinnedVersion(
  "solid-js",
  JSON.parse(readFileSync(path.join(SOLID, "package.json"), "utf8")).version
);

const clientUrl = pathToFileURL(path.join(DOM_SRC, "client.js")).href;
const client = await import(clientUrl);

// The browser build of solid-js/web. Imported by explicit dist path: under the
// "node" export condition, the bare specifier "solid-js/web" resolves to the
// SSR build, whose surface is deliberately different.
const solidWeb = await import(pathToFileURL(path.join(SOLID, "web", "dist", "web.js")).href);

// Names client.js re-exports directly from "rxcore" (client.js:33-40).
const RXCORE_REEXPORTS = ["effect", "memo", "untrack", "getOwner", "createComponent", "mergeProps"];
// Names client.js imports from "rxcore" for internal use only (client.js:10-18).
const RXCORE_INTERNAL = ["root", "sharedConfig"];

// Names re-exported from ./constants (client.js:20-30).
const CONSTANTS_REEXPORTS = [
  "Properties",
  "ChildProperties",
  "getPropAlias",
  "Aliases",
  "DOMElements",
  "SVGElements",
  "SVGNamespace",
  "DelegatedEvents"
];

// Names client.js binds to a no-op `voidFn` because they only mean something in
// the server build (client.js:41-46). Present in the ABI, inert at runtime.
const VOID_ON_CLIENT = [
  "useAssets",
  "getAssets",
  "Assets",
  "generateHydrationScript",
  "HydrationScript",
  "getRequestEvent"
];

function describe(name, value) {
  const kind =
    typeof value === "function"
      ? "function"
      : typeof value === "symbol"
        ? "symbol"
        : value === null
          ? "null"
          : Array.isArray(value)
            ? "array"
            : value instanceof Set
              ? "set"
              : typeof value;

  const entry = { name, kind };
  if (kind === "function") entry.arity = value.length;

  const origin = RXCORE_REEXPORTS.includes(name)
    ? "rxcore"
    : CONSTANTS_REEXPORTS.includes(name)
      ? "constants"
      : VOID_ON_CLIENT.includes(name)
        ? "voidFn (server-only, inert on client)"
        : "client.js";
  entry.origin = origin;

  entry.inSolidWeb = name in solidWeb;
  return entry;
}

const names = Object.keys(client).sort();
const exportsList = names.map((n) => describe(n, client[n]));

const missingFromSolidWeb = exportsList.filter((e) => !e.inSolidWeb).map((e) => e.name);

// The names the Rust emitter is specified to call. The artifact MUST provide
// these; anything else in the ABI is nice-to-have.
const EMITTER_REQUIRED = [
  "template",
  "insert",
  "effect",
  "setAttribute",
  "setBoolAttribute",
  "className",
  "classList",
  "style",
  "spread",
  "assign",
  "use",
  "delegateEvents",
  "addEventListener",
  "createComponent",
  "memo",
  "dynamicProperty",
  "getNextElement",
  "getNextMatch",
  "getNextMarker",
  "runHydrationEvents",
  "getHydrationKey",
  "hydrate",
  "render",
  "NoHydration"
];

const emitterMissingFromClient = EMITTER_REQUIRED.filter((n) => !names.includes(n));
const emitterMissingFromSolidWeb = EMITTER_REQUIRED.filter((n) => !(n in solidWeb));

writeJson("abi.json", {
  ...provenance("extract-abi.mjs"),
  $source: "dom-expressions/src/client.js, imported with rxcore bound to solid-js",
  $arityNote:
    "arity is Function.length, which stops at the first parameter with a default " +
    "or a rest parameter. e.g. template(html, isImportNode, isSVG, isMathML) reports 4, " +
    "but delegateEvents(eventNames, document = window.document) reports 1. Treat arity " +
    "as a lower bound on the declared parameter count, not a call-signature contract.",
  exportCount: exportsList.length,
  exports: exportsList,
  rxcore: {
    $note:
      "The reactivity seam. client.js imports these from the bare specifier " +
      '"rxcore", which is a build-time alias, not a package. solid-js points it ' +
      "at solid's core; that is what the vendored runtime artifact bundles.",
    reExported: RXCORE_REEXPORTS,
    internalOnly: RXCORE_INTERNAL
  },
  constantsReExports: CONSTANTS_REEXPORTS,
  voidOnClient: VOID_ON_CLIENT,
  solidWebParity: {
    $note:
      "Measured against solid-js/web/dist/web.js (the BROWSER build). " +
      "runtime/dom re-exports from solid-js/web, so a name absent here cannot " +
      "simply be re-exported.",
    solidWebExportCount: Object.keys(solidWeb).length,
    missingFromSolidWeb
  },
  emitterRequired: {
    $note:
      "The subset the Rust emitter is specified to call. These must exist in the " +
      "vendored artifact or the emitter cannot link.",
    names: EMITTER_REQUIRED,
    missingFromClient: emitterMissingFromClient,
    missingFromSolidWeb: emitterMissingFromSolidWeb
  }
});

console.log(`abi.json  ${exportsList.length} exports from client.js`);
console.log(`  solid-js/web (browser) exports: ${Object.keys(solidWeb).length}`);
console.log(
  `  ABI names absent from solid-js/web (${missingFromSolidWeb.length}): ` +
    (missingFromSolidWeb.join(", ") || "none")
);
console.log(
  `  emitter-required absent from client.js: ` + (emitterMissingFromClient.join(", ") || "none")
);
console.log(
  `  emitter-required absent from solid-js/web: ` +
    (emitterMissingFromSolidWeb.join(", ") || "none")
);
