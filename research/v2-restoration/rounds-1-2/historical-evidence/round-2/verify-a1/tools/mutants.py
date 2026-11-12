#!/usr/bin/env python3
"""mutants.py [names...]: builds one jsc-link per mutant from verify-a1/mut-src (a `git archive` of
the commit under test), into verify-a1/mut/bin/<name>, runs the jsc-link unit tests against the
mutated source, and restores the source. Logs in verify-a1/mut/."""
import os, shutil, subprocess, sys

V = "/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a1"
SRC = V + "/mut-src"
FUSE = "jsc-link/src/lower/fuse.rs"
SSA = "jsc-link/src/lower/ssa.rs"


def rep(t, old, new, count=1):
    assert t.count(old) == count, (t.count(old), old)
    return t.replace(old, new)


def seg(t, fn):
    i = t.index("fn uses_as_storage")
    j = t.index("_ => false,", i)
    return t[:i] + fn(t[i:j]) + t[j:]


SETBOTH = "        OpKind::SetField { base, .. } | OpKind::SetDiscriminant { base, .. } => *base == value,"
BV = "        OpKind::BorrowView {\n            root: Root::Value(root),\n            path,\n            ..\n        }"
RV = "        | OpKind::RawView {\n            root: Root::Value(root),\n            path,\n            ..\n        }"
TAIL = " => *root == value && !path.iter().any(|proj| matches!(proj, Proj::Deref)),"
CALL = "        if uses_as_storage(&op.kind, value) {\n            return true;\n        }\n"
OWN = "            let own = self.op(OpKind::Copy(shared), local_ty);\n            self.write_bare_local(place.local, own);\n"

MUTANTS = {
    # fuse.rs: remove or narrow each arm of rule A
    "no_setfield": (FUSE, lambda t: rep(t, SETBOTH, "        OpKind::SetDiscriminant { base, .. } => *base == value,")),
    "no_setdisc": (FUSE, lambda t: rep(t, SETBOTH, "        OpKind::SetField { base, .. } => *base == value,")),
    "no_borrowview": (FUSE, lambda t: seg(t, lambda s: rep(s, BV + "\n" + RV, "        OpKind::RawView {\n            root: Root::Value(root),\n            path,\n            ..\n        }"))),
    "no_rawview": (FUSE, lambda t: seg(t, lambda s: rep(s, BV + "\n" + RV, BV))),
    "mutable_only": (FUSE, lambda t: seg(t, lambda s: rep(s, "            path,\n            ..\n", "            path,\n            mutable: true,\n", 2))),
    "borrow_mutable_only": (FUSE, lambda t: seg(t, lambda s: rep(s, BV, BV.replace("            ..\n", "            mutable: true,\n")))),
    "raw_mutable_only": (FUSE, lambda t: seg(t, lambda s: rep(s, RV, RV.replace("            ..\n", "            mutable: true,\n")))),
    "no_deref_exempt": (FUSE, lambda t: rep(t, TAIL, " => *root == value && path.len() < usize::MAX,")),
    "deref_first_only": (FUSE, lambda t: rep(t, TAIL, " => *root == value && !matches!(path.first(), Some(Proj::Deref)),")),
    "no_storage_rule": (FUSE, lambda t: rep(t, CALL, "")),
    # ssa.rs: rule B
    "no_own_value": (SSA, lambda t: rep(t, OWN, "            let _ = shared;\n")),
    "own_even_through_deref": (SSA, lambda t: rep(t, "            && !self.table.needs_clone(local_ty)\n            && !place\n                .projection\n                .iter()\n                .any(|projection| matches!(projection, Projection::Deref))\n", "            && !self.table.needs_clone(local_ty)\n")),
    "own_for_every_type": (SSA, lambda t: rep(t, "            && !self.table.needs_clone(local_ty)\n            && !place\n", "            && !place\n")),
    "own_for_cells_too": (SSA, lambda t: rep(t, "        if value_root\n            && !self.table.needs_clone(local_ty)\n", "        if !self.table.needs_clone(local_ty)\n")),
    # the tighter rule for point 5: a view rooted at a value that is itself an object read out of a
    # place (Field/Deref of an indirect type) designates that object whether spelled or named.
    "tight_place_alias": (FUSE, lambda t: rep(rep(t, CALL,
        "        if uses_as_storage(&op.kind, value) && !self.reads_shared_object(value, op) {\n            return true;\n        }\n"),
        "    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which\n",
        "    fn reads_shared_object(&self, value: ValueId, op: &crate::jir::repr::Op) -> bool {\n"
        "        if !matches!(op.kind, OpKind::BorrowView { .. } | OpKind::RawView { .. }) {\n            return false;\n        }\n"
        "        if !self.table.is_indirect(self.func.values[value.0 as usize].ty) {\n            return false;\n        }\n"
        "        let Some(Def::Op(block, index)) = self.defs[value.0 as usize] else {\n            return false;\n        };\n"
        "        matches!(\n            self.func.blocks[block.0 as usize].ops[index].kind,\n            OpKind::Field { .. } | OpKind::Deref(_)\n        )\n    }\n\n"
        "    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which\n")),
    # point 5 candidates. C1: the root is a call's result of an object type (one object however it is
    # spelled, evaluated once, nothing for the member fold to see through). C2: the view is consumed
    # by the very next op, a `Deref` of it (a read; `inline_adjacent_place_views` spells it as one).
    "tight3": (FUSE, lambda t: rep(rep(t, CALL,
        "        if uses_as_storage(&op.kind, value) && !self.storage_is_its_own(value, use_, op) {\n            return true;\n        }\n"),
        "    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which\n",
        "    fn storage_is_its_own(&self, value: ValueId, use_: &Use, op: &crate::jir::repr::Op) -> bool {\n"
        "        if !matches!(op.kind, OpKind::BorrowView { .. } | OpKind::RawView { .. }) {\n            return false;\n        }\n"
        "        if let Some(result) = op.result {\n"
        "            if let [only] = self.uses[result.0 as usize].as_slice() {\n"
        "                if !only.conditional && only.block == use_.block && only.position == use_.position + 1 {\n"
        "                    if let Some(next) = self.func.blocks[only.block.0 as usize].ops.get(only.position) {\n"
        "                        if matches!(next.kind, OpKind::Deref(pointer) if pointer == result) {\n                            return true;\n                        }\n"
        "                    }\n"
        "                }\n"
        "            }\n"
        "        }\n"
        "        if self.table.is_indirect(self.func.values[value.0 as usize].ty) {\n"
        "            if let Some(Def::Op(block, index)) = self.defs[value.0 as usize] {\n"
        "                if matches!(self.func.blocks[block.0 as usize].ops[index].kind, OpKind::Call { .. }) {\n                    return true;\n                }\n"
        "            }\n"
        "        }\n"
        "        false\n"
        "    }\n\n"
        "    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which\n")),
    # C3 (with C1 and C2): a WHOLE view of an object-typed root is the object itself, so spelling the
    # root inside the view is one fresh object. That is sound unless the view is in turn spelled
    # inside a place walk (a reborrow, load, store or drop through it), where the member fold could
    # see through the literal. So: exempt when the view has several uses (it is then declared, and
    # the root is evaluated once into its name) or its one use is a call argument.
    "tight4": (FUSE, lambda t: rep(rep(t, CALL,
        "        if uses_as_storage(&op.kind, value) && !self.storage_is_its_own(value, use_, op) {\n            return true;\n        }\n"),
        "    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which\n",
        "    fn storage_is_its_own(&self, value: ValueId, use_: &Use, op: &crate::jir::repr::Op) -> bool {\n"
        "        let (OpKind::BorrowView { path, .. } | OpKind::RawView { path, .. }) = &op.kind else {\n            return false;\n        };\n"
        "        let Some(result) = op.result else {\n            return false;\n        };\n"
        "        let view_uses = self.uses[result.0 as usize].as_slice();\n"
        "        if let [only] = view_uses {\n"
        "            if !only.conditional && only.block == use_.block && only.position == use_.position + 1 {\n"
        "                if let Some(next) = self.func.blocks[only.block.0 as usize].ops.get(only.position) {\n"
        "                    if matches!(next.kind, OpKind::Deref(pointer) if pointer == result) {\n                        return true;\n                    }\n"
        "                }\n"
        "            }\n"
        "        }\n"
        "        if !self.table.is_indirect(self.func.values[value.0 as usize].ty) {\n            return false;\n        }\n"
        "        if let Some(Def::Op(block, index)) = self.defs[value.0 as usize] {\n"
        "            if matches!(self.func.blocks[block.0 as usize].ops[index].kind, OpKind::Call { .. }) {\n                return true;\n            }\n"
        "        }\n"
        "        if !path.is_empty() {\n            return false;\n        }\n"
        "        match view_uses {\n"
        "            [] => false,\n"
        "            [only] => {\n"
        "                !only.conditional\n"
        "                    && matches!(\n"
        "                        self.func.blocks[only.block.0 as usize].ops.get(only.position).map(|next| &next.kind),\n"
        "                        Some(OpKind::Call { host: None, .. })\n"
        "                    )\n"
        "            }\n"
        "            _ => true,\n"
        "        }\n"
        "    }\n\n"
        "    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which\n")),
    # tight4 plus the chain: a whole view that is only reborrowed whole (`[Deref]`), unsized, moved
    # or copied on its way to a call argument (`f(&[a, b])`) still is the one fresh object.
    "tight5": (FUSE, lambda t: rep(rep(t, CALL,
        "        if uses_as_storage(&op.kind, value) && !self.storage_is_its_own(value, use_, op) {\n            return true;\n        }\n"),
        "    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which\n",
        "    fn storage_is_its_own(&self, value: ValueId, use_: &Use, op: &crate::jir::repr::Op) -> bool {\n"
        "        let (OpKind::BorrowView { path, .. } | OpKind::RawView { path, .. }) = &op.kind else {\n            return false;\n        };\n"
        "        let Some(result) = op.result else {\n            return false;\n        };\n"
        "        if let [only] = self.uses[result.0 as usize].as_slice() {\n"
        "            if !only.conditional && only.block == use_.block && only.position == use_.position + 1 {\n"
        "                if let Some(next) = self.func.blocks[only.block.0 as usize].ops.get(only.position) {\n"
        "                    if matches!(next.kind, OpKind::Deref(pointer) if pointer == result) {\n                        return true;\n                    }\n"
        "                }\n"
        "            }\n"
        "        }\n"
        "        if !self.table.is_indirect(self.func.values[value.0 as usize].ty) {\n            return false;\n        }\n"
        "        if let Some(Def::Op(block, index)) = self.defs[value.0 as usize] {\n"
        "            if matches!(self.func.blocks[block.0 as usize].ops[index].kind, OpKind::Call { .. }) {\n                return true;\n            }\n"
        "        }\n"
        "        if !path.is_empty() {\n            return false;\n        }\n"
        "        let mut current = result;\n"
        "        for _ in 0..8 {\n"
        "            let only = match self.uses[current.0 as usize].as_slice() {\n"
        "                [] => return false,\n"
        "                [only] => only,\n"
        "                _ => return true,\n"
        "            };\n"
        "            if only.conditional {\n                return false;\n            }\n"
        "            let Some(next) = self.func.blocks[only.block.0 as usize].ops.get(only.position) else {\n                return false;\n            };\n"
        "            let Some(next_result) = next.result else {\n                return matches!(next.kind, OpKind::Call { host: None, .. });\n            };\n"
        "            match &next.kind {\n"
        "                OpKind::Call { host: None, .. } => return true,\n"
        "                OpKind::BorrowView { root: Root::Value(root), path, .. }\n"
        "                | OpKind::RawView { root: Root::Value(root), path, .. }\n"
        "                    if *root == current && path.as_slice() == [Proj::Deref] =>\n"
        "                {\n                    current = next_result;\n                }\n"
        "                OpKind::Cast { kind: CastKind::PointerCoercion, v, .. } if *v == current => current = next_result,\n"
        "                OpKind::Move(v) | OpKind::Copy(v) if *v == current => current = next_result,\n"
        "                _ => return false,\n"
        "            }\n"
        "        }\n"
        "        false\n"
        "    }\n\n"
        "    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which\n")),
    "tight_call": (FUSE, lambda t: rep(rep(t, CALL,
        "        if uses_as_storage(&op.kind, value) && !self.storage_is_its_own(value, op) {\n            return true;\n        }\n"),
        "    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which\n",
        "    fn storage_is_its_own(&self, value: ValueId, op: &crate::jir::repr::Op) -> bool {\n"
        "        if !matches!(op.kind, OpKind::BorrowView { .. } | OpKind::RawView { .. }) {\n            return false;\n        }\n"
        "        if self.table.is_indirect(self.func.values[value.0 as usize].ty) {\n"
        "            if let Some(Def::Op(block, index)) = self.defs[value.0 as usize] {\n"
        "                if matches!(self.func.blocks[block.0 as usize].ops[index].kind, OpKind::Call { .. }) {\n                    return true;\n                }\n"
        "            }\n"
        "        }\n"
        "        false\n"
        "    }\n\n"
        "    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which\n")),
    # diagnostic only (behaviour unchanged): one stderr line per candidate that rule A keeps declared
    "diag": (FUSE, lambda t: rep(rep(t, CALL,
        "        if uses_as_storage(&op.kind, value) {\n            self.diag(value, op);\n            return true;\n        }\n"),
        "    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which\n",
        "    fn diag(&self, value: ValueId, op: &crate::jir::repr::Op) {\n"
        "        let name = |kind: &OpKind| format!(\"{kind:?}\").split(|c: char| !c.is_alphanumeric()).next().unwrap_or(\"\").to_owned();\n"
        "        let def = match self.defs[value.0 as usize] {\n"
        "            Some(Def::Op(block, index)) => name(&self.func.blocks[block.0 as usize].ops[index].kind),\n"
        "            Some(Def::Param(_)) => \"Param\".to_owned(),\n"
        "            None => \"None\".to_owned(),\n"
        "        };\n"
        "        let (mutable, path) = match &op.kind {\n"
        "            OpKind::BorrowView { mutable, path, .. } | OpKind::RawView { mutable, path, .. } => (\n"
        "                if *mutable { \"mut\" } else { \"shared\" },\n"
        "                if path.is_empty() { \"whole\" } else if path.iter().all(|p| matches!(p, Proj::Field { .. } | Proj::Downcast { .. })) { \"fields\" } else { \"elements\" },\n"
        "            ),\n"
        "            _ => (\"-\", \"-\"),\n"
        "        };\n"
        "        let consumer = match op.result.map(|result| self.uses[result.0 as usize].as_slice()) {\n"
        "            Some([only]) => match self.func.blocks[only.block.0 as usize].ops.get(only.position) {\n"
        "                Some(next) => name(&next.kind),\n"
        "                None => \"Terminator\".to_owned(),\n"
        "            },\n"
        "            Some([]) => \"unused\".to_owned(),\n"
        "            Some(_) => \"several\".to_owned(),\n"
        "            None => \"-\".to_owned(),\n"
        "        };\n"
        "        let indirect = self.table.is_indirect(self.func.values[value.0 as usize].ty);\n"
        "        eprintln!(\"A1DIAG def={def} use={} view={mutable} path={path} consumer={consumer} indirect={indirect}\", name(&op.kind));\n"
        "    }\n\n"
        "    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which\n")),
    # the same, sound against the member fold: the chain of field reads must end at a value that is
    # itself declared (or a parameter), or at the pointee of a pointer.
    "tight2": (FUSE, lambda t: rep(rep(t, CALL,
        "        if uses_as_storage(&op.kind, value) && !self.reads_shared_object(value, op) {\n            return true;\n        }\n"),
        "    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which\n",
        "    fn reads_shared_object(&self, value: ValueId, op: &crate::jir::repr::Op) -> bool {\n"
        "        if !matches!(op.kind, OpKind::BorrowView { .. } | OpKind::RawView { .. }) {\n            return false;\n        }\n"
        "        if !self.table.is_indirect(self.func.values[value.0 as usize].ty) {\n            return false;\n        }\n"
        "        let mut current = value;\n"
        "        for _ in 0..8 {\n"
        "            let Some(Def::Op(block, index)) = self.defs[current.0 as usize] else {\n                return false;\n            };\n"
        "            match self.func.blocks[block.0 as usize].ops[index].kind {\n"
        "                OpKind::Deref(_) => return true,\n"
        "                OpKind::Field { base, .. } => {\n"
        "                    if !self.plan.inline.contains(&base) {\n                        return true;\n                    }\n"
        "                    current = base;\n"
        "                }\n"
        "                _ => return false,\n"
        "            }\n"
        "        }\n"
        "        false\n"
        "    }\n\n"
        "    /// Whether `op` is a reference-to-raw cast of `value` to a pointer to an aggregate, which\n")),
}

names = sys.argv[1:] or list(MUTANTS)
os.makedirs(V + "/mut/bin", exist_ok=True)
orig = {FUSE: open(V + "/fuse.rs.orig").read(), SSA: open(V + "/ssa.rs.orig").read()}
for name in names:
    rel, fn = MUTANTS[name]
    path = os.path.join(SRC, rel)
    try:
        open(path, "w").write(fn(orig[rel]))
        p = subprocess.run(["cargo", "build", "--release", "--locked", "-p", "jsc-link"], cwd=SRC, capture_output=True, text=True)
        if p.returncode:
            print(name, "BUILD FAILED\n", p.stderr[-3000:])
            continue
        shutil.copy(SRC + "/target/release/jsc-link", V + "/mut/bin/" + name)
        t = subprocess.run(["cargo", "test", "--locked", "-p", "jsc-link"], cwd=SRC, capture_output=True, text=True)
        open(V + f"/mut/{name}.unit.log", "w").write(t.stdout + t.stderr)
        failed = sorted(set(l.split()[1] for l in t.stdout.splitlines() if l.startswith("test ") and l.rstrip().endswith("FAILED")))
        print(name, "built; unit tests:", "exit", t.returncode, "failed:", failed)
    finally:
        open(path, "w").write(orig[rel])
