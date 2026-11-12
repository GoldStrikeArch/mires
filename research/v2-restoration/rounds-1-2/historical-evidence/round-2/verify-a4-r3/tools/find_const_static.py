#!/usr/bin/env python3
"""find_const_static.py <dir>...: in every .rs file under the directories, list `const` items whose
initializer mentions the name of a `static` declared anywhere in the same directory tree, and
`static` items whose type or initializer builds a slice/str from another static. Textual, so it
over-reports; every hit is to be read."""
import re, sys
from pathlib import Path

def items(text):
    # crude item splitter: `const NAME: TY = INIT;` / `static [mut] NAME: TY = INIT;` at any depth
    for m in re.finditer(r"\b(const|static(?:\s+mut)?)\s+([A-Z][A-Z0-9_]*)\s*:\s*([^=;]+?)=\s*", text):
        kind, name, ty = m.group(1), m.group(2), m.group(3).strip()
        depth, i = 0, m.end()
        while i < len(text):
            c = text[i]
            if c in "([{": depth += 1
            elif c in ")]}": depth -= 1
            elif c == ";" and depth <= 0: break
            i += 1
        yield kind, name, ty, text[m.end():i], text.count("\n", 0, m.start()) + 1

for d in sys.argv[1:]:
    files = [p for p in Path(d).rglob("*.rs") if "/target/" not in str(p) and "/dist/" not in str(p) and "/_build/" not in str(p) and "node_modules" not in str(p)]
    statics = set()
    parsed = []
    for p in files:
        try: text = p.read_text()
        except Exception: continue
        text = re.sub(r"//[^\n]*", "", text)
        for kind, name, ty, init, line in items(text):
            parsed.append((p, kind, name, ty, init, line))
            if kind.startswith("static"): statics.add(name)
    hits = 0
    for p, kind, name, ty, init, line in parsed:
        used = sorted(s for s in statics if s != name and re.search(rf"\b{s}\b", init))
        if not used: continue
        wide = bool(re.search(r"&\s*(?:'static\s+)?(?:\[[^;\]]*\]|str)\b", ty))
        if kind == "const" or wide:
            hits += 1
            print(f"{p}:{line}: {kind} {name}: {ty[:60]} = ...{', '.join(used)}... {'[WIDE TYPE]' if wide else ''}")
    print(f"== {d}: {len(files)} files, {len(statics)} statics, {hits} hits")
