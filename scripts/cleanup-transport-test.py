"""Verify generic cleanup transport, incremental edits, and producer identity.

Usage: cleanup-transport-test.py [WORK SYSROOT off|report|on]
The sysroot must have matching cleanup-enabled, minified ES2018 emit options. Without arguments the
test runs `on` in build/cleanup-transport against the `js-profile=release-balanced` sysroot, which
it builds when missing (the backend itself must already be built).

The metadata cases compile a crate against its dependency's `.rmeta` alone, the way cargo compiles
the sysroot's `alloc` against `core`: its copy of the dependency's generic must still come from the
cleanup MIR in the rlib beside that `.rmeta`, and without that rlib the compilation must fail.
`guarded::<fn() -> u32>` is instantiated in two crates, so two objects carry a copy and the kept
copy must have cleanup edges. The sysroot's own `alloc` copy of `core::iter::adapters::try_process`
must have them too.

Under JSC_PANIC=unwind (the unwind build mode, scripts/sysroot-cache.sh) every crate is compiled for
targets/mires32-unwind.json with -Cpanic=unwind and js-cleanup=off: rustc's optimized MIR, which
every rlib and `.rmeta` carries, keeps the unwind edges, and no cleanup MIR member exists. The rows
that test that member (its producer identity, checksum and version, its import into a metadata-only
consumer, and the refusal when its rlib is missing) are skipped there with their reason printed;
every row that compiles, links or runs a program still runs.
"""

import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys

root = Path(__file__).resolve().parent.parent
panic_mode = os.environ.get("JSC_PANIC", "") or "unwind"
assert panic_mode in ["abort", "unwind"], "JSC_PANIC must be unset, abort or unwind"
unwind = panic_mode == "unwind"
skipped = []


def skip(name, reason):
    skipped.append(dict(name=name, reason=reason))
    print(name, "SKIP", reason, flush=True)


NO_MEMBER = "JSC_PANIC=unwind: no cleanup MIR member exists (js-cleanup=off; rustc's optimized MIR " \
            "in the rlib and .rmeta carries the unwind edges)"
if len(sys.argv) == 1:
    work = root / "build/cleanup-transport"
    shutil.rmtree(work, ignore_errors=True)
    subprocess.run(["cargo", "build", "-p", "js-extern-macro", "--locked"], cwd=root, check=True,
                   capture_output=True)
    built = subprocess.run(["bash", "scripts/build_sysroot.sh"], cwd=root, check=True,
                           capture_output=True, text=True,
                           env=dict(os.environ, SKIP_BUILD="1",
                                    JS_EXTRA_ARGS="js-profile=release-balanced"))
    sysroot = Path(re.findall(r"^==> (?:reusing )?sysroot: (.+)$", built.stdout, re.MULTILINE)[-1])
    mode = "on"
else:
    work, sysroot = (Path(value).resolve() for value in sys.argv[1:3])
    mode = sys.argv[3]
assert mode in ["off", "report", "on"]
work.mkdir()
suffix = ".dylib" if platform.system() == "Darwin" else ".so"
backend = root / ("target/release/librustc_codegen_mires" + suffix)
macro = work / "proc-macro" / ("libjs_extern_macro" + suffix)
macro.parent.mkdir()
shutil.copyfile(root / "target/debug" / macro.name, macro)
library = work / "libmires_js.rlib"
channel = re.search(r'channel\s*=\s*"([^"]+)"', (root / "rust-toolchain.toml").read_text())[1]
producer = work / "provider.rs"
client = work / "client.rs"
for target, name in [(producer, "cleanup_provider.rs"), (client, "cleanup_client.rs")]:
    shutil.copyfile(root / "examples/cross-crate-tests" / name, target)
original_source = producer.read_text()
subprocess.run(["node", "scripts/make-esm-shim.mjs", str(work / "shim.mjs")], cwd=root, check=True)
checker = root / "scripts/active-poll-cleanup-check.mjs"
changed_checker = work / "changed-check.mjs"
changed_checker.write_text(re.sub(r"\b80\b", "180", checker.read_text()))
flags = f"js-opt={mode} js-minify=on js-ecma-target=2018 js-cleanup={'off' if unwind else 'on'}"
target = ["--target=" + str(root / "targets/mires32-unwind.json"), "-Zunstable-options"] if unwind \
    else ["--target=wasm32-unknown-unknown"]
base = [
    "rustc", "+" + channel, "-Zcodegen-backend=" + str(backend),
    "--edition=2021", *target, "--sysroot", str(sysroot),
    "-Ccodegen-units=1", "-Cpanic=unwind" if unwind else "-Cpanic=abort", "-Coverflow-checks=off",
    "-Cdebug-assertions=off", "-Cdebuginfo=0", "-Zquery-dep-graph", "-Zprint-mono-items=yes",
    "-Cllvm-args=js-producer=" + hashlib.sha256(backend.read_bytes()).hexdigest(),
    "--remap-path-prefix", str(root) + "/=",
]
env = dict(os.environ, JSC_LINK_BIN=str(root / "target/release/jsc-link"), JSC_JIR_SELFCHECK="1")
archive = work / "libcleanup_provider.rlib"
program = work / "program.mjs"
dependencies = ["--extern", "mires_js=" + str(library),
                "-Ldependency=" + str(work), "-Ldependency=" + str(macro.parent)]
provider_command = base + dependencies + [
    "--crate-type=rlib", "--crate-name=cleanup_provider", "-Cllvm-args=" + flags,
    "-Cincremental=" + str(work / "provider-cache"), "-o", str(archive), str(producer),
]
client_command = base + dependencies + [
    "--crate-type=cdylib", "--crate-name=cleanup_client", "--extern", "cleanup_provider=" + str(archive),
    "-Cllvm-args=" + flags + " js-lane=jir js-modules=esm js-shim-module=./shim.mjs js-source-map=on",
    "-o", str(program), str(client),
]
rows = []


def run(name, command, error=None):
    result = subprocess.run(command, cwd=root, env=env, capture_output=True, text=True, timeout=180)
    log = result.stdout + result.stderr
    (work / (name + ".log")).write_text(log)
    row = dict(name=name, command=command, exitCode=result.returncode,
               cguReuse=re.findall(r"^CGU_REUSE \S+ (\S+)$", log, re.MULTILINE))
    rows.append(row)
    (work / "commands.json").write_text(json.dumps(rows, indent=2) + "\n")
    if error is None:
        assert result.returncode == 0, row
    else:
        assert result.returncode != 0 and error in log and "compiler unexpectedly panicked" not in log, row
    print(name, "PASS", flush=True)
    return row


def check(name, changed=False):
    run(name + "-runtime", ["node", str(changed_checker if changed else checker), str(program)])
    report = json.loads(program.with_suffix(".mjs.lane.json").read_text())
    assert report["fallback_count"] == 0
    run(name + "-map", ["node", "scripts/check-sourcemap.js", str(program)])
    for extension in [".mjs", ".mjs.map", ".mjs.lane.json"]:
        shutil.copyfile(program.with_suffix(extension), work / (name + extension))


run("library", base + ["--crate-type=rlib", "--crate-name=mires_js",
                      "--extern", "js_extern_macro=" + str(macro), "-Cllvm-args=" + flags,
                      "-o", str(library), str(root / "mires-js/src/lib.rs")])
for stage in ["cold", "warm", "changed", "changed-warm"]:
    if stage == "changed":
        assert "Guard(80)" in original_source
        producer.write_text(original_source.replace("Guard(80)", "Guard(180)"))
    provider_row = run(stage + "-provider", provider_command)
    shutil.copyfile(archive, work / (stage + "-provider.rlib"))
    client_row = run(stage + "-client", client_command + ["-Cincremental=" + str(work / "client-cache")])
    check(stage, stage.startswith("changed"))
    if stage in ["warm", "changed-warm"]:
        assert provider_row["cguReuse"] == ["PostLto"], provider_row
        assert client_row["cguReuse"] == ["PostLto"], client_row
        # A cached consumer could hide missing executable MIR in the warm provider archive.
        # Compile it independently too, without an incremental cache.
        run(stage + "-relinked", client_command)
        check(stage + "-relinked", stage.startswith("changed"))

original = (work / "cold-provider.rlib").read_bytes()
magic = b"RCGJS-MIR-3\0"
if unwind:
    assert magic not in original, "unwind mode must not write a cleanup MIR member"
    for name in ["producer-mismatch", "checksum-mismatch", "old-version"]:
        skip(name, NO_MEMBER)
start = -1 if unwind else original.index(magic)
assert unwind or original.find(magic, start + 1) == -1
offset = 8
end = None
while not unwind and offset < len(original):
    size = int(original[offset + 48:offset + 58])
    data = offset + 60
    if data <= start < data + size:
        end = data + size
        break
    offset = data + size + size % 2
assert unwind or end is not None
payload_start = start + len(magic) + 32
for name, error in [] if unwind else [
    ("producer-mismatch", "cleanup MIR archive producer binary identity mismatch"),
    ("checksum-mismatch", "cleanup MIR archive checksum mismatch"),
    ("old-version", "invalid cleanup MIR archive header or size"),
]:
    data = bytearray(original)
    if name == "old-version":
        data[start + len(magic) - 2] = ord("2")
    else:
        data[payload_start] ^= 1
        if name == "producer-mismatch":
            data[start + len(magic):payload_start] = hashlib.sha256(data[payload_start:end]).digest()
    archive.write_bytes(data)
    shutil.copyfile(archive, work / (name + ".rlib"))
    program.unlink(missing_ok=True)
    run(name, client_command, error)
    assert not program.exists()

archive.write_bytes(original)
producer.rename(work / "removed-provider.rs")
run("source-absent", client_command)
check("source-absent")
source_map = json.loads(program.with_suffix(".mjs.map").read_text())
assert original_source in source_map["sourcesContent"]
assert (work / "cold.mjs").read_bytes() == (work / "warm.mjs").read_bytes()
assert (work / "changed.mjs").read_bytes() == (work / "changed-warm.mjs").read_bytes()
client.write_text("""#![no_std]
#![no_main]
#![deny(unsafe_code)]
use mires_js::js_export;
#[js_export]
pub fn calc() -> u32 { cleanup_provider::identity(42) }
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }
""")
run("runtime-only-client", client_command)
run("runtime-only-execute", ["node", "--input-type=module", "-e",
    "import {calc} from " + json.dumps(program.as_uri()) +
    "; if (calc() !== 42) throw Error('wrong result'); console.log(42);"])
assert json.loads(program.with_suffix(".mjs.lane.json").read_text())["fallback_count"] == 0

metadata = work / "metadata"
metadata.mkdir()
fixtures = root / "examples/cross-crate-tests"
# Each crate instantiates the generics it uses, as at the optimization levels the packages build
# with, so `guarded::<fn() -> u32>` has a copy in both the middle crate and the client.
separate = base + ["-Zshare-generics=no", "-Cllvm-args=" + flags]
run("metadata-provider", separate + [
    "--crate-type=rlib", "--crate-name=cleanup_metadata_provider", "--emit=metadata,link",
    "--out-dir", str(metadata), str(fixtures / "cleanup_metadata_provider.rs")])


def middle(name, provider, error=None):
    return run(name, separate + [
        "--crate-type=rlib", "--crate-name=cleanup_metadata_middle",
        "--extern", "cleanup_metadata_provider=" + str(provider / "libcleanup_metadata_provider.rmeta"),
        "-Ldependency=" + str(provider), "-o", str(provider / "libcleanup_metadata_middle.rlib"),
        str(fixtures / "cleanup_metadata_middle.rs")], error)


middle("metadata-middle", metadata)
if unwind:
    skip("metadata-middle-import", NO_MEMBER)
else:
    assert "cleanup-mir-import: " in (work / "metadata-middle.log").read_text()
metadata_program = metadata / "program.js"
lane_report = metadata / "lane.json"
run("metadata-client", separate + [
    "--crate-type=cdylib", "--crate-name=cleanup_metadata_client",
    "--extern", "cleanup_metadata_middle=" + str(metadata / "libcleanup_metadata_middle.rlib"),
    "--extern", "cleanup_metadata_provider=" + str(metadata / "libcleanup_metadata_provider.rlib"),
    "-Ldependency=" + str(metadata), "-Cllvm-args=js-lane=jir js-lane-report=" + str(lane_report),
    "-o", str(metadata_program), str(fixtures / "cleanup_metadata_client.rs")])
run("metadata-execute", ["node", "-e", r"""
const fs = require('node:fs')
const vm = require('node:vm')
const [program, shim] = process.argv.slice(1)
const context = vm.createContext({ TextDecoder, TextEncoder })
vm.runInContext(fs.readFileSync(shim, 'utf8'), context)
const drops = []
context.__rt.js_log_f64 = (value) => drops.push(Number(value))
const marker = new Error('host marker')
context.__rt.host_throw = () => { throw marker }
vm.runInContext(fs.readFileSync(program, 'utf8'), context)
const cases = [
  ['single copy', 7, 12, (throwing) => context.metadata_host_throw(throwing)],
  ['middle copy call', 9, 14, (throwing) => context.shared_host_throw(throwing, false)],
  ['client copy call', 11, 16, (throwing) => context.shared_host_throw(throwing, true)],
]
for (const [name, guard, value, call] of cases) {
  drops.length = 0
  const result = call(false)
  if (result !== value || JSON.stringify(drops) !== JSON.stringify([guard])) {
    throw new Error(`${name}: returned ${result}, dropped ${JSON.stringify(drops)}`)
  }
  drops.length = 0
  let caught
  try { call(true) } catch (error) { caught = error }
  if (caught !== marker || JSON.stringify(drops) !== JSON.stringify([guard])) {
    throw new Error(`${name} host throw: caught ${caught === marker ? 'the marker' : String(caught)}, dropped ${JSON.stringify(drops)}`)
  }
}
console.log('metadata transport: destructors run on host throws')
""", str(metadata_program), str(root / "runtime/shim.js")])
rows_by_prefix = {}
for row in json.loads(lane_report.read_text())["items"]:
    for prefix in ["cleanup_metadata_provider$guarded$", "core$iter$adapters$try_process$"]:
        if row["name"].startswith(prefix):
            rows_by_prefix.setdefault(prefix, []).append(row)
assert len(rows_by_prefix.get("cleanup_metadata_provider$guarded$", [])) == 2, rows_by_prefix
assert rows_by_prefix.get("core$iter$adapters$try_process$"), rows_by_prefix
for rows in rows_by_prefix.values():
    for row in rows:
        assert "unwind" in row["class"], row
print("metadata-lane-report PASS", flush=True)
metadata_only = work / "metadata-only"
metadata_only.mkdir()
shutil.copyfile(metadata / "libcleanup_metadata_provider.rmeta",
                metadata_only / "libcleanup_metadata_provider.rmeta")
if unwind:
    skip("metadata-rlib-absent", NO_MEMBER + "; the .rmeta alone is a complete dependency")
else:
    middle("metadata-rlib-absent", metadata_only,
           "cleanup MIR of `cleanup_metadata_provider` is unavailable")

(work / "result.json").write_text(json.dumps(dict(
    passed=True, mode=mode, panic=panic_mode, skipped=skipped, runtimeCases=141,
    producerIdentityChecked=not unwind,
    coldWarmCguReuse=True, changedWarmCguReuse=True, removedSourceMapPreserved=True,
    runtimeNamespacePrepared=True, metadataOnlyDependencyImported=True,
    metadataOnlyDependencyWithoutRlibRefused=not unwind, duplicateCopiesKeepCleanup=True,
    allocCoreGenericCarriesCleanup=True,
    backendSha256=hashlib.sha256(backend.read_bytes()).hexdigest(),
), indent=2) + "\n")
