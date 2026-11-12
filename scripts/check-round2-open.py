#!/usr/bin/env python3
"""Run preserved Round 2 counterexamples. Wrong answers remain failures.

Examples:
    python3 scripts/check-round2-open.py --mode both
    python3 scripts/check-round2-open.py --mode package --case large_enum_copy

Requires a built compiler. Builds only matching sysroots; never updates expectations.
Exit 0 means every selected case passed; exit 1 means a behavioral/compile failure;
exit 2 means invalid input or missing test/tool infrastructure.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
CASES = ROOT / "examples/round2-open"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def execute(command, log, env, timeout=300, stdin=None):
    try:
        result = subprocess.run(command, cwd=ROOT, env=env, input=stdin,
                                text=True, capture_output=True, timeout=timeout)
        log.write_text(result.stdout + result.stderr)
        return {"exitCode": result.returncode, "stdout": result.stdout,
                "stderr": result.stderr, "timeout": False}
    except subprocess.TimeoutExpired:
        log.write_text("Timed out; this is a failure, not an accepted outcome.\n")
        return {"exitCode": None, "stdout": "", "stderr": "timeout", "timeout": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=["release", "package", "both"], default="both")
    parser.add_argument("--case", action="append", dest="cases", default=[])
    parser.add_argument("--output", type=Path, default=ROOT / "build/round2-open")
    args = parser.parse_args()
    available = {p.stem: p for p in CASES.glob("*.rs") if p.name != "prelude.rs"}
    unknown = set(args.cases) - available.keys()
    if unknown:
        parser.error("unknown cases: " + ", ".join(sorted(unknown)))
    selected = sorted(args.cases or available)
    if not selected:
        parser.error("no cases found; an empty run is not a pass")
    for name in selected:
        if not available[name].with_suffix(".expected").is_file():
            parser.error("missing native expectation for " + name)
    modes = ["release", "package"] if args.mode == "both" else [args.mode]
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    rows, setup = [], []
    for mode in modes:
        env = dict(os.environ, JS_EXTRA_ARGS="js-profile=release-balanced",
                   SKIP_BUILD="1", JSC_RUST_MIR_INLINE="on" if mode == "package" else "off")
        # Explicitly clear inherited settings for the non-package arm.
        env.pop("JSC_RUST_OPT_LEVEL", None)
        env.pop("JSC_SYSROOT_ARGS", None)
        if mode == "package":
            env["JSC_RUST_OPT_LEVEL"] = "z"
        dest = out / mode
        dest.mkdir(exist_ok=True)
        built = execute(["bash", "scripts/build_sysroot.sh"], dest / "sysroot.log", env, 900)
        setup.append({"mode": mode, "exitCode": built["exitCode"], "timeout": built["timeout"]})
        if built["exitCode"] != 0:
            print(mode + ": matching sysroot setup failed", file=sys.stderr)
            continue
        for name in selected:
            source = available[name]
            expected = source.with_suffix(".expected").read_text()
            program = dest / (name + ".js")
            compiled = execute(["bash", "scripts/compile_core.sh", str(source), str(program)],
                               dest / (name + ".compile.log"), env)
            row = {"name": name, "mode": mode, "sourceSha256": digest(source),
                   "expectedSha256": digest(source.with_suffix(".expected")),
                   "expected": expected, "compileExit": compiled["exitCode"], "passed": False}
            if compiled["exitCode"] == 0 and program.is_file():
                text = (ROOT / "runtime/shim.js").read_text() + "\n" + program.read_text() + "\nrust_entry();\n"
                result = execute(["node", "-"], dest / (name + ".run.log"), env, 20, text)
                row.update(runExit=result["exitCode"], actual=result["stdout"], error=result["stderr"],
                           timeout=result["timeout"], programSha256=digest(program),
                           passed=result["exitCode"] == 0 and result["stdout"] == expected)
            rows.append(row)
            print(mode, name, "PASS" if row["passed"] else "OPEN", flush=True)
    complete = len(rows) == len(selected) * len(modes)
    receipt = {"schema": 1, "generatedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
               "scope": "Round 2 native-expectation counterexamples; no wrong-result goldens",
               "profile": "release-balanced", "cleanup": "on", "modes": modes,
               "setup": setup, "complete": complete, "cases": rows,
               "passed": complete and all(r["passed"] for r in rows)}
    (out / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    return 2 if not complete else (0 if receipt["passed"] else 1)


if __name__ == "__main__":
    sys.exit(main())
