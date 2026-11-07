// Copy the babel-plugin test fixture directories VERBATIM out of the pinned git
// tree into fixtures/upstream/, with a README recording their origin.
//
//   node --import ./register-loader.mjs import-upstream-fixtures.mjs
//
// These are committed so the contract is self-contained: the expected outputs
// are the ground truth verify-reference.mjs checks against, and they must be
// reviewable in-tree without re-fetching a tarball.
//
// Byte-for-byte copies. Every file's sha256 is recorded in the README so an
// accidental edit is detectable.

import { GIT, FIXTURES, provenance, lock } from "./paths.mjs";
import { readdirSync, readFileSync, writeFileSync, mkdirSync, rmSync, existsSync } from "node:fs";
import { createHash } from "node:crypto";
import path from "node:path";

const FAMILIES = [
  "__dom_fixtures__",
  "__dom_hydratable_fixtures__",
  "__ssr_hydratable_fixtures__"
];

const SRC = path.join(GIT, "packages", "babel-plugin-jsx-dom-expressions", "test");
const DEST = path.join(FIXTURES, "upstream");

if (!existsSync(SRC)) {
  throw new Error(`missing ${SRC} -- run contract/vendor/fetch.sh first`);
}

rmSync(DEST, { recursive: true, force: true });
mkdirSync(DEST, { recursive: true });

const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");

const manifest = [];
let fileCount = 0;

for (const family of FAMILIES) {
  const familySrc = path.join(SRC, family);
  if (!existsSync(familySrc)) throw new Error(`missing fixture family ${familySrc}`);

  const familyDest = path.join(DEST, family);
  mkdirSync(familyDest, { recursive: true });

  const cases = readdirSync(familySrc, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => d.name)
    .sort();

  for (const name of cases) {
    const caseSrc = path.join(familySrc, name);
    const caseDest = path.join(familyDest, name);
    mkdirSync(caseDest, { recursive: true });

    const files = readdirSync(caseSrc, { withFileTypes: true })
      .filter((d) => d.isFile())
      .map((d) => d.name)
      .sort();

    for (const file of files) {
      // Buffer copy, not a string round trip: preserves bytes exactly,
      // including the missing trailing newline on a few output.js files.
      const buf = readFileSync(path.join(caseSrc, file));
      writeFileSync(path.join(caseDest, file), buf);
      manifest.push({
        path: `${family}/${name}/${file}`,
        bytes: buf.length,
        sha256: sha256(buf)
      });
      fileCount++;
    }
  }
}

const p = provenance("import-upstream-fixtures.mjs");

const readme = `# Upstream babel-plugin fixtures (verbatim copy)

These directories are copied **byte for byte** from the pinned dom-expressions
monorepo. Nothing here is authored by us and nothing here should ever be edited
by hand.

## Origin

| | |
|---|---|
| repo | \`${lock.git.repo}\` |
| commit | \`${lock.git.sha}\` |
| path | \`packages/babel-plugin-jsx-dom-expressions/test/\` |
| plugin version at that commit | \`${lock.npm["babel-plugin-jsx-dom-expressions"].version}\` |
| extracted | ${lock.extractionDate} |
| copied by | \`contract/harness/import-upstream-fixtures.mjs\` |

Families copied: ${FAMILIES.map((f) => `\`${f}\``).join(", ")}.

## What they are

Each case directory holds:

- \`code.js\`, the JSX input (named \`.js\` despite being JSX; the plugin pulls
  in \`@babel/plugin-syntax-jsx\` itself, and babel-plugin-tester does not infer
  a parser from the extension).
- \`output.js\`, the expected compiled output, as produced by
  babel-plugin-tester and **formatted with prettier** before being written.

## Why they are committed

They are the ground truth for G0 proof 3. \`contract/harness/verify-reference.mjs\`
compiles every \`code.js\` with our pinned toolchain and compares against
\`output.js\`. A byte match proves our pin, babel version, plugin version,
plugin options, and formatter, is faithful to the tree that produced them.

Reproducing them exactly requires more than the plugin version:

- \`@babel/core@${lock.npm["@babel/core"].version}\` with \`@babel/traverse@7.23.2\`
  forced. \`Scope.generateUid\` numbering changed in later 7.x, which renames every
  \`_tmpl$\`/\`_el$\` identifier.
- \`prettier@2.8.2\` plus the upstream repo's \`.prettierrc\`.

See \`contract/CONTRACT-DOM.md\` for the full option sets.

## Known wrinkle

Three \`output.js\` files lack a trailing newline where the other 40 have one
(\`__dom_fixtures__/SVG\`, \`__dom_hydratable_fixtures__/SVG\`,
\`__ssr_fixtures__/attributeExpressions\`). That is an inconsistency in the
upstream repo, not a formatting difference, our output is prettier's, which
always ends with a newline. \`verify-reference.mjs\` reports those three as
\`trailing-whitespace\` rather than \`exact\`.

## Integrity

${fileCount} files. sha256 of each, so an accidental edit is detectable:

\`\`\`
${manifest.map((m) => `${m.sha256}  ${m.path}`).join("\n")}
\`\`\`
`;

writeFileSync(path.join(DEST, "README.md"), readme);
writeFileSync(
  path.join(DEST, "manifest.json"),
  `${JSON.stringify({ ...p, $source: `${lock.git.repo}@${lock.git.sha}`, families: FAMILIES, fileCount, files: manifest }, null, 2)}\n`
);

console.log(`fixtures/upstream/  ${FAMILIES.length} families, ${fileCount} files copied verbatim`);
for (const family of FAMILIES) {
  const n = manifest.filter((m) => m.path.startsWith(`${family}/`)).length;
  console.log(`  ${family}  ${n} files`);
}
