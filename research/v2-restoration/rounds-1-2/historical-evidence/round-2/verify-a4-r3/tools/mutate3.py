#!/usr/bin/env python3
"""Round-3 verifier mutation controls.

Usage: mutate3.py <id>...   ("all" for every one)

Each mutant is applied to the verifier worktree (which must be at e2245df and clean in the mutated
files), the compiler is rebuilt, its two binaries are copied to mut-bin/<id>/release, the edit is
reverted, sysroots are built for the mutated backend (a linker-only mutant reuses the frozen
backend and its sysroots), and the committed fixtures 291, 292, 249, 247, 241 and 26_async are
compiled and run through a probe root in def, jiroff, rb and rbz. Every (fixture, mode) that fails
is recorded, so the report can say which modes kill a mutant: the COMMON gate runs fixtures in
release-balanced only. Interpreter mutants run the rcgjs-object unit tests instead.
Results: mutants3.log, mutants3.jsonl; per-test output in out/mut3/<id>/.
"""
import json, os, shutil, subprocess, sys
from pathlib import Path

V = Path("/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3")
W = Path("/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-a3724c72be1271b05")
CT = W / "examples/core-tests"
TESTS = ["291_static_one_element_slices", "292_closure_fn_constants", "249_dyn_constant_refused",
         "247_static_wide_pointers", "241_niche_wide_pointer_constants", "26_async"]
BCONST = "backend/src/constant.rs"
BENC = "backend/src/jir/encode.rs"
LCONST = "jsc-link/src/lower/js/constant.rs"
EVAL = "rcgjs-object/src/jir/interp/eval.rs"

ENV = {
    "def": {"JS_EXTRA_ARGS": ""},
    "jiroff": {"JS_EXTRA_ARGS": "js-lane=jir js-opt=off"},
    "rb": {"JS_EXTRA_ARGS": "js-profile=release-balanced"},
    "rbz": {"JS_EXTRA_ARGS": "js-profile=release-balanced", "JSC_RUST_OPT_LEVEL": "z", "JSC_RUST_MIR_INLINE": "on"},
}

# id: (description, kind, [(file, old, new, expected_count)])   kind: backend | linker | interp
MUTANTS = {
    "S1": ("one-element slice: fresh array of the binding instead of the static's slot (round-2 form)", "backend", [(BCONST,
        "                _ if static_ty == element && is_indirect(tcx, element) => (\n                    crate::ptr::slot_buf(jsast::rt_call(\"box\", vec![binding])),\n                    Some(1),\n                ),\n",
        "                _ if static_ty == element && is_indirect(tcx, element) => (\n                    jsast::array(vec![binding]),\n                    Some(1),\n                ),\n", 1)]),
    "C1": ("baseline lane: closure fn-pointer constant printed bare (no wrapper)", "backend", [(BCONST,
        "                    jsast::arrow(params, jsast::call(target, arguments))\n",
        "                    {\n                        let _ = (params, arguments);\n                        target\n                    }\n", 1)]),
    "C2": ("baseline lane: rust-call exclusion removed (trait-method pointer gets the wrapper)", "backend", [(BCONST,
        "                ty::FnPtr(..) if ty.fn_sig(tcx).abi() == rustc_abi::ExternAbi::RustCall => target,\n",
        "", 1)]),
    "C3": ("baseline lane: closure at a non-fn-pointer type printed bare instead of refused", "backend", [(BCONST,
        "                _ => zombie(\n                    cgu,\n                    log,\n                    span,\n                    format!(\n                        \"a closure stored as a `{ty}`, which is not a `fn` pointer, in constant \\\n                         data is not supported by rustc_codegen_js\"\n                    ),\n                ),\n            }\n        }\n",
        "                _ => target,\n            }\n        }\n", 1)]),
    "C4": ("encoder: closure type not described (implementer's K2)", "backend", [(BENC,
        "                if let Some(closure) = crate::constant::reified_closure_type(tcx, instance) {\n                    self.type_id(closure)?;\n                }\n",
        "", 1)]),
    "C5": ("reified_closure_type: the call_once shim of a Fn/FnMut closure not recognized", "backend", [(BCONST,
        "            Some(instance.args.type_at(0)).filter(|ty| matches!(ty.kind(), ty::Closure(..)))\n",
        "            None::<Ty<'tcx>>.filter(|_| { let _ = instance; true })\n", 1)]),
    "C6": ("reified_closure_type: the closure's own body (FnOnce closure) not recognized", "backend", [(BCONST,
        "            Some(Ty::new_closure(tcx, def_id, instance.args))\n",
        "            None::<Ty<'tcx>>.filter(|_| { let _ = (def_id, instance); true })\n", 1)]),
    "D1": ("baseline lane: trait-object refusal removed", "backend", [(BCONST,
        "        .is_some_and(|pointee| matches!(pointee.kind(), ty::Dynamic(..)))\n    {\n",
        "        .is_some_and(|pointee| matches!(pointee.kind(), ty::Dynamic(..)) && false)\n    {\n", 1)]),
    "L1": ("linker: closure fn-pointer global printed bare (no wrapper)", "linker", [(LCONST,
        "                        Ok(jsast::arrow(params, jsast::call(target, arguments)))\n",
        "                        {\n                            let _ = (params, arguments);\n                            Ok(target)\n                        }\n", 1)]),
    "L2": ("linker: rust-call exclusion removed", "linker", [(LCONST,
        "                        if closure && signature.abi != SigAbi::RustCall =>\n",
        "                        if closure && { let _ = SigAbi::RustCall; true } =>\n", 1)]),
    "L3": ("linker: trait-object-into-static decline removed", "linker", [(LCONST,
        "                if matches!(\n                    value::kind(self.table, pointee),\n                    Some(TypeKind::Dynamic { .. })\n                ) {\n",
        "                if false && matches!(\n                    value::kind(self.table, pointee),\n                    Some(TypeKind::Dynamic { .. })\n                ) {\n", 1)]),
    "L4": ("linker: a closure at a non-fn-pointer type printed bare instead of declined", "linker", [(LCONST,
        "                    ) if !closure => Ok(target),\n",
        "                    ) => Ok(target),\n", 1)]),
    "I1": ("interpreter: payload-carrying variant evaluated without its payload", "interp", [(EVAL,
        "                if payload && carries {\n", "                if false && payload && carries {\n", 1)]),
    "I2": ("interpreter: fieldless niche variant reported unsupported (the round-2 behaviour)", "interp", [(EVAL,
        "                return Ok(Value::Aggregate {\n                    ty,\n                    variant,\n                    fields: Vec::new(),\n                });\n            }\n            let variant = table\n",
        "                let _ = variant;\n                return Err(Trap::Unsupported(\"mutant\"));\n            }\n            let variant = table\n", 1)]),
    "I3": ("interpreter: niche variant taken as the discriminant (main's rule)", "interp", [(EVAL,
        "                    (index, index == niche.untagged_variant)\n",
        "                    let _ = index;\n                    (bits as u32, bits as u32 == niche.untagged_variant)\n", 1)]),
}


def sh(cmd, env=None, cwd=None):
    e = dict(os.environ)
    for k in ("JSC_RUST_OPT_LEVEL", "JSC_RUST_MIR_INLINE", "JS_EXTRA_ARGS"):
        e.pop(k, None)
    if env:
        e.update(env)
    return subprocess.run(cmd, cwd=cwd, env=e, capture_output=True, text=True)


def make_root(mid, bin_dir, sysroots):
    root = V / "mutroot3"
    if root.exists():
        shutil.rmtree(root)
    (root / "build").mkdir(parents=True)
    for e in ("backend Cargo.toml Cargo.lock examples jsc-link patches rcgjs-object runtime rust-toolchain.toml "
              "scripts experiments mires-js view-abi view-async js-macro js-extern-macro structurizer rustc-cleanup contract").split():
        if (W / e).exists():
            os.symlink(W / e, root / e)
    os.symlink(bin_dir, root / "target")
    os.symlink(sysroots, root / "build" / "sysroots")
    return root


def run_tests(mid, root, modes):
    out = V / "out" / "mut3" / mid
    out.mkdir(parents=True, exist_ok=True)
    shim = (W / "runtime/shim.js").read_text()
    entry = (W / "scripts/entry.js").read_text()
    failures = []
    for mode in modes:
        env = dict(ENV[mode])
        env["SKIP_BUILD"] = "1"
        for name in TESTS:
            t = CT / f"{name}.rs"
            js = out / f"{name}.{mode}.js"
            r = sh(["bash", "scripts/compile_core.sh", str(t), str(js)], env, cwd=root)
            (out / f"{name}.{mode}.log").write_text(r.stdout + r.stderr)
            expect_fail = t.with_suffix(".expect_fail")
            if expect_fail.exists():
                wanted = [l for l in expect_fail.read_text().splitlines() if l.strip()]
                text = r.stdout + r.stderr
                if r.returncode == 0:
                    failures.append(f"{name}:{mode}:compiled")
                elif not all(w in text for w in wanted):
                    failures.append(f"{name}:{mode}:diagnostic-missing")
                continue
            if r.returncode:
                failures.append(f"{name}:{mode}:compile-refused")
                continue
            run = subprocess.run(["node", "-"], input=shim + js.read_text() + entry, capture_output=True, text=True)
            expected = t.with_suffix(".expected").read_text()
            if run.returncode:
                failures.append(f"{name}:{mode}:crash")
            elif run.stdout != expected:
                failures.append(f"{name}:{mode}:wrong-output")
    return failures


def main():
    ids = sys.argv[1:]
    if ids == ["all"]:
        ids = list(MUTANTS)
    log = open(V / "mutants3.log", "a")
    jl = open(V / "mutants3.jsonl", "a")
    for mid in ids:
        desc, kind, edits = MUTANTS[mid]
        files = sorted({f for f, *_ in edits})
        originals = {f: (W / f).read_text() for f in files}
        failures = None
        try:
            texts = dict(originals)
            for f, old, new, count in edits:
                assert texts[f].count(old) == count, f"{mid}: `{old[:60]}` occurs {texts[f].count(old)} times in {f}"
                texts[f] = texts[f].replace(old, new)
            for f in files:
                (W / f).write_text(texts[f])
            if kind == "interp":
                r = sh(["cargo", "test", "--locked", "-p", "rcgjs-object", "--lib"], cwd=W)
                tail = [l for l in (r.stdout + r.stderr).splitlines() if l.startswith("test result") or " FAILED" in l or "panicked" in l]
                verdict = ("KILLED by rcgjs-object unit tests: " if r.returncode else "SURVIVED rcgjs-object unit tests: ") + " | ".join(tail[:6])[:500]
                if r.returncode == 0:
                    p = sh(["cargo", "test", "--locked", "-p", "jsc-link", "--lib", "--", "--nocapture", "--test-threads=1",
                            "dce_preserves_fixture_semantics", "pea_preserves_fixture_semantics",
                            "summaries_admit_every_interpreted_behavior",
                            "fixture_bodies_build_verify_and_agree_with_the_semantic_interpreter"],
                           {"RCGJS_SSA_FIXTURES_REQUIRED": "1", "RCGJS_WITNESS_CHECK": "1"}, cwd=W)
                    (V / "out" / "mut3").mkdir(parents=True, exist_ok=True)
                    (V / "out" / "mut3" / f"{mid}.props.full").write_text(p.stdout + p.stderr)
                    lines = [l for l in (p.stdout + p.stderr).splitlines() if l.startswith("test result") or "MISMATCH" in l or "VIOLATION" in l]
                    verdict += (" ;; property tests KILLED: " if p.returncode else " ;; property tests SURVIVED: ") + " | ".join(lines[:4])[:400]
            else:
                b = sh(["bash", "scripts/build.sh", "--locked"], cwd=W)
                if b.returncode:
                    verdict = "BUILD-FAILED " + (b.stdout + b.stderr)[-700:].replace("\n", " | ")
                else:
                    bin_dir = V / "mut-bin" / mid
                    (bin_dir / "release").mkdir(parents=True, exist_ok=True)
                    shutil.copy2(W / "target/release/jsc-link", bin_dir / "release/jsc-link")
                    shutil.copy2(W / "target/release/librustc_codegen_mires.dylib", bin_dir / "release/librustc_codegen_mires.dylib")
                    same_backend = subprocess.run(["cmp", "-s", str(bin_dir / "release/librustc_codegen_mires.dylib"), str(V / "fix-bin/release/librustc_codegen_mires.dylib")]).returncode == 0
                    for f in files:
                        (W / f).write_text(originals[f])
                    if same_backend:
                        sysroots = V / "fix-sysroots"
                    else:
                        sysroots = V / "mut-sysroots" / mid
                        sysroots.mkdir(parents=True, exist_ok=True)
                    root = make_root(mid, bin_dir, sysroots)
                    modes = ["def", "jiroff", "rb", "rbz"]
                    if not same_backend:
                        built = []
                        for mode in modes:
                            env = dict(ENV[mode]); env["SKIP_BUILD"] = "1"
                            s = sh(["bash", "scripts/build_sysroot.sh"], env, cwd=root)
                            if s.returncode == 0:
                                built.append(mode)
                        modes = built
                    failures = run_tests(mid, root, modes)
                    verdict = (f"KILLED {failures}" if failures else "SURVIVED") + f" (modes run: {','.join(modes)}; backend {'unchanged' if same_backend else 'mutated'})"
                    if not same_backend:
                        shutil.rmtree(sysroots, ignore_errors=True)
        finally:
            for f in files:
                (W / f).write_text(originals[f])
        line = f"{mid}\t{kind}\t{desc}\t{verdict}"
        print(line, flush=True)
        log.write(line + "\n"); log.flush()
        jl.write(json.dumps({"id": mid, "kind": kind, "desc": desc, "verdict": verdict, "failures": failures}) + "\n"); jl.flush()


main()
