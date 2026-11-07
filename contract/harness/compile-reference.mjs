// Compile a .jsx file with babel + the pinned babel-plugin-jsx-dom-expressions.
// This is the REFERENCE compiler: whatever the Rust emitter produces is
// measured against what this produces.
//
// As a CLI:
//   node --import ./register-loader.mjs compile-reference.mjs FILE.jsx \
//        [--generate dom|ssr|universal] [--hydratable] [--module-name r-dom] [--raw]
//
// As a library:
//   import { compile, PRESETS } from "./compile-reference.mjs";
//   compile(source, { filename, ...PRESETS.domHydratable });
//
// FIDELITY NOTE (this is the crux of G0 proof 3)
// ----------------------------------------------
// Upstream's expected outputs are produced by babel-plugin-tester@10.1.0, which
// runs every result through prettier BEFORE writing output.js. So the committed
// fixtures are prettier artifacts, and reproducing them byte-for-byte requires
// running the same formatter with the same config. We do not reimplement it: we
// import babel-plugin-tester's own `prettierFormatter` and call it exactly as
// the tester does --
//
//   prettier.format(code, { filepath: filename, ...prettier.resolveConfig.sync(cwd) })
//
// -- with cwd set to the fixture's directory, so prettier's config resolution
// walks up and finds the upstream repo's .prettierrc on its own.
//
// Pass { raw: true } to skip formatting and see babel's unformatted output.

import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import path from "node:path";
import { VENDOR, GIT } from "./paths.mjs";

const require = createRequire(path.join(VENDOR, "package.json"));

const babel = require("@babel/core");
const plugin = require("babel-plugin-jsx-dom-expressions");
const { prettierFormatter } = require("babel-plugin-tester");

/**
 * The exact pluginOptions from upstream's own spec files at the pinned commit.
 * Reproduced verbatim; see vendor/upstream/git/packages/
 * babel-plugin-jsx-dom-expressions/test/*.spec.js
 */
export const PRESETS = {
  // test/dom.spec.js
  dom: {
    moduleName: "r-dom",
    builtIns: ["For", "Show"],
    generate: "dom",
    wrapConditionals: true,
    contextToCustomElements: true,
    staticMarker: "@once",
    requireImportSource: false
  },
  // test/dom-hydratable.spec.js
  // NOTE: no wrapConditionals and no requireImportSource here -- upstream omits
  // both in the hydratable config. Adding them changes the output.
  domHydratable: {
    moduleName: "r-dom",
    builtIns: ["For", "Show"],
    generate: "dom",
    hydratable: true,
    contextToCustomElements: true,
    staticMarker: "@once"
  },
  // test/ssr.spec.js
  ssr: {
    moduleName: "r-server",
    builtIns: ["For", "Show"],
    generate: "ssr",
    wrapConditionals: true,
    contextToCustomElements: true,
    staticMarker: "@once"
  },
  // test/ssr-hydratable.spec.js
  ssrHydratable: {
    moduleName: "r-server",
    builtIns: ["For", "Show"],
    generate: "ssr",
    hydratable: true,
    contextToCustomElements: true,
    staticMarker: "@once"
  }
};

/** Maps a fixture directory name to the preset that produced its output.js. */
export const FIXTURE_PRESETS = {
  __dom_fixtures__: PRESETS.dom,
  __dom_hydratable_fixtures__: PRESETS.domHydratable,
  __ssr_fixtures__: PRESETS.ssr,
  __ssr_hydratable_fixtures__: PRESETS.ssrHydratable
};

/**
 * Compile JSX source with the pinned plugin.
 *
 * @param {string} source the .jsx source text
 * @param {object} opts
 * @param {string} opts.filename path used for babel + prettier config resolution
 * @param {string} [opts.cwd] directory prettier resolves its config from
 * @param {boolean} [opts.raw] skip prettier formatting
 * @param {...any} opts.pluginOptions everything else is passed to the plugin
 * @returns {string} the compiled (and by default formatted) module source
 */
export function compile(source, { filename, cwd, raw = false, ...pluginOptions } = {}) {
  // The plugin warns about a few constructs (e.g. a hand-written data-hk) via
  // console.log, which goes to STDOUT. run-trace.mjs writes JSON Lines to
  // stdout, so an unredirected warning corrupts the stream. Route the plugin's
  // chatter to stderr for the duration of the transform -- it is still visible,
  // just not mixed into machine-readable output.
  const realLog = console.log;
  console.log = (...args) => console.error(...args);
  let result;
  try {
    result = babel.transformSync(source, {
      filename,
      // Upstream's tester drives the plugin in isolation: no babelrc, no config
      // file, no presets. Any of those would change the output.
      babelrc: false,
      configFile: false,
      plugins: [[plugin, pluginOptions]]
    });
  } finally {
    console.log = realLog;
  }

  if (raw) return result.code;
  return prettierFormatter(result.code, {
    cwd: cwd ?? (filename ? path.dirname(filename) : process.cwd()),
    filename
  });
}

/**
 * Compile one of upstream's own fixtures from the pinned git tree.
 *
 * @param {string} family e.g. "__dom_hydratable_fixtures__"
 * @param {string} name e.g. "simpleElements"
 */
export function compileUpstreamFixture(family, name) {
  const dir = path.join(
    GIT,
    "packages",
    "babel-plugin-jsx-dom-expressions",
    "test",
    family,
    name
  );
  // Upstream names the input code.js (not code.jsx) even though it is JSX --
  // babel-plugin-tester does not infer the parser from the extension, and the
  // plugin pulls in @babel/plugin-syntax-jsx itself via `inherits`.
  const codePath = path.join(dir, "code.js");
  const source = readFileSync(codePath, "utf8");
  const preset = FIXTURE_PRESETS[family];
  if (!preset) throw new Error(`no preset registered for fixture family ${family}`);

  return {
    dir,
    codePath,
    expectedPath: path.join(dir, "output.js"),
    expected: readFileSync(path.join(dir, "output.js"), "utf8"),
    actual: compile(source, { filename: codePath, cwd: dir, ...preset })
  };
}

// ------------------------------------------------------------------- CLI
if (import.meta.url === `file://${process.argv[1]}`) {
  const argv = process.argv.slice(2);
  const file = argv.find((a) => !a.startsWith("--"));
  if (!file) {
    console.error(
      "usage: compile-reference.mjs FILE.jsx [--generate dom|ssr] [--hydratable] " +
        "[--module-name NAME] [--raw]"
    );
    process.exit(2);
  }
  const flag = (name, fallback) => {
    const i = argv.indexOf(`--${name}`);
    return i >= 0 ? argv[i + 1] : fallback;
  };
  const generate = flag("generate", "dom");
  const hydratable = argv.includes("--hydratable");
  const base =
    generate === "ssr" ? (hydratable ? PRESETS.ssrHydratable : PRESETS.ssr) : hydratable ? PRESETS.domHydratable : PRESETS.dom;

  const opts = { ...base, generate };
  const moduleName = flag("module-name", null);
  if (moduleName) opts.moduleName = moduleName;

  const abs = path.resolve(file);
  process.stdout.write(
    compile(readFileSync(abs, "utf8"), {
      filename: abs,
      cwd: path.dirname(abs),
      raw: argv.includes("--raw"),
      ...opts
    })
  );
}
