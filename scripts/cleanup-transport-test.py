"""Verify generic cleanup transport, incremental edits, and producer identity.

Usage: cleanup-transport-test.py WORK SYSROOT off|report|on
The sysroot must have matching cleanup-enabled, minified ES2018 emit options.
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
flags = f"js-opt={mode} js-minify=on js-ecma-target=2018 js-cleanup=on"
base = [
    "rustc", "+" + channel, "-Zcodegen-backend=" + str(backend),
    "--edition=2021", "--target=wasm32-unknown-unknown", "--sysroot", str(sysroot),
    "-Ccodegen-units=1", "-Cpanic=abort", "-Coverflow-checks=off",
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
start = original.index(magic)
assert original.find(magic, start + 1) == -1
offset = 8
end = None
while offset < len(original):
    size = int(original[offset + 48:offset + 58])
    data = offset + 60
    if data <= start < data + size:
        end = data + size
        break
    offset = data + size + size % 2
assert end is not None
payload_start = start + len(magic) + 32
for name, error in [
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
(work / "result.json").write_text(json.dumps(dict(
    passed=True, mode=mode, runtimeCases=141, producerIdentityChecked=True,
    coldWarmCguReuse=True, changedWarmCguReuse=True, removedSourceMapPreserved=True,
    runtimeNamespacePrepared=True,
    backendSha256=hashlib.sha256(backend.read_bytes()).hexdigest(),
), indent=2) + "\n")
