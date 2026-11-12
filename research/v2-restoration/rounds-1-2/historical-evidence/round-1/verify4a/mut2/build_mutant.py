#!/usr/bin/env python3
"""Builds a release `jsc-link` with one named mutant applied in the mut2 sandbox, copies the
binary to mut2/bin/jsc-link-<name>, and restores the sources."""
import subprocess, sys, os, shutil, importlib.util
here = os.path.dirname(os.path.abspath(__file__))
SRC = here + "/src"; ORIG = sys.argv[1]; name = sys.argv[2]
E = "jsc-link/src/analysis/effects.rs"; C = "jsc-link/src/analysis/summary_check.rs"; Wt = "jsc-link/src/opt/witness.rs"
M = {
 "div": [(E, '    "compare_bytes",\n    "sqrtf32",', '    "compare_bytes",\n    "unchecked_div",\n    "sqrtf32",')],
 "div-check": [(E, '    "compare_bytes",\n    "sqrtf32",', '    "compare_bytes",\n    "unchecked_div",\n    "sqrtf32",'),
               (C, '                            | "compare_bytes"\n', '                            | "compare_bytes"\n                            | "unchecked_div"\n')],
 "exactdiv": [(E, '    "compare_bytes",\n    "sqrtf32",', '    "compare_bytes",\n    "exact_div",\n    "sqrtf32",')],
 "exactdiv-check": [(E, '    "compare_bytes",\n    "sqrtf32",', '    "compare_bytes",\n    "exact_div",\n    "sqrtf32",'),
               (C, '                            | "compare_bytes"\n', '                            | "compare_bytes"\n                            | "exact_div"\n')],
 "table-invoke": [("rcgjs-object/src/host.rs", "                | Self::Uint8ArrayFromBytes\n        )", "                | Self::Uint8ArrayFromBytes\n                | Self::Invoke\n        )")],
 "promise": [(E, '"callback4" | "localwaker"\n        )', '"callback4" | "localwaker" | "promise" | "invoke"\n        )')],
 "try": [(E, ".is_some_and(|descriptor| !descriptor.layout.needs_drop)", ".is_some_and(|_descriptor| true)")],
 "foreign": [(E, "        Some(_) => Some(ThrowCause::Body {\n            name,\n            missing: false,\n        }),\n        None => Some(ThrowCause::Opaque(OpaqueCause::Foreign)),", "        Some(_) => None,\n        None => Some(ThrowCause::Opaque(OpaqueCause::Foreign)),")],
 # the proposed repair's cost probe: compare_bytes removed from all three lists
 "no-compare-bytes": [(E, '    "compare_bytes",\n    "sqrtf32",', '    "sqrtf32",'),
                      (C, '                            | "compare_bytes"\n', ''),
                      (Wt, '            | "saturating_sub"\n            | "compare_bytes"\n    )\n}', '            | "saturating_sub"\n    )\n}')],
}
edits = M[name]
for f, old, new in edits:
    p = os.path.join(SRC, f); s = open(p).read()
    assert s.count(old) == 1, (f, s.count(old))
    open(p, "w").write(s.replace(old, new))
r = subprocess.run(["cargo", "build", "--release", "-p", "jsc-link", "--locked"], cwd=SRC, capture_output=True, text=True)
open(here + f"/build-{name}.log", "w").write(r.stdout + r.stderr)
if r.returncode == 0:
    shutil.copyfile(SRC + "/target/release/jsc-link", here + f"/bin/jsc-link-{name}"); os.chmod(here + f"/bin/jsc-link-{name}", 0o755)
for f in sorted({f for f, _, _ in edits}):
    shutil.copyfile(os.path.join(ORIG, f), os.path.join(SRC, f))
print(name, "exit", r.returncode)
