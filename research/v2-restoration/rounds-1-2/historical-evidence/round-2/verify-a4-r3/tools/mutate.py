#!/usr/bin/env python3
"""Round-2 verifier mutation controls for the linker (jsc-link) parts of A4.

Usage: mutate.py <id>...        (ids from MUTANTS; "all" for every one)

Each mutant is applied to the verifier worktree, `jsc-link` is rebuilt, the binary is copied
beside the unmutated backend dylib into mut-bin/<id>/release, the edit is reverted (git checkout of
that one file), and the committed tests 240-248 + niche_constant are compiled and run through a
probe root (mutroot) in release-balanced and then `js-lane=jir js-opt=off`. The first failing test
kills the mutant. Results: mutants.log, mutants.jsonl; per-test output in out/mut/<id>/.
"""
import json, os, shutil, subprocess, sys
from pathlib import Path

V = Path("/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3")
W = Path("/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-a3724c72be1271b05")
ROOT = V / "mutroot"
CT = W / "examples/core-tests"
TESTS = [CT / f"{n}.rs" for n in (
    "245_niche_absent_variants", "246_niche_single_variant_payloads", "248_single_variant_discriminant",
    "242_niche_integer_constants", "241_niche_wide_pointer_constants", "240_niche_pointer_constants",
    "244_niche_pointer_wrappers", "243_niche_static_references", "247_static_wide_pointers")] + [
    W / "examples/round1-open/niche_constant.rs"]
VALUE = "jsc-link/src/lower/js/value.rs"
CONST = "jsc-link/src/lower/js/constant.rs"
RVALUE = "jsc-link/src/lower/js/rvalue.rs"
DCE = "jsc-link/src/opt/dce.rs"

# id: (description, [(file, old, new, expected_count)])
MUTANTS = {
    "BASE": ("no change (reference lane counts)", []),
    "A1": ("single_layout_variant takes variant 0 (the round-1 rule)", [(VALUE,
        "    variants\n        .iter()\n        .position(|variant| !absent_variant(table, variant))\n}",
        "    let _ = table;\n    (!variants.is_empty()).then_some(0)\n}", 1)]),
    "A2": ("niche walk: enum step takes the first variant", [(VALUE,
        "                .get(single_layout_variant(table, variants)?)?\n",
        "                .first()?\n", 1)]),
    "A3": ("niche walk: tuple step removed", [(VALUE,
        "        TypeKind::Tuple(elements)\n        | TypeKind::Closure {\n            upvars: elements, ..\n        } => elements.get(index).copied(),\n",
        "        TypeKind::Closure {\n            upvars: elements, ..\n        } => elements.get(index).copied(),\n", 1)]),
    "A4": ("niche walk: array step removed", [(VALUE,
        "        TypeKind::Array { element, length } => ((index as u64) < *length).then_some(*element),\n",
        "", 1)]),
    "A5": ("niche walk: pattern-type step removed", [(VALUE,
        "        TypeKind::Pattern(base) if index == 0 => Some(*base),\n", "", 1)]),
    "A6": ("niche walk: struct step removed (union kept)", [(VALUE,
        "            form: NominalForm::Struct | NominalForm::Union,\n            variants,\n            ..\n        } => Some(variants.first()?.fields.get(index)?.ty),\n",
        "            form: NominalForm::Union,\n            variants,\n            ..\n        } => Some(variants.first()?.fields.get(index)?.ty),\n", 1)]),
    "A6b": ("niche walk: union step removed (struct kept)", [(VALUE,
        "            form: NominalForm::Struct | NominalForm::Union,\n            variants,\n            ..\n        } => Some(variants.first()?.fields.get(index)?.ty),\n",
        "            form: NominalForm::Struct,\n            variants,\n            ..\n        } => Some(variants.first()?.fields.get(index)?.ty),\n", 1)]),
    "A7": ("scalar_bits: transparent peel removed", [(VALUE,
        "    if let Some(inner) = descriptor.js.transparent {\n        return scalar_bits(table, inner.ty, depth + 1);\n    }\n",
        "", 1)]),
    "A8": ("scalar_bits: pattern arm removed", [(VALUE,
        "        TypeKind::Pattern(base) => scalar_bits(table, *base, depth + 1),\n", "", 1)]),
    "A9": ("scalar_bits: nested niche enum arm gives None (decline)", [(VALUE,
        "        } => niche_width(table, variants, niche),\n",
        "        } => {\n            let _ = (variants, niche);\n            None\n        }\n", 1)]),
    "A10": ("scalar_bits: direct tag arm gives None (decline)", [(VALUE,
        "        } => variants\n            .iter()\n            .find_map(|variant| variant.physical_tag)\n            .map(|tag| u32::from(tag.bytes) * 8),\n",
        "        } => {\n            let _ = variants;\n            None\n        }\n", 1)]),
    "A11": ("decline path: unknown width guessed as 128 bits (the round-1 behaviour)", [(VALUE,
        "    let width = niche_width(table, variants, niche)?;\n",
        "    let width = niche_width(table, variants, niche).unwrap_or(128);\n", 1)]),
    "A12": ("niche subtraction never wraps at the scalar width", [(VALUE,
        "        _ => (1u128 << width) - 1,\n", "        _ => u128::MAX,\n", 1)]),
    "A14": ("niche walk: no stop at a pointer (wide pointer step declines)", [(VALUE,
        "                TypeKind::Reference { .. }\n                | TypeKind::RawPointer { .. }\n                | TypeKind::FnPtr { .. } => break,\n",
        "", 1)]),
    "A15": ("absent rule: any uninhabited field (one_zst ignored)", [(VALUE,
        "        && variant.fields.iter().all(|field| one_zst(field.ty))\n",
        "        && { let _ = &one_zst; true }\n", 1)]),
    "A16": ("uninhabited_layout shallow (struct, tuple, array steps removed)", [
        (VALUE, "        }) => variants.first().is_some_and(any_field),\n", "        }) => { let _ = variants; false }\n", 1),
        (VALUE, "            .any(|element| uninhabited_layout(table, *element, depth + 1)),\n", "            .any(|element| { let _ = element; false }),\n", 1),
        (VALUE, "            *length != 0 && uninhabited_layout(table, *element, depth + 1)\n", "            { let _ = (length, element); false }\n", 1)]),
    "A16s": ("uninhabited_layout: struct step only removed", [
        (VALUE, "        }) => variants.first().is_some_and(any_field),\n", "        }) => { let _ = variants; false }\n", 1)]),
    "A16t": ("uninhabited_layout: tuple step only removed", [
        (VALUE, "            .any(|element| uninhabited_layout(table, *element, depth + 1)),\n", "            .any(|element| { let _ = element; false }),\n", 1)]),
    "A16a": ("uninhabited_layout: array step only removed", [
        (VALUE, "            *length != 0 && uninhabited_layout(table, *element, depth + 1)\n", "            { let _ = (length, element); false }\n", 1)]),
    "A17": ("uninhabited enum only when it has no variants", [(VALUE,
        "        }) => variants.iter().all(any_field),\n", "        }) => variants.is_empty(),\n", 1)]),
    "A18": ("uninhabited array ignores a zero length", [(VALUE,
        "            *length != 0 && uninhabited_layout(table, *element, depth + 1)\n",
        "            { let _ = length; uninhabited_layout(table, *element, depth + 1) }\n", 1)]),
    "A19": ("uninhabited_layout: pattern step removed", [(VALUE,
        "        Some(TypeKind::Pattern(base)) => uninhabited_layout(table, *base, depth + 1),\n", "", 1)]),
    "A21": ("is_single_layout_enum: Number-repr exclusion removed", [(VALUE,
        "        .all(|variant| variant.physical_tag.is_none())\n        && !matches!(descriptor.js.enum_repr, Some(EnumRepr::Number { .. }))\n}",
        "        .all(|variant| variant.physical_tag.is_none())\n}", 1)]),
    "A22": ("constant.rs: single-layout scalar constant decoded as variant 0", [(CONST,
        "                        (index as u32, true)\n",
        "                        let _ = index;\n                        (0, true)\n", 1)]),
    "A23": ("constant.rs: single-layout scalar constant declined (as before round 2)", [(CONST,
        "                    None if value::is_single_layout_enum(table, ty) => {\n",
        "                    None if value::is_single_layout_enum(table, ty) && variants.is_empty() => {\n", 1)]),
    "A24": ("constant.rs: transparent peel removed", [(CONST,
        "        if let Some(inner) = descriptor.js.transparent {\n            return self.scalar_expr(bits, inner.ty);\n        }\n        if let TypeKind::Pattern(base) = descriptor.kind {",
        "        if let TypeKind::Pattern(base) = descriptor.kind {", 1)]),
    "A25": ("constant.rs: pattern peel removed", [(CONST,
        "        if let TypeKind::Pattern(base) = descriptor.kind {\n            return self.scalar_expr(bits, base);\n        }\n",
        "", 1)]),
    "A26": ("constant.rs global_expr: a function at a non-pointer type is printed bare again", [(CONST,
        "                _ => Err(decline(\n                    OpClass::Constant,\n                    \"a pointer to a function at a type that is not a pointer\",\n                )),\n",
        "                _ => Ok(jsast::id(\n                    self.interns.str(*link_name).unwrap_or(\"\").to_owned(),\n                )),\n", 1)]),
    "A27": ("constant.rs global_expr: wide pointer to a static not declined", [(CONST,
        "                if matches!(\n                    value::kind(self.table, pointee),\n                    Some(TypeKind::Slice(_) | TypeKind::Str)\n                ) {\n",
        "                if false && matches!(\n                    value::kind(self.table, pointee),\n                    Some(TypeKind::Slice(_) | TypeKind::Str)\n                ) {\n", 1)]),
    "A28": ("dce.rs static_discriminant: first shallowly inhabited variant (pre-61e0c34)", [(DCE,
        "        true => crate::lower::js::value::single_layout_variant(table, variants).unwrap_or(0),\n",
        "        true => inhabited.first().copied().unwrap_or(0),\n", 1)]),
    "A29": ("rvalue.rs discriminant: first shallowly inhabited variant (pre-61e0c34)", [(RVALUE,
        "                true => value::single_layout_variant(table, variants).unwrap_or(0),\n",
        "                true => inhabited.first().copied().unwrap_or(0),\n", 1)]),
    "A30": ("dce.rs scalar_variant: niche constants never folded", [(DCE,
        "        return crate::lower::js::value::niche_variant(table, variants, niche, bits);\n",
        "        let _ = (variants, niche, bits);\n        return None;\n", 1)]),
    "A31": ("niche_variants range: end taken as start (one niche variant only)", [(VALUE,
        "    let count = (end - start + 1) as u128;\n", "    let count = 1u128;\n    let _ = end;\n", 1)]),
    "A32": ("niche_variants range: starts at variant 0 regardless of absence", [(VALUE,
        "    let present = |index: usize| !absent_variant(table, &variants[index]);\n",
        "    let present = |index: usize| { let _ = (table, &variants[index]); true };\n", 1)]),
}

ENV = {
    "rb": {"JS_EXTRA_ARGS": "js-profile=release-balanced"},
    "jiroff": {"JS_EXTRA_ARGS": "js-lane=jir js-opt=off"},
}


def sh(cmd, env=None, cwd=None):
    e = dict(os.environ)
    e.pop("JSC_RUST_OPT_LEVEL", None)
    e.pop("JSC_RUST_MIR_INLINE", None)
    if env:
        e.update(env)
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, env=e)


def make_root(bin_dir):
    if ROOT.exists():
        shutil.rmtree(ROOT)
    (ROOT / "build").mkdir(parents=True)
    for e in ("backend Cargo.toml Cargo.lock examples jsc-link patches rcgjs-object runtime rust-toolchain.toml "
              "scripts experiments mires-js view-abi view-async js-macro js-extern-macro structurizer "
              "rustc-cleanup contract").split():
        if (W / e).exists():
            os.symlink(W / e, ROOT / e)
    os.symlink(bin_dir, ROOT / "target")
    os.symlink(V / "fix-sysroots", ROOT / "build" / "sysroots")


def run_tests(mid):
    out = V / "out" / "mut" / mid
    out.mkdir(parents=True, exist_ok=True)
    shim = (W / "runtime/shim.js").read_text()
    entry = (W / "scripts/entry.js").read_text()
    for mode in ("rb", "jiroff"):
        env = dict(ENV[mode])
        env["SKIP_BUILD"] = "1"
        for t in TESTS:
            js = out / f"{t.stem}.{mode}.js"
            r = sh(["bash", "scripts/compile_core.sh", str(t), str(js)], env, cwd=ROOT)
            if r.returncode:
                (out / f"{t.stem}.{mode}.log").write_text(r.stderr)
                return f"{t.stem}:{mode}:compile-refused"
            run = subprocess.run(["node", "-"], input=shim + js.read_text() + entry, capture_output=True, text=True)
            expected = t.with_suffix(".expected").read_text()
            if run.returncode:
                return f"{t.stem}:{mode}:crash"
            if run.stdout != expected:
                return f"{t.stem}:{mode}:wrong-output"
    return None


def main():
    ids = sys.argv[1:]
    if ids == ["all"]:
        ids = list(MUTANTS)
    log = open(V / "mutants.log", "a")
    jl = open(V / "mutants.jsonl", "a")
    for mid in ids:
        desc, edits = MUTANTS[mid]
        files = sorted({f for f, *_ in edits})
        originals = {f: (W / f).read_text() for f in files}
        try:
            texts = dict(originals)
            for f, old, new, count in edits:
                assert texts[f].count(old) == count, f"{mid}: `{old[:50]}` occurs {texts[f].count(old)} times in {f}"
                texts[f] = texts[f].replace(old, new)
            for f in files:
                (W / f).write_text(texts[f])
            b = sh(["cargo", "build", "--release", "--locked", "-p", "jsc-link"], cwd=W)
            if b.returncode:
                verdict = "BUILD-FAILED " + b.stderr[-600:].replace("\n", " | ")
            else:
                bin_dir = V / "mut-bin" / mid
                (bin_dir / "release").mkdir(parents=True, exist_ok=True)
                shutil.copy2(W / "target/release/jsc-link", bin_dir / "release/jsc-link")
                shutil.copy2(V / "fix-bin/release/librustc_codegen_mires.dylib", bin_dir / "release/librustc_codegen_mires.dylib")
                make_root(bin_dir)
                killer = run_tests(mid)
                verdict = f"KILLED by {killer}" if killer else "SURVIVED (240-248 + niche_constant, rb and jiroff)"
        finally:
            for f in files:
                (W / f).write_text(originals[f])
        line = f"{mid}\t{desc}\t{verdict}"
        print(line, flush=True)
        log.write(line + "\n")
        log.flush()
        jl.write(json.dumps({"id": mid, "desc": desc, "verdict": verdict}) + "\n")
        jl.flush()


main()
