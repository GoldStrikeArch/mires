// orders.mjs <linkdir> <shim.js> <expected file | -> <calls: chunk:entry,chunk:entry,... | -> [mode]
//
// Loads the chunk files a link wrote into <linkdir> (every *.js but program.js) in every order a
// host could: each order is one generated main module whose `import "./file.js"` lines come in
// that order, followed by the imports of the entries to call and the calls. With up to four files
// every permutation is run; with more, each file first with the rest ascending and descending.
// Prints one JSON line: { files, orders, ok, bad: [{ order, status, error }] }.
// A run is ok when node exits 0 and (when an expectation is given) stdout equals it.
import { readdirSync, mkdirSync, copyFileSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { join } from "node:path";

const [dir, shim, expectedPath, callsArg] = process.argv.slice(2);
const files = readdirSync(dir).filter((f) => f.endsWith(".js") && f !== "program.js").sort();
const run = join(dir, "run");
rmSync(run, { recursive: true, force: true });
mkdirSync(run, { recursive: true });
writeFileSync(join(run, "package.json"), '{ "type": "module" }\n');
copyFileSync(shim, join(run, "shim.js"));
for (const f of files) copyFileSync(join(dir, f), join(run, f));
const expected = expectedPath === "-" ? null : readFileSync(expectedPath, "utf8");
const calls = callsArg === "-" || !callsArg ? [] : callsArg.split(",").map((p) => p.split(":"));

function permutations(list) {
  if (list.length <= 1) return [list];
  const out = [];
  list.forEach((head, at) => {
    const rest = list.slice(0, at).concat(list.slice(at + 1));
    for (const tail of permutations(rest)) out.push([head, ...tail]);
  });
  return out;
}
let orders;
if (files.length <= 4) orders = permutations(files);
else {
  orders = [];
  files.forEach((head, at) => {
    const rest = files.slice(0, at).concat(files.slice(at + 1));
    orders.push([head, ...rest], [head, ...rest.slice().reverse()]);
  });
}
const bad = [];
let ok = 0;
orders.forEach((order, n) => {
  let text = order.map((f) => `import "./${f}";\n`).join("");
  text += calls.map(([chunk, entry]) => `import { ${entry} } from "./${chunk}.js";\n`).join("");
  text += calls.map(([, entry]) => `${entry}();\n`).join("");
  const main = join(run, `main_${n}.mjs`);
  writeFileSync(main, text);
  const result = spawnSync("node", [main], { encoding: "utf8" });
  const good = result.status === 0 && (expected === null || result.stdout === expected);
  if (good) ok += 1;
  else {
    const error =
      (result.stderr || "").split("\n").find((line) => /Error/.test(line)) ||
      (result.status === 0 ? "stdout differs from the expectation" : "exit " + result.status);
    bad.push({ order: order.map((f) => f.replace(/\.js$/, "")).join(">"), status: result.status, error: error.slice(0, 160) });
  }
});
console.log(JSON.stringify({ files: files.map((f) => f.replace(/\.js$/, "")), orders: orders.length, ok, bad }));
