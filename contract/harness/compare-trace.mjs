// Structurally diff two runtime-call traces.
//
//   node --import ./register-loader.mjs compare-trace.mjs A.trace B.trace
//        [--delta FILE] [--fixture NAME] [--json] [--quiet]
//
// Exit 0 when the traces agree once accepted deltas are applied, 1 when they do
// not, 2 on usage error.
//
// WHAT "AGREE" MEANS
// ------------------
// Not textual equality. trace.mjs already normalises identity away -- nodes are
// numbered in first-appearance order, functions become fn#N, templates tmpl#N --
// but that numbering is per-RUN, so two emitters that produce the same call
// sequence in a different internal order can still disagree on the numbers
// while agreeing on the structure. So the numbering is re-derived here, per
// file, before anything is compared: the first node either trace mentions is
// $n0 in that trace, the second $n1, and so on. A trace compares equal to
// itself with every id shifted.
//
// What survives that erasure -- op names, their order, argument shapes, and the
// aliasing between ids (which two arguments refer to the SAME node) -- is
// exactly the part the emitter is on the hook for.
//
// ALIGNMENT
// ---------
// Records are aligned by longest common subsequence over their normalised JSON,
// not pairwise by index. Index alignment reports one inserted record as "every
// record after this one differs", which is useless. Traces are tens of records
// long, so the O(n*m) table is free.
//
// ACCEPTED DELTAS
// ---------------
// Some divergence is known, understood, and not a bug -- Topcoat escaping `>`
// in text where solid does not (contract/fixtures/escaping.json $topcoatDelta)
// is the standing example. Those are declared in a delta file so they stop
// failing the comparison WITHOUT being silently dropped: every applied rule is
// reported, and a rule that matched nothing is reported too, because a stale
// rule is how an accepted delta quietly becomes an unnoticed regression.
//
// Rule kinds, deliberately few:
//
//   ignore-op     {kind, op}                    drop records with this op from both sides
//   ignore-field  {kind, op, field}             blank one field before comparing
//   rewrite       {kind, side, op, field,       string-replace a field on ONE side
//                  from, to, regex?}            (side: "a" | "b")
//   accept-diff   {kind, op, field, a, b}       demote one exact value pair to "accepted"
//
// Every rule also carries a free-text `why`, and it is required: a delta whose
// justification nobody wrote down is not an accepted delta, it is an unexplained
// one.

import { readFileSync, existsSync } from "node:fs";
import path from "node:path";

// ---------------------------------------------------------------- reading
/** Parse a trace file: a JSON-Lines header followed by one line per record. */
export function readTrace(file) {
  const text = readFileSync(file, "utf8");
  const lines = text.split("\n").filter((l) => l.trim().length);
  if (!lines.length) throw new Error(`${file}: empty trace`);
  const header = JSON.parse(lines[0]);
  if (header.$record !== "header") {
    throw new Error(`${file}: first line is not a header record`);
  }
  return { file, header, records: lines.slice(1).map((l) => JSON.parse(l)) };
}

// ---------------------------------------------------------- normalisation
//
// Ids appear in three shapes across trace.mjs:
//
//   "#3"                        an anonymous node, numbered on first appearance
//   "fn#1" / "tmpl#0"           a function or a template cloner
//   "tmpl#0:root.firstChild"    a labelled node, its structural path preserved
//   "hk#2" / "marker#5"         a hydration node
//
// Only the NUMBER carries run-specific information; the prefix and any trailing
// path are structure and must survive. So the rewriter matches the number
// together with whatever prefix it has, and leaves the rest of the string alone.
const ID_RE = /(^|[^A-Za-z0-9_$])(fn|tmpl|hk|marker)?#(\d+)/g;

/**
 * Build a renamer that assigns canonical ids in first-appearance order, shared
 * across every record in one trace so aliasing is preserved.
 */
function makeRenamer() {
  const seen = new Map();
  const counters = new Map();
  return (s) =>
    s.replace(ID_RE, (_m, lead, prefix, num) => {
      const kind = prefix ?? "n";
      const key = `${kind}#${num}`;
      if (!seen.has(key)) {
        const n = counters.get(kind) ?? 0;
        counters.set(kind, n + 1);
        seen.set(key, `$${kind}${n}`);
      }
      return `${lead}${seen.get(key)}`;
    });
}

/** Apply a renamer to every string anywhere in a value. */
function renameDeep(value, rename) {
  if (typeof value === "string") return rename(value);
  if (Array.isArray(value)) return value.map((v) => renameDeep(v, rename));
  if (value && typeof value === "object") {
    const out = {};
    for (const k of Object.keys(value)) out[k] = renameDeep(value[k], rename);
    return out;
  }
  return value;
}

/** Canonical JSON with object keys sorted, so key order never shows as a diff. */
function stable(value) {
  if (Array.isArray(value)) return `[${value.map(stable).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((k) => `${JSON.stringify(k)}:${stable(value[k])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value === undefined ? null : value);
}

export function normalize(records) {
  const rename = makeRenamer();
  // One renamer for the whole stream: ids are numbered across records, not
  // within them, which is what makes cross-record aliasing comparable.
  return records.map((r) => renameDeep(r, rename));
}

// ----------------------------------------------------------------- deltas
/**
 * Load accepted deltas. Two shapes are read:
 *
 *   *.json  { fixtures: { "<name>": [rule, ...] }, ... }  or  [rule, ...]
 *   *.md    a front-matter block delimited by --- lines whose body is JSON.
 *
 * Front matter is JSON rather than YAML on purpose: a delta file that needs a
 * YAML parser needs a dependency, and this package deliberately has none.
 */
export function loadDeltas(file, fixture) {
  if (!existsSync(file)) throw new Error(`delta file not found: ${file}`);
  const text = readFileSync(file, "utf8");

  let doc;
  if (path.extname(file) === ".md") {
    const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(text);
    if (!m) return { rules: [], source: file, note: "no front matter" };
    doc = JSON.parse(m[1]);
  } else {
    doc = JSON.parse(text);
  }

  let rules;
  if (Array.isArray(doc)) rules = doc;
  else if (Array.isArray(doc.deltas)) rules = doc.deltas;
  else if (doc.fixtures) {
    if (!fixture) {
      throw new Error(
        `${file} is keyed by fixture; pass --fixture NAME (or use a trace whose header names one)`
      );
    }
    rules = doc.fixtures[fixture] ?? [];
  } else rules = [];

  for (const r of rules) {
    if (!r.why) {
      throw new Error(
        `delta rule ${JSON.stringify(r.id ?? r.kind)} in ${file} has no "why". ` +
          `An accepted delta without a written justification is not accepted, it is unexplained.`
      );
    }
  }
  return { rules, source: file, fixture: fixture ?? null };
}

const applyRewrite = (value, rule) => {
  if (typeof value !== "string") return value;
  return rule.regex
    ? value.replace(new RegExp(rule.from, "g"), rule.to)
    : value.split(rule.from).join(rule.to);
};

/**
 * Apply the ignore/rewrite rules to one side. Returns the transformed records
 * plus a per-rule hit count, so a rule that never fired can be reported.
 */
function applyRules(records, rules, side, hits) {
  let out = records;

  for (const [i, rule] of rules.entries()) {
    if (rule.kind === "ignore-op") {
      const before = out.length;
      out = out.filter((r) => r.op !== rule.op);
      hits[i] += before - out.length;
    } else if (rule.kind === "ignore-field") {
      out = out.map((r) => {
        if (r.op !== rule.op || !(rule.field in r)) return r;
        hits[i]++;
        return { ...r, [rule.field]: "<ignored>" };
      });
    } else if (rule.kind === "rewrite" && rule.side === side) {
      out = out.map((r) => {
        if (r.op !== rule.op || !(rule.field in r)) return r;
        const next = applyRewrite(r[rule.field], rule);
        if (next === r[rule.field]) return r;
        hits[i]++;
        return { ...r, [rule.field]: next };
      });
    }
  }

  return out;
}

/** Does an accept-diff rule cover this pair of aligned records? */
function accepts(rule, a, b) {
  if (rule.kind !== "accept-diff") return false;
  if (rule.op && a?.op !== rule.op) return false;
  if (rule.op && b?.op !== rule.op) return false;
  if (!rule.field) return false;
  return String(a?.[rule.field]) === String(rule.a) && String(b?.[rule.field]) === String(rule.b);
}

// -------------------------------------------------------------- alignment
/** Longest common subsequence over stable-JSON keys. */
function align(a, b) {
  const ka = a.map(stable);
  const kb = b.map(stable);
  const n = ka.length;
  const m = kb.length;

  const table = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      table[i][j] =
        ka[i] === kb[j] ? table[i + 1][j + 1] + 1 : Math.max(table[i + 1][j], table[i][j + 1]);
    }
  }

  const ops = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (ka[i] === kb[j]) ops.push({ kind: "same", ai: i++, bi: j++ });
    else if (table[i + 1][j] >= table[i][j + 1]) ops.push({ kind: "only-a", ai: i++, bi: null });
    else ops.push({ kind: "only-b", ai: null, bi: j++ });
  }
  while (i < n) ops.push({ kind: "only-a", ai: i++, bi: null });
  while (j < m) ops.push({ kind: "only-b", ai: null, bi: j++ });

  // An only-a immediately followed by an only-b with the SAME op is far more
  // usefully read as one changed record than as a delete plus an insert.
  const merged = [];
  for (let k = 0; k < ops.length; k++) {
    const cur = ops[k];
    const next = ops[k + 1];
    if (
      cur.kind === "only-a" &&
      next?.kind === "only-b" &&
      a[cur.ai]?.op === b[next.bi]?.op
    ) {
      merged.push({ kind: "changed", ai: cur.ai, bi: next.bi });
      k++;
    } else merged.push(cur);
  }
  return merged;
}

/** The fields that differ between two records with the same op. */
function fieldDiffs(a, b) {
  const keys = [...new Set([...Object.keys(a), ...Object.keys(b)])].sort();
  return keys
    .filter((k) => stable(a[k]) !== stable(b[k]))
    .map((k) => ({ field: k, a: a[k], b: b[k] }));
}

// ---------------------------------------------------------------- compare
/**
 * Compare two traces.
 *
 * @returns {{equal: boolean, differences: object[], accepted: object[],
 *            unusedRules: object[], summary: object}}
 */
export function compareTraces(traceA, traceB, deltas = { rules: [] }) {
  const rules = deltas.rules ?? [];
  const hits = rules.map(() => 0);

  const a = normalize(applyRules(traceA.records, rules, "a", hits));
  const b = normalize(applyRules(traceB.records, rules, "b", hits));

  const differences = [];
  const accepted = [];

  for (const step of align(a, b)) {
    if (step.kind === "same") continue;

    const ra = step.ai === null ? null : a[step.ai];
    const rb = step.bi === null ? null : b[step.bi];

    if (step.kind === "changed") {
      const diffs = fieldDiffs(ra, rb);
      const covered = diffs.every((d) =>
        rules.some((rule, i) => {
          const ok = accepts(rule, ra, rb) && rule.field === d.field;
          if (ok) hits[i]++;
          return ok;
        })
      );
      const entry = { kind: "changed", op: ra.op, aIndex: step.ai, bIndex: step.bi, fields: diffs };
      (covered ? accepted : differences).push(entry);
      continue;
    }

    const entry =
      step.kind === "only-a"
        ? { kind: "missing-in-b", op: ra.op, aIndex: step.ai, record: ra }
        : { kind: "extra-in-b", op: rb.op, bIndex: step.bi, record: rb };
    differences.push(entry);
  }

  const unusedRules = rules
    .map((rule, i) => ({ rule, hits: hits[i] }))
    .filter((r) => r.hits === 0);

  // Header facts worth surfacing even when the record streams agree: a trace
  // that stopped early agrees with another stopped trace only by accident.
  const headerNotes = [];
  for (const field of ["status", "error"]) {
    if (traceA.header[field] !== traceB.header[field]) {
      headerNotes.push({ field, a: traceA.header[field], b: traceB.header[field] });
    }
  }

  return {
    equal: differences.length === 0,
    differences,
    accepted,
    unusedRules,
    headerNotes,
    summary: {
      a: { file: traceA.file, records: traceA.records.length, status: traceA.header.status },
      b: { file: traceB.file, records: traceB.records.length, status: traceB.header.status },
      differences: differences.length,
      accepted: accepted.length,
      rules: rules.length
    }
  };
}

/** Render a comparison for a human. */
export function formatResult(result) {
  const out = [];
  const { summary } = result;
  out.push(`a: ${summary.a.file}  (${summary.a.records} records, ${summary.a.status})`);
  out.push(`b: ${summary.b.file}  (${summary.b.records} records, ${summary.b.status})`);
  out.push("");

  for (const note of result.headerNotes) {
    out.push(`  header ${note.field}: a=${JSON.stringify(note.a)} b=${JSON.stringify(note.b)}`);
  }
  if (result.headerNotes.length) out.push("");

  for (const d of result.differences) {
    if (d.kind === "changed") {
      out.push(`  CHANGED  ${d.op}  (a#${d.aIndex} -> b#${d.bIndex})`);
      for (const f of d.fields) {
        out.push(`    ${f.field}:`);
        out.push(`      a: ${stable(f.a)}`);
        out.push(`      b: ${stable(f.b)}`);
      }
    } else if (d.kind === "missing-in-b") {
      out.push(`  ONLY IN A  ${d.op}  (a#${d.aIndex})  ${stable(d.record)}`);
    } else {
      out.push(`  ONLY IN B  ${d.op}  (b#${d.bIndex})  ${stable(d.record)}`);
    }
  }

  for (const a of result.accepted) {
    out.push(`  accepted  ${a.op}  (a#${a.aIndex} -> b#${a.bIndex})  ${a.fields.map((f) => f.field).join(", ")}`);
  }

  for (const u of result.unusedRules) {
    out.push(
      `  STALE RULE  ${u.rule.id ?? u.rule.kind} matched nothing -- ` +
        `an accepted delta that no longer occurs should be deleted, not left standing`
    );
  }

  out.push("");
  out.push(
    result.equal
      ? `traces agree (${summary.accepted} accepted delta(s), ${summary.rules} rule(s) loaded)`
      : `${summary.differences} difference(s), ${summary.accepted} accepted`
  );
  return out.join("\n");
}

// ------------------------------------------------------------------- CLI
if (import.meta.url === `file://${process.argv[1]}`) {
  const argv = process.argv.slice(2);
  const flag = (name) => {
    const i = argv.indexOf(`--${name}`);
    return i >= 0 ? argv[i + 1] : null;
  };
  // Only these flags consume the argument after them; the rest are booleans,
  // so a positional may legitimately follow one.
  const VALUE_FLAGS = new Set(["--delta", "--fixture"]);
  const files = argv.filter((x, i) => !x.startsWith("--") && !VALUE_FLAGS.has(argv[i - 1]));

  if (files.length !== 2) {
    console.error(
      "usage: compare-trace.mjs A.trace B.trace [--delta FILE] [--fixture NAME] [--json] [--quiet]"
    );
    process.exit(2);
  }

  const a = readTrace(path.resolve(files[0]));
  const b = readTrace(path.resolve(files[1]));

  const deltaFile = flag("delta");
  const fixture = flag("fixture") ?? a.header.fixture ?? b.header.fixture ?? null;
  const deltas = deltaFile ? loadDeltas(path.resolve(deltaFile), fixture) : { rules: [] };

  const result = compareTraces(a, b, deltas);

  if (argv.includes("--json")) process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  else if (!argv.includes("--quiet")) process.stdout.write(`${formatResult(result)}\n`);

  process.exit(result.equal ? 0 : 1);
}
