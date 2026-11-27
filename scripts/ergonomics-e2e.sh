#!/usr/bin/env bash
# End-to-end verification of the ergonomic Rust -> JS/TS workflow:
#   - cargo-mires CLI + RUSTC_WRAPPER
#   - thin mires-std sysroot (ordinary std Rust, HashMap, TypeId, dyn Error)
#   - zero-macro #[js_export] (plain structs/enums, Option, Result, HashMap, async fn, impl class)
#   - self-contained inline runtime ESM + .d.mts + package.json
#   - Node.js ESM execution + TypeScript `tsc --strict --module nodenext` (when tsc is installed)

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
cargo_mires="$root/target/release/cargo-mires"

if [ ! -x "$cargo_mires" ]; then
	echo "ergonomics-e2e.sh: missing $cargo_mires (run cargo build --release -p cargo-mires)" >&2
	exit 1
fi

tmp=$(mktemp -d)
cleanup() {
	rm -rf "$tmp" \
		"$root/examples/fullstack-workspace/target" \
		"$root/examples/fullstack-workspace/Cargo.lock" \
		"$root/examples/npm-markdown/target" \
		"$root/examples/npm-markdown/Cargo.lock"
}
trap cleanup EXIT

echo "==> native host tests for examples"
cargo test --manifest-path "$root/examples/fullstack-workspace/Cargo.toml" --target-dir "$tmp/host-fs"
cargo test --manifest-path "$root/examples/npm-markdown/Cargo.toml" --target-dir "$tmp/host-md"

echo "==> cargo-mires build (release) on fullstack-workspace & npm-markdown"
"$cargo_mires" build \
	--manifest-path "$root/examples/fullstack-workspace/Cargo.toml" \
	-p client --release \
	--out-dir "$tmp/fs" \
	--target-dir "$tmp/fs-target"

"$cargo_mires" build \
	--manifest-path "$root/examples/npm-markdown/Cargo.toml" \
	--release --npm \
	--out-dir "$tmp/md" \
	--target-dir "$tmp/md-target"

echo "==> cargo-mires build (dev & release) on js_export v3 feature crate (async, HashMap, class, export type)"
mkdir -p "$tmp/v3-crate/src"
cat >"$tmp/v3-crate/Cargo.toml" <<EOF
[workspace]

[package]
name = "v3_e2e"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
mires = { path = "$root/mires" }

[package.metadata.mires]
format = "esm"
runtime = "inline"
dts = true
EOF

cat >"$tmp/v3-crate/src/lib.rs" <<'EOF'
use std::collections::HashMap;
use mires::{js_export, Promise};

pub struct CountReport {
    pub total: u32,
    pub counts: HashMap<String, u32>,
}

#[js_export(name = "tallyWords")]
pub fn tally_words(mut base: HashMap<String, u32>, extra: Vec<String>) -> CountReport {
    for word in extra {
        *base.entry(word).or_insert(0) += 1;
    }
    let total = base.values().copied().sum();
    CountReport {
        total,
        counts: base,
    }
}

#[js_export(name = "invertIds")]
pub fn invert_ids(items: HashMap<i32, String>) -> HashMap<i32, String> {
    items
        .into_iter()
        .map(|(k, v)| (k * 10, format!("{v}!")))
        .collect()
}

#[js_export(name = "asyncGreet")]
pub async fn async_greet(name: &str, fail: Option<bool>) -> Result<String, String> {
    let _ = Promise::resolve(mires::JsValue::erase(1_i32)).await;
    if fail == Some(true) {
        return Err(format!("rejected:{name}"));
    }
    Ok(format!("hello, {name}"))
}

#[js_export]
pub struct Counter {
    value: i32,
    label: String,
}

#[js_export]
impl Counter {
    pub fn new(initial: i32, label: String) -> Self {
        Self {
            value: initial,
            label,
        }
    }

    pub fn from_zero(label: String) -> Self {
        Self { value: 0, label }
    }

    pub fn increment(&mut self, by: i32) -> i32 {
        self.value += by;
        self.value
    }

    pub fn describe(&self) -> String {
        format!("{}:{}", self.label, self.value)
    }

    pub fn into_value(self) -> i32 {
        self.value
    }

    pub fn max_value(a: i32, b: i32) -> i32 {
        a.max(b)
    }
}
EOF

"$cargo_mires" build \
	--manifest-path "$tmp/v3-crate/Cargo.toml" \
	--out-dir "$tmp/v3-dev" \
	--target-dir "$tmp/v3-target"

"$cargo_mires" build \
	--manifest-path "$tmp/v3-crate/Cargo.toml" \
	--release \
	--out-dir "$tmp/v3-rel" \
	--target-dir "$tmp/v3-target"

echo "==> Node.js ESM execution checks"
node --input-type=module -e "
import assert from 'node:assert/strict';
import { filterTasks, summarizeTasks, validateTitle } from '$tmp/fs/client.mjs';
import { renderMarkdown } from '$tmp/md/npm_markdown.mjs';

const tasks = [
  { id: 1, title: 'Ship mires', done: true, priority: 'High', tags: ['rust', 'mires'] },
  { id: 2, title: 'Write docs', done: false, priority: 'Medium', tags: ['rust', 'docs'] },
];
assert.equal(filterTasks(tasks).length, 2);
assert.equal(filterTasks(tasks, { done: true }).length, 1);
assert.equal(summarizeTasks(tasks).completed, 1);
assert.equal(validateTitle('  Hi  '), 'Hi');
assert.throws(() => validateTitle('   '), /empty/);

const out = renderMarkdown('# Hello\n\nSome **bold** and \`code\`');
assert.equal(out.word_count, 5);
assert.deepEqual(out.headings, ['Hello']);
const dup = renderMarkdown('# Hi\n# Hi', { heading_ids: true });
assert.ok(dup.html.includes('id=\"hi-2\"'));
assert.throws(() => renderMarkdown('   ', { strict: true }), /empty markdown input/);

for (const mode of ['v3-dev', 'v3-rel']) {
  const mod = await import('$tmp/' + mode + '/v3_e2e.mjs');
  const rep = mod.tallyWords({ rust: 2 }, ['rust', 'js']);
  assert.equal(rep.total, 4, mode);
  assert.deepEqual(rep.counts, { rust: 3, js: 1 }, mode);

  const inverted = mod.invertIds(new Map([[2, 'two'], [5, 'five']]));
  assert.ok(inverted instanceof Map, mode);
  assert.equal(inverted.get(20), 'two!', mode);
  assert.equal(inverted.get(50), 'five!', mode);

  const greeting = await mod.asyncGreet('world');
  assert.equal(greeting, 'hello, world', mode);
  await assert.rejects(() => mod.asyncGreet('bad', true), /rejected:bad/, mode);

  const c = new mod.Counter(10, 'hits');
  assert.equal(c.increment(5), 15, mode);
  assert.equal(c.describe(), 'hits:15', mode);
  assert.equal(mod.Counter.max_value(3, 9), 9, mode);
  assert.equal(c.into_value(), 15, mode);
  assert.throws(() => c.describe(), /already freed/, mode);

  const z = mod.Counter.from_zero('zero');
  assert.equal(z.describe(), 'zero:0', mode);
  z.free();
  z.free(); // idempotent
  assert.throws(() => z.increment(1), /already freed/, mode);
}
console.log('ERGONOMICS_E2E_NODE_OK');
"

if [ -x "$root/node_modules/.bin/tsc" ]; then
	echo "==> TypeScript strict nodenext declaration checks"
	cat >"$tmp/check.mts" <<'EOF'
import { filterTasks, summarizeTasks, validateTitle } from "./fs/client.mjs";
import type { SharedTaskItem, SharedFilterOptions } from "./fs/client.mjs";
import { renderMarkdown } from "./md/npm_markdown.mjs";
import type { RenderOptions, RenderOutput } from "./md/npm_markdown.mjs";
import { tallyWords, invertIds, asyncGreet, Counter } from "./v3-rel/v3_e2e.mjs";
import type { CountReport } from "./v3-rel/v3_e2e.mjs";

const opts: SharedFilterOptions = { done: true };
const tasks: SharedTaskItem[] = [{ id: 1, title: "a", done: false, priority: "High", tags: ["x"] }];
const a = filterTasks(tasks);
const b = filterTasks(tasks, opts);
const s = summarizeTasks(a);
const v: string = validateTitle("ok");

const mdOpts: RenderOptions = { heading_ids: true };
const m1: RenderOutput = renderMarkdown("# Hi");
const m2: RenderOutput = renderMarkdown("# Hi", mdOpts);

const report: CountReport = tallyWords({ rust: 1 }, ["js"]);
const total: number = report.total + report.counts["rust"];
const map: Map<number, string> = invertIds(new Map([[1, "a"]]));
const p: Promise<string> = asyncGreet("ts");
const p2: Promise<string> = asyncGreet("ts", true);
const c = new Counter(1, "lbl");
const n: number = c.increment(2) + Counter.max_value(1, 2);
const d: string = c.describe();
const z: Counter = Counter.from_zero("z");
z.free();
const moved: number = c.into_value();
console.log(b.length, s.total, v, m1.html, m2.word_count, total, map.size, p, p2, n, d, moved);
EOF
	"$root/node_modules/.bin/tsc" --noEmit --strict --module nodenext --moduleResolution nodenext "$tmp/check.mts"
	echo "ERGONOMICS_E2E_TSC_OK"
fi

echo "=== ERGONOMICS E2E PASSED ==="
