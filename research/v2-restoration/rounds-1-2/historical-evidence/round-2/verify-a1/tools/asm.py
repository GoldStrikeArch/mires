#!/usr/bin/env python3
"""asm.py: fam/hdr.txt + fam/body/*.rs -> fam/src/*.rs (plus prelude.rs)."""
import os, shutil, sys
V = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
W = "/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae6e006621ef540c6"
hdr = open(V + "/fam/hdr.txt").read()
os.makedirs(V + "/fam/src", exist_ok=True)
shutil.copy(W + "/examples/core-tests/prelude.rs", V + "/fam/src/prelude.rs")
for f in sorted(os.listdir(V + "/fam/body")):
    if f.endswith(".rs"):
        body = open(V + "/fam/body/" + f).read()
        if body.startswith("//!raw"):
            open(V + "/fam/src/" + f, "w").write(body)
        else:
            open(V + "/fam/src/" + f, "w").write(hdr + body)
        print(f)
