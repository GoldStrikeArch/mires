#!/usr/bin/env python3
"""Execute conversation-reconstructed later regressions without accepting wrong answers."""
import hashlib, json, os, subprocess, sys
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "build/known-gaps"
OUT.mkdir(parents=True, exist_ok=True)
env = dict(os.environ, JS_EXTRA_ARGS="js-profile=release-balanced", SKIP_BUILD="1")
rows = []
with (OUT / "sysroot.log").open("w") as log:
    setup = subprocess.run(["bash", "scripts/build_sysroot.sh"], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
if setup.returncode:
    sys.exit(setup.returncode)
for source in sorted((ROOT / "examples/known-gaps").glob("*.rs")):
    if source.name == "prelude.rs":
        continue
    expected = source.with_suffix(".expected").read_text()
    program = OUT / (source.stem + ".js")
    row = {"name": source.stem, "sourceSha256": hashlib.sha256(source.read_bytes()).hexdigest(), "expected": expected}
    with (OUT / (source.stem + ".compile.log")).open("w") as log:
        result = subprocess.run(["bash", "scripts/compile_core.sh", str(source), str(program)], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
    row["compileExit"] = result.returncode
    if result.returncode == 0:
        try:
            text = (ROOT / "runtime/shim.js").read_text() + "\n" + program.read_text() + "\nrust_entry();\n"
            result = subprocess.run(["node", "-"], input=text, text=True, capture_output=True, timeout=20)
            row.update(runExit=result.returncode, actual=result.stdout, error=result.stderr)
            row["passed"] = result.returncode == 0 and result.stdout == expected
        except subprocess.TimeoutExpired:
            row.update(passed=False, timeout=True)
    else:
        row["passed"] = False
    rows.append(row)
    print(source.stem, "PASS" if row["passed"] else "OPEN", flush=True)
receipt = {"schema": 1, "scope": "later regression targets, strict native expectations", "profile": "release-balanced", "cleanup": "on", "passed": all(r["passed"] for r in rows), "cases": rows}
(OUT / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
sys.exit(0 if receipt["passed"] else 1)
