#!/usr/bin/env bash
# End-to-end verification of the Mires Framework Embedding SDK:
#   - 100% stable Rust host builds (`cargo +stable test` / `cargo +stable run`)
#   - `cfg(mires)` target isolation (no `wasm32` collision, zero `unexpected_cfgs` warnings)
#   - Cross-crate `#[js_opaque]` & `#[js_opaque(ts = "...")]` + `HostHandle`
#   - Attribute-discovered chunks (`#[js_export(chunk = "...", role = "island")]`) + shared chunk
#   - `#[js_export(rename_all = "camelCase")]`, `#[js_export(skip)]`, safe `async` class methods
#   - Typed `mires::callback(...)` closures
#   - Rust `///` doc comments -> `.d.mts` JSDoc, short TS type names, `[Symbol.dispose](): void`
#   - `cargo-mires build --message-format=json --cfg ...`
#   - `mires_build::Crate` + `is_target_build()` + workspace dep tracking + `emit_rust_module()`

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
cargo_mires="$root/target/release/cargo-mires"

if [ ! -x "$cargo_mires" ]; then
	echo "framework-sdk-e2e.sh: missing $cargo_mires (run cargo build --release -p cargo-mires)" >&2
	exit 1
fi

tsc_bin=""
if [ -x "$root/node_modules/.bin/tsc" ]; then
	tsc_bin="$root/node_modules/.bin/tsc"
elif [ -x "$root/../../../node_modules/.bin/tsc" ]; then
	tsc_bin="$root/../../../node_modules/.bin/tsc"
fi

tmp=$(mktemp -d)
cleanup() {
	rm -rf "$tmp"
}
trap cleanup EXIT

ws="$tmp/fw-workspace"
mkdir -p \
	"$ws/shared/src" \
	"$ws/fw-dom/src" \
	"$ws/islands/src" \
	"$ws/server/src" \
	"$ws/store-lib/src"

cat >"$ws/Cargo.toml" <<'EOF'
[workspace]
resolver = "2"
members = ["shared", "fw-dom", "islands", "server", "store-lib"]
EOF

cat >"$ws/shared/Cargo.toml" <<'EOF'
[package]
name = "shared"
version = "0.1.0"
edition = "2021"
publish = false
EOF

cat >"$ws/shared/src/lib.rs" <<'EOF'
#![forbid(unsafe_code)]

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadgeData {
    pub label: String,
    pub count: i32,
}

pub fn format_badge(label: &str, count: i32) -> String {
    format!("[{label}:{count}]")
}
EOF

cat >"$ws/fw-dom/Cargo.toml" <<EOF
[package]
name = "fw-dom"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
mires = { path = "$root/mires" }
EOF

cat >"$ws/fw-dom/src/lib.rs" <<'EOF'
#![forbid(unsafe_code)]
#![deny(warnings)]

use mires::prelude::*;
use mires::HostHandle;

/// Explicit TypeScript spelling for an external framework element handle.
#[js_opaque(ts = "HTMLElement")]
#[derive(Clone, Copy)]
pub struct DomNode(pub HostHandle);

/// Branded opaque handle (empty TS spelling -> unique symbol type alias).
#[js_opaque]
#[derive(Clone, Copy)]
pub struct ReactiveOwner(pub HostHandle);

js_bindings! {
    #[js(call = "String")]
    pub fn to_js_string(value: JsValue) -> JsString;

    #[js(get = "tagName")]
    pub fn node_tag(node: DomNode) -> JsString;
}
EOF

cat >"$ws/islands/Cargo.toml" <<EOF
[package]
name = "islands"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
shared = { path = "../shared" }
fw-dom = { path = "../fw-dom" }
mires = { path = "$root/mires" }

[lints.rust]
unexpected_cfgs = { level = "warn", check-cfg = ['cfg(topcoat_client)'] }
EOF

cat >"$ws/islands/src/lib.rs" <<'EOF'
#![deny(warnings)]

use fw_dom::{DomNode, ReactiveOwner};
use mires::prelude::*;
use shared::{format_badge, BadgeData};

/// Mounts the counter island and formats a badge using shared logic.
#[js_export(chunk = "counter", role = "island", rename_all = "camelCase")]
pub fn mount_counter(root: DomNode, data: BadgeData) -> (DomNode, String) {
    let prefix = {
        #[cfg(topcoat_client)]
        {
            "client:"
        }
        #[cfg(not(topcoat_client))]
        {
            "server:"
        }
    };
    let text = format!("{prefix}{}", format_badge(&data.label, data.count + 1));
    (root, text)
}

/// Mounts the search island, sharing `format_badge` with `counter` so `shared.js` is emitted.
#[js_export(chunk = "search", role = "island", rename_all = "camelCase")]
pub fn mount_search(owner: ReactiveOwner, query: &str, hits: i32) -> (ReactiveOwner, String) {
    let text = format_badge(query, hits);
    (owner, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_badge_logic_runs_on_stable_host() {
        assert_eq!(format_badge("ok", 3), "[ok:3]");
    }
}
EOF

cat >"$ws/store-lib/Cargo.toml" <<EOF
[package]
name = "store-lib"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
mires = { path = "$root/mires" }
shared = { path = "../shared" }
EOF

cat >"$ws/store-lib/src/lib.rs" <<'EOF'
#![deny(warnings)]

use mires::prelude::*;
use shared::{format_badge, BadgeData};

/// Builds a typed JS adder callback using `mires::callback`.
#[js_export(rename_all = "camelCase")]
pub fn make_adder(offset: i32) -> JsValue {
    callback(move |x: i32| x + offset)
}

/// Stateful store exported as a camelCase ES6 class with skip + async methods.
#[js_export]
pub struct Store {
    label: String,
    count: i32,
}

#[js_export(rename_all = "camelCase")]
impl Store {
    /// Creates a new Store.
    pub fn new(initial: BadgeData) -> Self {
        Self {
            label: initial.label,
            count: initial.count,
        }
    }

    /// Increments the store counter and returns the formatted badge.
    pub fn add_item(&mut self, delta: i32) -> String {
        self.count += delta;
        format_badge(&self.label, self.count)
    }

    #[js_export(skip)]
    pub fn rust_only_secret(&self) -> i32 {
        self.count * 999
    }

    /// Asynchronously fetches a formatted banner string.
    pub async fn fetch_banner(prefix: &str) -> String {
        let _ = Promise::resolve(JsValue::erase(1_i32)).await;
        format!("{prefix}:ready")
    }

    /// Consumes the store asynchronously and returns the final summary.
    pub async fn into_final_summary(self) -> String {
        let _ = Promise::resolve(JsValue::erase(2_i32)).await;
        format_badge(&self.label, self.count)
    }
}
EOF

cat >"$ws/server/Cargo.toml" <<EOF
[package]
name = "server"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
shared = { path = "../shared" }

[build-dependencies]
mires-build = { path = "$root/mires-build" }
EOF

cat >"$ws/server/build.rs" <<'EOF'
use mires_build::DiscoverChunks;

fn main() {
    if mires_build::is_target_build() {
        return;
    }
    let artifacts = mires_build::Crate::new("../islands")
        .release()
        .cfg("topcoat_client")
        .runtime(mires_build::RuntimeMode::External)
        .discover_chunks(DiscoverChunks::On)
        .emit_rust_module("islands_assets.rs")
        .build()
        .expect("compile islands via mires-build");

    assert!(artifacts.program.is_file(), "primary program missing");
    assert!(artifacts.manifest_path.as_ref().is_some_and(|p| p.is_file()));
    assert!(
        artifacts.chunks.len() >= 2,
        "expected >= 2 discovered chunks, got {:?}",
        artifacts.chunks
    );
}
EOF

cat >"$ws/server/src/main.rs" <<'EOF'
mod islands_assets {
    include!(concat!(env!("OUT_DIR"), "/islands_assets.rs"));
}

fn main() {
    assert!(!islands_assets::ENTRY_JS.is_empty());
    assert_eq!(islands_assets::ENTRY_SHA256.len(), 64);
    assert!(islands_assets::MANIFEST_JSON.is_some());

    let chunk_names: Vec<&str> = islands_assets::CHUNKS.iter().map(|c| c.name).collect();
    assert!(chunk_names.contains(&"counter"), "missing counter chunk: {chunk_names:?}");
    assert!(chunk_names.contains(&"search"), "missing search chunk: {chunk_names:?}");
    let counter = islands_assets::CHUNKS.iter().find(|c| c.name == "counter").unwrap();
    assert_eq!(counter.role, "island");
    assert!(!counter.bytes.is_empty());
    assert_eq!(counter.sha256.len(), 64);
    println!("SERVER_EMBEDDED_CHUNKS_OK: {:?}", chunk_names);
}
EOF

echo "==> 1. Stable Rust workspace tests (zero nightly features, zero warnings)"
RUSTFLAGS="-D warnings" cargo +stable test \
	--manifest-path "$ws/Cargo.toml" \
	-p shared -p fw-dom -p islands -p store-lib \
	--target-dir "$tmp/stable-host"

echo "==> 2. Stable Rust server build.rs embedding islands via mires-build + cargo-mires JSON"
MIRES_ROOT="$root" cargo +stable run \
	--manifest-path "$ws/server/Cargo.toml" \
	--target-dir "$tmp/stable-server"

echo "==> 3. cargo-mires --message-format=json on islands (discovered chunks + --cfg topcoat_client)"
json_out=$("$cargo_mires" build \
	--manifest-path "$ws/Cargo.toml" \
	-p islands \
	--release \
	--runtime external \
	--cfg topcoat_client \
	--discover-chunks on \
	--message-format json \
	--out-dir "$tmp/islands-out" \
	--target-dir "$tmp/mires-target")

echo "==> 4. cargo-mires --message-format=json on store-lib (camelCase, skip, async methods, callback)"
store_json=$("$cargo_mires" build \
	--manifest-path "$ws/Cargo.toml" \
	-p store-lib \
	--release \
	--message-format json \
	--out-dir "$tmp/store-out" \
	--target-dir "$tmp/mires-target")

echo "==> 5. Node.js runtime + JSON manifest + .d.mts verification"
JSON_OUT="$json_out" STORE_JSON="$store_json" ISLANDS_OUT="$tmp/islands-out" STORE_OUT="$tmp/store-out" node --input-type=module -e '
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const islandsBuild = JSON.parse(process.env.JSON_OUT);
assert.equal(islandsBuild.schema, 1);
assert.equal(islandsBuild.package, "islands");
assert.ok(islandsBuild.manifest && islandsBuild.manifest.entries, "embedded manifest missing");
assert.ok(
  islandsBuild.dep_files.some((p) => p.endsWith(path.join("shared", "src", "lib.rs"))),
  "dep_files must track workspace path dependency shared/src/lib.rs: " + JSON.stringify(islandsBuild.dep_files)
);
assert.ok(
  islandsBuild.dep_files.some((p) => p.endsWith(path.join("fw-dom", "src", "lib.rs"))),
  "dep_files must track workspace path dependency fw-dom/src/lib.rs"
);

const chunkNames = islandsBuild.chunks.map((c) => c.name);
assert.ok(chunkNames.includes("counter"), "counter chunk missing: " + JSON.stringify(chunkNames));
assert.ok(chunkNames.includes("search"), "search chunk missing: " + JSON.stringify(chunkNames));
const counterChunk = islandsBuild.chunks.find((c) => c.name === "counter");
assert.equal(counterChunk.role, "island");

// Execute auto-discovered island chunks directly in Node ESM!
const counterMod = await import(counterChunk.path);
const fakeEl = { tagName: "DIV" };
const [returnedEl, counterText] = counterMod.mountCounter(fakeEl, { label: "clicks", count: 4 });
assert.strictEqual(returnedEl, fakeEl, "#[js_opaque] DomNode must pass through by JS identity");
assert.equal(counterText, "client:[clicks:5]", "--cfg topcoat_client must be active");

const searchChunk = islandsBuild.chunks.find((c) => c.name === "search");
const searchMod = await import(searchChunk.path);
const fakeOwner = { id: 7 };
const [returnedOwner, searchText] = searchMod.mountSearch(fakeOwner, "rust", 12);
assert.strictEqual(returnedOwner, fakeOwner, "#[js_opaque] ReactiveOwner must pass through by JS identity");
assert.equal(searchText, "[rust:12]");

// Check .d.mts contents on islands & store-lib
const islandsDts = fs.readFileSync(islandsBuild.dts, "utf8");
assert.ok(islandsDts.includes("HTMLElement"), "explicit #[js_opaque(ts = \"HTMLElement\")] missing:\n" + islandsDts);
assert.ok(islandsDts.includes("unique symbol"), "branded #[js_opaque] unique symbol missing:\n" + islandsDts);
assert.ok(islandsDts.includes("export type BadgeData ="), "short type name BadgeData missing:\n" + islandsDts);
assert.ok(!islandsDts.includes("SharedBadgeData"), "legacy full-path SharedBadgeData leaked:\n" + islandsDts);
assert.ok(islandsDts.includes("Mounts the counter island"), "JSDoc from /// missing:\n" + islandsDts);

// Execute store-lib (typed callback, camelCase class, skip, async static & consuming methods, Symbol.dispose)
const storeBuild = JSON.parse(process.env.STORE_JSON);
const storeMod = await import(storeBuild.program);
const add10 = storeMod.makeAdder(10);
assert.equal(typeof add10, "function");
assert.equal(add10(32), 42);

const s = new storeMod.Store({ label: "items", count: 2 });
assert.equal(s.addItem(3), "[items:5]");
assert.equal(typeof s.rustOnlySecret, "undefined", "#[js_export(skip)] method must not be exported");
assert.equal(typeof s.rust_only_secret, "undefined", "#[js_export(skip)] method must not be exported");
assert.equal(await storeMod.Store.fetchBanner("v1"), "v1:ready");
assert.equal(await s.intoFinalSummary(), "[items:5]");
assert.throws(() => s.addItem(1), /already freed/);

const s2 = new storeMod.Store({ label: "disp", count: 1 });
if (typeof Symbol.dispose !== "undefined") {
  s2[Symbol.dispose]();
  assert.throws(() => s2.addItem(1), /already freed/);
}

const storeDts = fs.readFileSync(storeBuild.dts, "utf8");
assert.ok(storeDts.includes("__MiresDisposableBase"), "conditional Symbol.dispose base missing from .d.mts:\n" + storeDts);
assert.ok(storeDts.includes("export declare class Store extends __MiresDisposableBase"), "Store must extend __MiresDisposableBase:\n" + storeDts);
assert.ok(storeDts.includes("addItem("), "camelCase method addItem missing:\n" + storeDts);
assert.ok(storeDts.includes("static fetchBanner("), "async static method missing:\n" + storeDts);
assert.ok(storeDts.includes("intoFinalSummary(): Promise<string>;"), "async consuming method missing:\n" + storeDts);
assert.ok(!storeDts.includes("rustOnlySecret") && !storeDts.includes("rust_only_secret"), "skipped method leaked into .d.mts:\n" + storeDts);
assert.ok(storeDts.includes("Increments the store counter"), "method JSDoc missing:\n" + storeDts);

console.log("FRAMEWORK_SDK_E2E_NODE_OK");
'

if [ -n "$tsc_bin" ]; then
	echo "==> 6. TypeScript strict nodenext verification ($tsc_bin)"
	cat >"$tmp/check.mts" <<'EOF'
import { mountCounter, mountSearch } from "./islands-out/islands.mjs";
import type { BadgeData, ReactiveOwner } from "./islands-out/islands.mjs";
import { makeAdder, Store } from "./store-out/store_lib.mjs";

const data: BadgeData = { label: "ts", count: 1 };
declare const el: HTMLElement;
declare const owner: ReactiveOwner;

const [el2, txt]: [HTMLElement, string] = mountCounter(el, data);
const [owner2, q]: [ReactiveOwner, string] = mountSearch(owner, "ok", 2);

const fn: unknown = makeAdder(5);
const store = new Store(data);
const badge: string = store.addItem(1);
const banner: Promise<string> = Store.fetchBanner("hi");
const summary: Promise<string> = store.intoFinalSummary();
store.free();
console.log(el2, txt, owner2, q, fn, badge, banner, summary);
EOF
	"$tsc_bin" --noEmit --strict --lib es2022,dom --module nodenext --moduleResolution nodenext "$tmp/check.mts"

	# Also verify TS 5.2+ esnext.disposable compatibility (Store satisfies Disposable & [Symbol.dispose]()).
	cat >"$tmp/disposable.d.ts" <<'EOF'
interface SymbolConstructor {
  readonly dispose: unique symbol;
}
interface Disposable {
  [Symbol.dispose](): void;
}
EOF
	cat >"$tmp/check-disposable.mts" <<'EOF'
import { Store } from "./store-out/store_lib.mjs";
const s = new Store({ label: "disp", count: 0 });
const d: Disposable = s;
s[Symbol.dispose]();
console.log(d);
EOF
	"$tsc_bin" --noEmit --strict --lib es2022,dom --module nodenext --moduleResolution nodenext "$tmp/disposable.d.ts" "$tmp/check-disposable.mts"
	echo "FRAMEWORK_SDK_E2E_TSC_OK"
else
	echo "warning: tsc not found; skipped TypeScript check" >&2
fi

echo "=== FRAMEWORK SDK E2E PASSED ==="
