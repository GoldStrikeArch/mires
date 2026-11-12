#!/usr/bin/env python3
"""mutate.py <which> <pristine dir> <src root>: writes one mutant of jsc-link/src/link.rs and
jsc-link/src/link/statics.rs (from the pristine copies of e7880e0) into <src root>.

The implementer's six:
  V3   the placeholder is always `{}`, also for an array literal
  V6   write_chunks no longer calls split_cycles
  V8   chunks: cycles printed as plain lets
  E1   write_chunks no longer calls evaluation_cycles
  E2   evaluation_cycles counts no read
  P1   the `__proto__` key refusal is off
The verifier's:
  E3   the import back is only looked for directly (`from` imports from `at`), not through other files
  E4   a function read around the cycle counts as well (the target need not be a binding)
  E5   only a `static` counts as a reader (a vtable or hoisted constant initializer does not)
  E6   only a `static` counts as a read target (an imported vtable or hoisted constant does not)
  E7   evaluation_cycles runs even after split_cycles reported
  E8   the read is only refused when BOTH files read a binding of the other (a one-sided read passes)
  P2   only the bare `__proto__` key is found, not the quoted one
  P3   a `__proto__` key is only found at the top level of the literal
"""
import sys

which, pristine, root = sys.argv[1:4]
S, L = "jsc-link/src/link/statics.rs", "jsc-link/src/link.rs"
text = {p: open(f"{pristine}/{p.split('/')[-1]}").read() for p in (S, L)}
edits = {
    "V3": [(S, "            b'[' => \"[]\",\n", "            b'[' => \"{}\",\n")],
    "V6": [(L, "    statics::split_cycles(dcx, table, &owner, |at| label(at).to_owned());\n", "")],
    "V8": [(L, "        let mut items = statics::print_order(&placed[at], &cycles[at]);\n",
               "        let mut items = order(table, owned);\n")],
    "E1": [(L, "    if dcx.errors() == errors {\n", "    if false && dcx.errors() == errors {\n")],
    "E2": [(S, "            if !read_at_load {\n", "            if read_at_load || !read_at_load {\n")],
    "P1": [(S, "        if has_proto_key(literal) {\n", "        if false && has_proto_key(literal) {\n")],
    "E3": [(S, "                if from == at || !reaches[from].contains(&at) {\n",
               "                if from == at || !imported[from].contains_key(&at) {\n")],
    "E4": [(S, "                    .filter(|target| target.kind.is_binding())\n",
               "                    .filter(|target| target.kind.is_binding() || true)\n")],
    "E5": [(S, "            let read_at_load = item.kind.is_binding();\n",
               "            let read_at_load = item.kind == ItemKind::Static;\n")],
    "E6": [(S, "                    .filter(|target| target.kind.is_binding())\n",
               "                    .filter(|target| target.kind == ItemKind::Static)\n")],
    "E7": [(L, "    if dcx.errors() == errors {\n", "    if dcx.errors() == errors || true {\n")],
    "E8": [(S, "    for (members, reads) in cycles {\n",
               "    for (members, reads) in cycles {\n        if reads.len() < 2 {\n            continue;\n        }\n")],
    "P2": [(S, "                if &text[start..at] == \"\\\"__proto__\\\"\" && is_key(previous, at) {\n",
               "                if false && &text[start..at] == \"\\\"__proto__\\\"\" && is_key(previous, at) {\n")],
    "P3": [(S, "        if has_proto_key(literal) {\n",
               "        if has_proto_key(literal) && {\n"
               "            // mutant: only a key of the outermost literal counts\n"
               "            let inner = &literal[1..literal.len() - 1];\n"
               "            let mut depth = 0i32;\n"
               "            let mut flat = String::new();\n"
               "            for c in inner.chars() {\n"
               "                match c {\n"
               "                    '{' | '[' => depth += 1,\n"
               "                    '}' | ']' => depth -= 1,\n"
               "                    _ => {}\n"
               "                }\n"
               "                if depth == 0 && !matches!(c, '}' | ']') {\n"
               "                    flat.push(c);\n"
               "                }\n"
               "            }\n"
               "            has_proto_key(&format!(\"{{{flat}}}\"))\n"
               "        } {\n")],
}
if which != "none":
    for path, old, new in edits[which]:
        assert text[path].count(old) == 1, f"{which}: anchor found {text[path].count(old)} times"
        text[path] = text[path].replace(old, new)
for p, t in text.items():
    open(f"{root}/{p}", "w").write(t)
print(f"{which} written")
