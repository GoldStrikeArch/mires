// Node module-resolution hooks that make the raw dom-expressions sources
// importable.
//
// dom-expressions ships its src/ as bare ESM inside a package with no "type":
// "module" and no "exports" map, and its client.js imports from the bare
// specifier "rxcore" -- the reactivity seam a host framework fills in. Node
// therefore cannot import src/client.js unaided: it would treat the .js as CJS
// and choke on the import statements, then fail to resolve "rxcore".
//
// These hooks fix exactly three things and nothing else:
//
//   1. "rxcore"        -> harness/rxcore.mjs (solid-js's real reactive core, so
//                         re-exported function arities are the true ones)
//   2. "./reconcile"   -> "./reconcile.js" (extensionless relative specifiers)
//   3. src/*.js        -> forced format "module"
//
// Register with:  node --import ./harness/register-loader.mjs script.mjs
// or from inside a script via module.register() -- see register-loader.mjs.

// It also resolves bare specifiers (solid-js, parse5, @babel/core, prettier)
// against vendor/node_modules. The harness lives in contract/harness/ but the
// pinned toolchain installs into contract/vendor/node_modules, which is NOT on
// the node resolution path walking up from harness/. Rather than symlink or
// duplicate an install, we anchor bare-specifier resolution at
// vendor/package.json so harness scripts can use ordinary static imports.

import { fileURLToPath, pathToFileURL } from "node:url";
import { existsSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const RXCORE = pathToFileURL(path.join(here, "rxcore.mjs")).href;
const vendorRequire = createRequire(path.join(here, "..", "vendor", "package.json"));

// Anything under vendor/upstream/dom-expressions/src or the git tree's
// packages/dom-expressions/src is raw ESM that must be loaded as a module.
const SRC_RE = /\/(?:dom-expressions\/src|packages\/dom-expressions\/src)\/[^/]+\.js$/;

// The plugin's configured `moduleName` for each preset. Compiled fixtures
// import their runtime from these specifiers; run-trace.mjs points them at the
// recording stub so a compiled module can be executed unmodified. Keeping the
// mapping here -- rather than rewriting the compiled source -- means the bytes
// we trace are the same bytes verify-reference.mjs proved faithful.
const STUB = pathToFileURL(path.join(here, "trace.mjs")).href;
const STUBBED_MODULE_NAMES = new Set(["r-dom", "r-server"]);

// Asset imports a bundler would handle; meaningless to a call trace.
const ASSET_RE = /\.(css|scss|sass|less|svg|png|jpe?g|gif|webp|woff2?|ttf)$/i;
const EMPTY_MODULE = pathToFileURL(path.join(here, "empty-module.mjs")).href;

// Temp modules written by run-trace.mjs. Used to scope the "stub anything that
// cannot resolve" behaviour to tracing only.
const TRACE_MODULE_RE = /topcoat-trace-[0-9a-f-]+\.mjs$/;

// Synthetic stub modules are served under this scheme. The specifier carries
// the export names the importer asked for, so a NAMED import from an
// unresolvable package still links -- a default-only stub cannot satisfy
// `import { binding } from "somewhere"`, because ESM checks named exports at
// link time, before any code runs.
const STUB_SCHEME = "topcoat-stub:";

/**
 * Find the names an importer binds from a given specifier, so the synthesized
 * stub can export exactly those. Reads the importer's source rather than
 * rewriting it -- the traced bytes stay untouched.
 */
function requestedNames(parentURL, specifier) {
  const names = new Set();
  let source;
  try {
    source = readFileSync(fileURLToPath(parentURL), "utf8");
  } catch {
    return names;
  }
  const quoted = specifier.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const re = new RegExp(`import\\s+([^;]*?)\\s+from\\s*["']${quoted}["']`, "g");
  for (const m of source.matchAll(re)) {
    const clause = m[1];
    const braces = /\{([^}]*)\}/.exec(clause);
    if (braces) {
      for (const part of braces[1].split(",")) {
        const local = part.trim().split(/\s+as\s+/)[0].trim();
        if (local) names.add(local);
      }
    }
  }
  return names;
}

export async function resolve(specifier, context, nextResolve) {
  if (specifier === "rxcore") {
    return { url: RXCORE, format: "module", shortCircuit: true };
  }

  if (STUBBED_MODULE_NAMES.has(specifier)) {
    return { url: STUB, format: "module", shortCircuit: true };
  }

  // Non-JS asset imports (e.g. `import styles from "./styles.module.css"`)
  // appear in a few upstream fixtures. They are bundler concerns with no
  // runtime meaning here, so they resolve to an empty module rather than
  // aborting the trace before a single record is written.
  if (ASSET_RE.test(specifier)) {
    return { url: EMPTY_MODULE, format: "module", shortCircuit: true };
  }

  // Extensionless relative imports inside the dom-expressions sources.
  if (specifier.startsWith(".") && context.parentURL && SRC_RE.test(context.parentURL)) {
    if (!path.extname(specifier)) {
      const parentDir = path.dirname(fileURLToPath(context.parentURL));
      const candidate = path.resolve(parentDir, `${specifier}.js`);
      if (existsSync(candidate)) {
        return {
          url: pathToFileURL(candidate).href,
          format: "module",
          shortCircuit: true
        };
      }
    }
  }

  try {
    const resolved = await nextResolve(specifier, context);
    if (SRC_RE.test(resolved.url)) {
      return { ...resolved, format: "module" };
    }
    return resolved;
  } catch (err) {
    // Bare specifier that node could not find from the importer's location:
    // retry against the pinned vendor install before giving up.
    const bare = !specifier.startsWith(".") && !specifier.startsWith("/") && !specifier.includes(":");
    if (!bare) throw err;
    try {
      return {
        url: pathToFileURL(vendorRequire.resolve(specifier)).href,
        shortCircuit: true
      };
    } catch {
      // Last resort, and ONLY when tracing: some upstream fixtures import from
      // invented packages ("somewhere", "./foo") purely to exercise the
      // compiler's import handling. Those cannot resolve and never could. For a
      // call trace they are irrelevant, so stub them rather than abort.
      //
      // Gated on the IMPORTER being one of run-trace.mjs's temp modules, so the
      // extractors keep failing loudly on a genuinely missing dependency --
      // which is a real problem there, not noise.
      //
      // (An env-var gate does not work here: module hooks run on their own
      // thread with a snapshot of process.env taken at registration, so a
      // variable set later in the main thread is never visible.)
      if (TRACE_MODULE_RE.test(context.parentURL ?? "")) {
        const names = [...requestedNames(context.parentURL, specifier)].sort();
        return {
          url: `${STUB_SCHEME}${encodeURIComponent(JSON.stringify({ specifier, names }))}`,
          format: "module",
          shortCircuit: true
        };
      }
      throw err;
    }
  }
}

export async function load(url, context, nextLoad) {
  if (url.startsWith(STUB_SCHEME)) {
    const { specifier, names } = JSON.parse(
      decodeURIComponent(url.slice(STUB_SCHEME.length))
    );
    // Every requested name is an inert callable, so the module links and any
    // use of the binding is harmless. Recorded in the trace only through
    // whatever the compiled code does with it.
    const decls = names
      .map(
        (n) =>
          `export const ${n} = Object.assign(function ${n}() {}, { __stub: ${JSON.stringify(
            `${specifier}.${n}`
          )} });`
      )
      .join("\n");
    return {
      format: "module",
      shortCircuit: true,
      source: `// synthesized stub for unresolvable import ${JSON.stringify(specifier)}\n${decls}\nexport default Object.assign(function stub() {}, { __stub: ${JSON.stringify(specifier)} });\n`
    };
  }
  if (SRC_RE.test(url)) {
    return nextLoad(url, { ...context, format: "module" });
  }
  return nextLoad(url, context);
}
