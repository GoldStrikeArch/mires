// Extract the constant tables from dom-expressions/src/constants.js by
// IMPORTING it and reading the live Sets/objects -- not by scraping the source.
//
//   node --import ./register-loader.mjs extract-constants.mjs
//
// Writes fixtures/delegated-events.json and fixtures/properties.json.
//
// The DelegatedEvents list is additionally asserted against the 22 names the
// contract was written around, so a silent upstream addition/removal fails the
// run rather than quietly rewriting the fixture.

import { requireUpstream, writeJson, provenance, DOM_SRC } from "./paths.mjs";
import path from "node:path";
import { pathToFileURL } from "node:url";

requireUpstream();

const constants = await import(pathToFileURL(path.join(DOM_SRC, "constants.js")).href);

const sorted = (set) => [...set].sort();

// ---------------------------------------------------------------- delegated
// The 22 events the contract is written against. This list is asserted, not
// derived: it is the thing a future version bump must be forced to justify.
const EXPECTED_DELEGATED = [
  "beforeinput",
  "click",
  "contextmenu",
  "dblclick",
  "focusin",
  "focusout",
  "input",
  "keydown",
  "keyup",
  "mousedown",
  "mousemove",
  "mouseout",
  "mouseover",
  "mouseup",
  "pointerdown",
  "pointermove",
  "pointerout",
  "pointerover",
  "pointerup",
  "touchend",
  "touchmove",
  "touchstart"
];

const delegated = sorted(constants.DelegatedEvents);

const missing = EXPECTED_DELEGATED.filter((e) => !delegated.includes(e));
const added = delegated.filter((e) => !EXPECTED_DELEGATED.includes(e));
if (missing.length || added.length) {
  console.error("DelegatedEvents drift against the contract's expected 22:");
  if (missing.length) console.error("  removed upstream:", missing.join(", "));
  if (added.length) console.error("  added upstream:  ", added.join(", "));
  console.error(
    "\nThis is a real contract change: the emitter's $$name delegation set and\n" +
      "the hydration bootstrap's event list both depend on it. Update\n" +
      "EXPECTED_DELEGATED here and CONTRACT-DOM.md together, deliberately."
  );
  process.exit(1);
}

writeJson("delegated-events.json", {
  ...provenance("extract-constants.mjs"),
  $source: "dom-expressions/src/constants.js -> DelegatedEvents",
  $note:
    "Sorted alphabetically for stable diffing; upstream's Set has no meaningful order. " +
    "These are the events dom-expressions attaches ONE listener for at the document " +
    "root, dispatching via the $$<name> property on the walked-up node chain.",
  count: delegated.length,
  events: delegated
});

// -------------------------------------------------------------- other tables
// getPropAlias is a function, not a table; record its observable behavior over
// the Aliases keys plus a couple of probes so the emitter can reproduce it.
const aliasProbes = {};
for (const key of Object.keys(constants.Aliases)) {
  aliasProbes[key] = constants.Aliases[key];
}

// getPropAlias(prop, tagName) resolves element-specific property aliases.
const getPropAliasProbes = [];
for (const [prop, tag] of [
  ["class", "DIV"],
  ["formnovalidate", "BUTTON"],
  ["ismap", "IMG"],
  ["nomodule", "SCRIPT"],
  ["playsinline", "VIDEO"],
  ["readonly", "INPUT"],
  ["formnovalidate", "DIV"],
  ["readonly", "DIV"]
]) {
  getPropAliasProbes.push({
    prop,
    tagName: tag,
    result: constants.getPropAlias(prop, tag) ?? null
  });
}

writeJson("properties.json", {
  ...provenance("extract-constants.mjs"),
  $source: "dom-expressions/src/constants.js",
  $note:
    "All sorted alphabetically for stable diffing. Properties/ChildProperties/" +
    "BooleanAttributes are Sets upstream; SVGNamespace and Aliases are objects.",
  Properties: sorted(constants.Properties),
  ChildProperties: sorted(constants.ChildProperties),
  BooleanAttributes: sorted(constants.BooleanAttributes),
  Aliases: aliasProbes,
  SVGNamespace: constants.SVGNamespace,
  SVGElements: sorted(constants.SVGElements),
  DOMElements: sorted(constants.DOMElements),
  getPropAliasProbes
});

console.log(`delegated-events.json  ${delegated.length} events (all 22 expected present)`);
console.log(
  `properties.json        Properties=${constants.Properties.size} ` +
    `ChildProperties=${constants.ChildProperties.size} ` +
    `BooleanAttributes=${constants.BooleanAttributes.size} ` +
    `SVGElements=${constants.SVGElements.size} ` +
    `DOMElements=${constants.DOMElements.size}`
);
