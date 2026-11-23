// Extracts the transitive subset of runtime/shim.js a compiled program reaches.
//
//   node scripts/minimal-shim.mjs [--module] <shim.js> <out.js> <program.js>...
//
// The backend writes `__rt.<name>` at every runtime call site. This script starts with those
// names, follows the `__rt` references in their shim definitions, and publishes only the closed
// set. A missing definition is an error here instead of a TypeError in a browser.

import { existsSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { entryResources, readArtifactManifest, sha256 } from "./artifact-manifest.mjs";

/**
 * Every member the shim installs, as the statement that installs it.
 *
 * @param {string} source
 * @returns {Map<string, string>}
 */
export function shimMembers(source) {
  const lines = source.split("\n");
  const members = new Map();

  // The opening object literal is split into assignments so one log helper does not keep all six.
  const open = lines.findIndex((line) => line.startsWith("globalThis.__rt = {"));
  if (open === -1) throw new Error("the shim does not open with `globalThis.__rt = {`");
  let close = open;
  while (close < lines.length && lines[close] !== "};") close += 1;
  if (close === lines.length) throw new Error("the shim's opening object literal is not closed");

  let entry = null;
  const flush = () => {
    if (entry === null) return;
    const body = entry.body.join("\n").replace(/,\s*$/, "");
    members.set(entry.name, `globalThis.__rt.${entry.name} = ${body};`);
    entry = null;
  };
  for (let index = open + 1; index < close; index += 1) {
    const line = lines[index];
    const start = /^ {2}([A-Za-z_$][A-Za-z0-9_$]*):\s*(.*)$/.exec(line);
    if (start) {
      flush();
      entry = { name: start[1], body: [start[2]] };
    } else if (entry !== null && !/^ *\/\//.test(line)) {
      entry.body.push(line);
    }
  }
  flush();

  // Every later member is one top-level assignment, ending immediately before the next one's
  // leading documentation. runtime/shim.js deliberately keeps this rigid shape.
  const starts = [];
  for (let index = close + 1; index < lines.length; index += 1) {
    const match = /^globalThis\.__rt\.([A-Za-z_$][A-Za-z0-9_$]*)\s*=/.exec(lines[index]);
    if (match) starts.push({ line: index, name: match[1] });
  }
  for (let index = 0; index < starts.length; index += 1) {
    const from = starts[index].line;
    let to = (index + 1 < starts.length ? starts[index + 1].line : lines.length) - 1;
    while (to > from && (lines[to].trim() === "" || lines[to].startsWith("//"))) to -= 1;
    members.set(starts[index].name, lines.slice(from, to + 1).join("\n"));
  }
  return members;
}

/**
 * Every runtime member `source` reaches.
 *
 * @param {string} source
 * @returns {Set<string>}
 */
export function referencedMembers(source) {
  const members = new Set();
  for (const match of source.matchAll(/(?:globalThis\.)?__rt\.([A-Za-z_$][A-Za-z0-9_$]*)/g)) {
    const before = match.index === 0 ? "" : source[match.index - 1];
    if (!/[A-Za-z0-9_$.]/.test(before)) members.add(match[1]);
  }
  return members;
}

/**
 * Builds the smallest closed runtime for `programs`.
 *
 * @param {string[]} programs
 * @param {string} shimSource
 * @param {{module?: boolean, direct?: Iterable<string>}} options
 * @returns {{direct: string[], names: string[], source: string}}
 */
export function minimalShim(programs, shimSource, { module = false, direct: declared } = {}) {
  const members = shimMembers(shimSource);
  // A member's source group can also declare lexical helpers used by another
  // member. Retain the owning group for those references, including cycles.
  const bindings = new Map();
  for (const [name, source] of members) {
    for (const match of source.matchAll(/^(?:const|let|var|function)\s+([A-Za-z_$][A-Za-z0-9_$]*)/gm)) {
      bindings.set(match[1], name);
    }
  }
  const direct = new Set(declared ?? programs.flatMap((program) => [...referencedMembers(program)]));
  const needed = new Set(direct);
  const queue = [...direct];
  const missing = new Set();

  while (queue.length > 0) {
    const name = queue.pop();
    const statement = members.get(name);
    if (statement === undefined) {
      missing.add(name);
      continue;
    }
    const dependencies = referencedMembers(statement);
    for (const token of statement.matchAll(/[A-Za-z_$][A-Za-z0-9_$]*/g)) {
      if (bindings.has(token[0])) dependencies.add(bindings.get(token[0]));
    }
    for (const dependency of dependencies) {
      if (needed.has(dependency)) continue;
      needed.add(dependency);
      queue.push(dependency);
    }
  }
  if (missing.size > 0) {
    throw new Error(`the program reaches missing runtime members: ${[...missing].sort().join(", ")}`);
  }

  const names = [...members.keys()].filter((name) => needed.has(name));
  // Keyed DOM/hydration bundles and lifecycle tests still read hooks and state
  // through `globalThis.__rt`, so preserve the classic global object for them.
  const preserveGlobal = !module || names.some((name) => name.startsWith("keyed_") || name === "_keyed_states");
  if (preserveGlobal) {
    const output = [
      "// The subset of runtime/shim.js this program reaches. Generated; do not edit.",
      "globalThis.__rt = globalThis.__rt || {};",
      ...names.map((name) => members.get(name)),
    ];
    if (module) {
      output.push(`export const {\n${names.map((name) => `  ${name},`).join("\n")}\n} = globalThis.__rt;`);
    }
    output.push("");
    return { direct: [...direct].sort(), names, source: output.join("\n") };
  }

  // Lexical ESM mode avoids mutating a global `__rt` object so bundlers/minifiers can
  // mangle, inline, and dead-strip helpers. Keep `names` unchanged for manifest checks.
  const elideBoxes = !needed.has("box") && !needed.has("tail_ref") && !needed.has("tail_slice");
  const elideByteViews = !needed.has("_bv_enable");
  const elideElementViews = !needed.has("_element_view");
  const errPrefixPattern = /new Error\((\s*)"rustc_codegen_js: /g;
  const rtRefPattern =
    /'(?:\\.|[^'\\])*'|"(?:\\.|[^"\\])*"|\/\/[^\r\n]*|\/\*[\s\S]*?\*\/|(?:globalThis\.)?__rt\.([A-Za-z_$][A-Za-z0-9_$]*)/g;
  const statements = names.map((name) => {
    if (elideBoxes && name === "_boxes") return "let $rt__boxes = null;";
    if (elideByteViews && name === "_bv") return "const $rt__bv = null;";
    if (elideElementViews && name === "_element_view_of") return "const $rt__element_view_of = null;";
    let stmt = members.get(name);
    if (elideBoxes && name === "_put") {
      stmt = stmt.replace(
        /\r?\n\s*if\s*\(\s*value\s*!==\s*null\s*&&\s*typeof value\s*===\s*"object"\s*&&[^;{}]*_boxes\.has\(value\)\s*\)\s*\{\s*[^}]*_boxes\.set\([^;]*\);\s*\}/,
        "",
      );
    }
    stmt = stmt.replace(/^globalThis\.__rt\.([A-Za-z_$][A-Za-z0-9_$]*)\s*=/, "let $rt_$1 =");
    return stmt.replace(rtRefPattern, (match, refName, offset, full) => {
      if (refName === undefined) return match;
      const before = offset === 0 ? "" : full[offset - 1];
      if (/[A-Za-z0-9_$.]/.test(before)) return match;
      return `$rt_${refName}`;
    });
  });

  let body = statements.join("\n");
  let errCount = 0;
  for (const _ of body.matchAll(errPrefixPattern)) errCount += 1;

  const output = [
    "// The subset of runtime/shim.js this program reaches. Generated; do not edit.",
  ];
  if (errCount >= 2) {
    body = body.replace(errPrefixPattern, '$rt_err($1"');
    output.push('const $rt_err = (m) => new Error("rustc_codegen_js: " + m);');
  }
  if (body.length > 0) output.push(body);
  output.push(
    names.length > 0
      ? `export {\n${names.map((name) => `  $rt_${name} as ${name},`).join("\n")}\n};`
      : "export {};",
    "",
  );
  return { direct: [...direct].sort(), names, source: output.join("\n") };
}

function main() {
  const args = process.argv.slice(2);
  let module = false;
  let full = false;
  let manifestPath;
  while (args[0]?.startsWith('--')) {
    const option = args.shift();
    if (option === '--module') module = true;
    else if (option === '--full') full = true;
    else if (option === '--manifest') manifestPath = args.shift();
    else throw new Error(`unknown option ${option}`);
  }
  const [shimPath, outPath, ...programPaths] = args;
  if (!shimPath || !outPath || programPaths.length === 0 && !manifestPath) {
    console.error("usage: minimal-shim.mjs [--module] <shim.js> <out.js> <program.js>...");
    process.exit(2);
  }

  try {
    const shimSource = readFileSync(shimPath, "utf8");
    let direct;
    let deployment;
    let providerName;
    const requirements = [];
    if (manifestPath) {
      if (programPaths.length) throw new Error('manifest mode takes its programs from the entry graph');
      const { manifest, directory, contents } = readArtifactManifest(manifestPath);
      deployment = manifest;
      direct = new Set();
      mkdirSync(path.dirname(outPath), { recursive: true });
      const outputIdentity = existsSync(outPath) ? realpathSync(outPath)
        : path.join(realpathSync(path.dirname(outPath)), path.basename(outPath));
      providerName = path.relative(directory, outputIdentity).split(path.sep).join('/');
      if (!providerName || providerName.split('/').some(part => !part || part === '.' || part === '..')
        || path.isAbsolute(providerName) || providerName.includes('\\')) {
        throw new Error('runtime output must be inside the artifact package');
      }
      if (manifest.artifacts[providerName] && manifest.artifacts[providerName].kind !== 'runtime') {
        throw new Error('runtime output collides with a compiler artifact');
      }
      for (const name of entryResources(manifest)) {
        const artifact = manifest.artifacts[name];
        if (artifact.kind !== 'program') continue;
        const runtime = artifact.runtime;
        const observed = [...referencedMembers(contents.get(name))].sort();
        const declared = runtime ? [...runtime.direct, ...runtime.external].sort() : [];
        if (JSON.stringify(observed) !== JSON.stringify(declared)) {
          throw new Error(`${name}: runtime members disagree with linker manifest`);
        }
        if (!runtime) continue;
        if (runtime.source_sha256 !== sha256(shimSource)) throw new Error('canonical runtime source differs from compiler manifest');
        if (module !== (runtime.module != null)) throw new Error(`${name}: runtime module mode disagrees with manifest`);
        if (module && runtime.external.length) {
          throw new Error(`${name}: canonical runtime cannot supply external host members (${runtime.external.join(', ')}); use an application provider with shim(false)`);
        }
        const canonical = shimMembers(shimSource);
        if (runtime.external.some(member => canonical.has(member))) {
          throw new Error(`${name}: canonical runtime helper declared as external`);
        }
        if (module && (!runtime.module.startsWith('./')
          || path.resolve(directory, runtime.module) !== outputIdentity)) {
          throw new Error(`${name}: output does not supply declared runtime ${runtime.module}`);
        }
        const closure = minimalShim([], shimSource, { direct: runtime.direct }).names.sort();
        if (JSON.stringify(closure) !== JSON.stringify(runtime.closure)) throw new Error(`${name}: runtime closure differs from manifest`);
        runtime.direct.forEach(member => direct.add(member));
        requirements.push(artifact);
      }
    }
    if (full) direct = shimMembers(shimSource).keys();
    const result = minimalShim(
      programPaths.map((program) => readFileSync(program, "utf8")),
      shimSource,
      { module, direct },
    );
    mkdirSync(path.dirname(outPath), { recursive: true });
    writeFileSync(outPath, result.source);
    if (deployment) {
      for (const artifact of requirements) {
        artifact.runtime.artifact = providerName;
        if (artifact.runtime.module != null) artifact.imports[artifact.runtime.module] = providerName;
      }
      deployment.artifacts[providerName] = {
        kind: 'runtime', bytes: Buffer.byteLength(result.source), sha256: sha256(result.source),
        imports: {}, source_map: null, declarations: null, runtime: null,
        items: [], exports: module ? [...result.names].sort() : [],
      };
      writeFileSync(manifestPath, `${JSON.stringify(deployment, null, 2)}\n`);
      readArtifactManifest(manifestPath);
    }
    console.log(
      `minimal-shim: ${result.names.length} members (${result.direct.length} direct) -> ${outPath}`,
    );
  } catch (error) {
    console.error(`minimal-shim: ${error.message}`);
    process.exit(1);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
