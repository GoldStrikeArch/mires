"""Check real CGU reuse when a backend is replaced at the same path.

Usage: backend-identity-test.py WORK SYSROOT PREDECESSOR
The predecessor must predate wide-pointer metadata equality (true for this
probe). The current backend must return false. All artifacts remain in WORK.
"""

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

root = Path(__file__).resolve().parent.parent
work, sysroot, predecessor = map(lambda value: Path(value).resolve(), sys.argv[1:])
work.mkdir()
suffix = predecessor.suffix
current = root / ("target/release/librustc_codegen_mires" + suffix)
binary = work / ("librustc_codegen_mires" + suffix)
macro = work / ("libjs_extern_macro" + suffix)
shutil.copyfile(root / "target/debug" / macro.name, macro)
channel = re.search(r'channel\s*=\s*"([^"]+)"', (root / "rust-toolchain.toml").read_text())[1]
source = work / "cache.rs"
source.write_text("""#![no_std]
#![no_main]
#![deny(unsafe_code)]
use js_extern_macro::js_export;
#[js_export]
pub fn calc() -> bool {
    let values = [1_u32, 2];
    let long = &values as *const [u32; 2];
    let short = long as *const [u32; 1];
    (long as *const [u32]) == (short as *const [u32])
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }
""")
program = work / "program.mjs"
base = [
    "rustc", "+" + channel, "-Zcodegen-backend=" + str(binary),
    "--edition=2021", "--target=wasm32-unknown-unknown", "--sysroot", str(sysroot),
    "-Ccodegen-units=1", "-Cpanic=abort", "-Coverflow-checks=off",
    "-Cdebug-assertions=off", "-Cdebuginfo=0", "-Zquery-dep-graph", "-Zprint-mono-items=yes",
    "--extern", "js_extern_macro=" + str(macro),
    "--crate-type=cdylib", "--crate-name=backend_cache",
    "-Cllvm-args=js-opt=on js-minify=on js-ecma-target=2018 js-cleanup=on js-lane=baseline "
    "js-modules=esm js-shim-module=./shim.mjs",
    "-o", str(program), str(source),
]
env = dict(os.environ, JSC_LINK_BIN=str(root / "target/release/jsc-link"))
subprocess.run(["node", "scripts/make-esm-shim.mjs", str(work / "shim.mjs")], cwd=root, check=True)
(work / "run.mjs").write_text("import {calc} from './program.mjs'; console.log(JSON.stringify(calc()));\n")
rows = []


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


old_digest = digest(predecessor)
new_digest = digest(current)
assert old_digest != new_digest
stages = [
    ("old-cold", predecessor, "cache", old_digest, "true", "No"),
    ("old-warm", None, "cache", old_digest, "true", "PostLto"),
    ("new-missing", current, "cache", None, None, "requires -Cllvm-args=js-producer="),
    ("new-stale", None, "cache", old_digest, None, "does not match the backend binary"),
    ("new-duplicate", None, "cache", [new_digest, new_digest], None, "duplicate js-producer identity"),
    ("new-keyed", None, "cache", new_digest, "false", "No"),
    ("new-warm", None, "cache", new_digest, "false", "PostLto"),
    ("new-clean", None, "clean-cache", new_digest, "false", "No"),
]
for name, replacement, cache, identity, expected, reuse in stages:
    if replacement:
        shutil.copyfile(replacement, binary)
    program.unlink(missing_ok=True)
    command = base + ["-Cincremental=" + str(work / cache)]
    if identity is not None:
        command += ["-Cllvm-args=js-producer=" + value
                    for value in (identity if isinstance(identity, list) else [identity])]
    result = subprocess.run(command, cwd=root, env=env, capture_output=True, text=True)
    log = result.stdout + result.stderr
    (work / (name + ".log")).write_text(log)
    row = dict(stage=name, command=command, backendSha256=digest(binary), compileExit=result.returncode)
    rows.append(row)
    (work / "results.json").write_text(json.dumps(rows, indent=2) + "\n")
    if expected is None:
        assert result.returncode != 0 and reuse in log and not program.exists(), row
    else:
        assert result.returncode == 0, row
        actual_reuse = re.findall(r"^CGU_REUSE \S+ (\S+)$", log, re.MULTILINE)
        assert actual_reuse == [reuse], (name, actual_reuse, reuse)
        runtime = subprocess.run(["node", str(work / "run.mjs")], capture_output=True, text=True, timeout=15)
        (work / (name + ".runtime.log")).write_text(runtime.stdout + runtime.stderr)
        row.update(runtimeExit=runtime.returncode, stdout=runtime.stdout.strip(),
                   cguReuse=actual_reuse, programSha256=digest(program))
        shutil.copyfile(program, work / (name + ".mjs"))
        assert runtime.returncode == 0 and runtime.stdout.strip() == expected, row
    row["passed"] = True
    (work / "results.json").write_text(json.dumps(rows, indent=2) + "\n")
    print(name, "PASS", flush=True)
assert (work / "new-keyed.mjs").read_bytes() == (work / "new-warm.mjs").read_bytes()
assert (work / "new-keyed.mjs").read_bytes() == (work / "new-clean.mjs").read_bytes()
