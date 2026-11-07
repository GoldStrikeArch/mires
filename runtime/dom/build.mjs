// Build the vendored DOM runtime into a single self-contained ESM bundle.
//
//   node build.mjs           build dist/ and record the size baseline
//   node build.mjs --size    build and print sizes without touching the baseline
//
// Settings mirror crates/topcoat-runtime/browser/tsup.config.ts:
//   format esm / noExternal (bundle everything) / platform browser /
//   target es2022 / minify / sourcemap.

import { build } from "esbuild";
import { readFileSync, writeFileSync, existsSync, mkdirSync } from "node:fs";
import { gzipSync } from "node:zlib";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const dist = path.join(here, "dist");
const outfile = path.join(dist, "mires-dom.js");
const maxbytesFile = `${outfile}.maxbytes`;
const sizeOnly = process.argv.includes("--size");

mkdirSync(dist, { recursive: true });

const result = await build({
  entryPoints: [path.join(here, "src", "index.js")],
  outfile,
  bundle: true,
  // Bundle everything: no import survives into the output. The artifact must be
  // loadable by a browser with no resolver and no network beyond fetching it.
  packages: "bundle",
  external: [],
  format: "esm",
  platform: "browser",
  target: "es2022",
  minify: true,
  sourcemap: true,
  legalComments: "none",
  metafile: true,
  // solid ships a dev build behind this condition; make sure we get neither it
  // nor the SSR build.
  conditions: ["browser", "import"],
  define: { "process.env.NODE_ENV": '"production"' }
});

for (const w of result.warnings) console.warn(`warning: ${w.text}`);

const code = readFileSync(outfile);
const gz = gzipSync(code, { level: 9 });
const raw = code.length;
const gzip = gz.length;

// Assert the bundle really is self-contained -- a surviving bare import would
// mean the artifact needs a resolver at load time.
const text = code.toString("utf8");
const leakedImport = /(?:^|[;\s])import\s*(?:[^"']*from\s*)?["']([^"'.][^"']*)["']/.exec(text);
if (leakedImport) {
  console.error(
    `bundle is not self-contained: it still imports ${JSON.stringify(leakedImport[1])}`
  );
  process.exit(1);
}

console.log(`dist/mires-dom.js      ${raw} bytes raw`);
console.log(`                         ${gzip} bytes gzip`);

if (!sizeOnly) {
  if (existsSync(maxbytesFile)) {
    const previous = Number(readFileSync(maxbytesFile, "utf8").trim().split(/\s+/)[0]);
    if (Number.isFinite(previous)) {
      const delta = gzip - previous;
      const pct = ((delta / previous) * 100).toFixed(1);
      console.log(
        `baseline                 ${previous} bytes gzip  (${delta >= 0 ? "+" : ""}${delta}, ${delta >= 0 ? "+" : ""}${pct}%)`
      );
    }
  } else {
    // First measurement establishes the baseline.
    writeFileSync(
      maxbytesFile,
      `${gzip}\n# gzipped byte size of dist/mires-dom.js.\n# Baseline established ${new Date().toISOString().slice(0, 10)} by runtime/dom/build.mjs.\n# This is a measured value, not a budget target -- update it deliberately when\n# the artifact legitimately grows, so unexplained growth shows up in review.\n`
    );
    console.log(`baseline                 established at ${gzip} bytes gzip`);
  }
}
