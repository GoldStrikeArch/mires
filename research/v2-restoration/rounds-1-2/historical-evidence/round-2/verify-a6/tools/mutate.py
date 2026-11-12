#!/usr/bin/env python3
"""mutate.py <which> <pristine dir> <src root>: writes one mutant of jsc-link/src/link.rs and
jsc-link/src/link/statics.rs (from the pristine copies) into <src root>.

  none  unmutated
  V1    chunk fix reverted: the walk follows a reference into a binding the unit does not own
  V2    each member is filled right after its own declaration, not after every declaration
  V3    the placeholder is always `{}`, also for an array literal
  V4    self-reference detection off
  V5    Tarjan: an edge to a binding still on the stack does not lower the low link
  V6    write_chunks no longer calls split_cycles (static entry inside a cycle not refused)
  V7    whole program: cycles printed as plain lets (resolve uses the unrewritten members)
  V8    chunks: cycles printed as plain lets (write_chunks uses the unrewritten members)
  V9    the literal is not required to be exactly one object/array literal
  V10   the cycle is listed in pop order (component.reverse() removed)
  V11   a cycle is emitted before the bindings its members name outside it
"""
import sys

which, pristine, root = sys.argv[1:4]
S, L = "jsc-link/src/link/statics.rs", "jsc-link/src/link.rs"
text = {p: open(f"{pristine}/{p.split('/')[-1]}").read() for p in (S, L)}
edits = {
    "V1": (S, "if !target.kind.is_binding() || !self.members.contains(key) {",
              "if !target.kind.is_binding() {"),
    "V2": (S, "        last.js.push_str(&fill);\n    }\n    out\n",
              "        let _ = &fill;\n    }\n"
              "    for (item, declaration) in out.iter_mut().zip(&split) { item.js.push_str(&declaration.fill()); }\n"
              "    out\n"),
    "V3": (S, "            b'[' => \"[]\",\n", "            b'[' => \"{}\",\n"),
    "V4": (S, "fn names_itself(item: &LinkItem) -> bool {\n",
              "fn names_itself(item: &LinkItem) -> bool {\n    if item.kind == ItemKind::Static { return false; }\n"),
    "V5": (S, "Some(&seen) if self.on_stack.contains(key) => self.lower(name, seen),",
              "Some(&seen) if self.on_stack.contains(key) => { let _ = seen; }"),
    "V6": (L, "    statics::split_cycles(dcx, table, &owner, |at| label(at).to_owned());\n", ""),
    "V7": (L, "    let cycles = statics::initialize_cycles(dcx, &placed);\n    let mut ordered = statics::print_order(&placed, &cycles);\n",
              "    let cycles = statics::initialize_cycles(dcx, &placed);\n    let _ = &cycles;\n    let mut ordered = order(&table, &reachable);\n"),
    "V8": (L, "        let mut items = statics::print_order(&placed[at], &cycles[at]);\n",
              "        let mut items = order(table, owned);\n"),
    "V9": (S, "        if literal_len(literal) != Some(literal.len()) {", "        if false {"),
    "V10": (S, "            component.reverse();\n", ""),
    "V11": (S, "        if self.low[name] != entered {\n            return;\n        }\n",
               "        if self.low[name] != entered {\n            return;\n        }\n"
               "        if self.stack.last() != Some(&name) || names_itself {\n"
               "            // mutant: hoist a cycle to the front of everything placed so far\n"
               "            let mut component = Vec::new();\n"
               "            while let Some(member) = self.stack.pop() {\n"
               "                self.on_stack.remove(member);\n"
               "                component.push(&self.table[member]);\n"
               "                if member == name { break; }\n"
               "            }\n"
               "            component.reverse();\n"
               "            self.out.insert(0, Placed::Cycle(component));\n"
               "            return;\n"
               "        }\n"),
}
if which != "none":
    path, old, new = edits[which]
    assert text[path].count(old) == 1, f"{which}: anchor found {text[path].count(old)} times"
    text[path] = text[path].replace(old, new)
for p, t in text.items():
    open(f"{root}/{p}", "w").write(t)
print(f"{which} written")
