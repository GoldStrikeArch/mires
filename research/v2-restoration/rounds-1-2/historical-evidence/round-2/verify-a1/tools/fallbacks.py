#!/usr/bin/env python3
"""fallbacks.py <lane.json>...: for each link lane report, the fallback count and every item that
is not on the jir lane, with its decline reason; items whose only reason is that they are
synthesized glue (no Rust body) are counted separately."""
import json, sys

for p in sys.argv[1:]:
    d = json.load(open(p))
    items = d.get("items", [])
    other = [(i["name"], i.get("lane"), i.get("decline")) for i in items if i.get("lane") != "jir"]
    synth = [x for x in other if x[2] == "encode.synthesized"]
    real = [x for x in other if x[2] != "encode.synthesized"]
    print(f"{p.split('/verify-a1/')[-1]}: items {len(items)} fallback_count {d.get('fallback_count')} "
          f"synthesized-glue {len(synth)} declined-with-a-Rust-body {len(real)}")
    for x in real[:30]:
        print("    ", x)
