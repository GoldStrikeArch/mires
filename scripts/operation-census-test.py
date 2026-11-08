"""Check operation metadata against real compiled fixtures and lane sidecars."""

import csv
import json
from pathlib import Path
import sys

work = Path(sys.argv[1])
reports = {}
for fixture in ("69_dead_call_dropped", "75_repr_native_vec", "107_box_pea_composition", "108_standard_identity_owner"):
    for mode in ("off", "report", "on"):
        stem = work / f"{fixture}-{mode}"
        report = json.loads((stem / "report.json").read_text())
        reports[fixture, mode] = report
        lane = json.loads((stem / "program.js.lane.json").read_text())
        assert report["jir_lane"] == lane, "embedded lane decisions differ from the actual sidecar"
        assert lane["fallback_count"] == 0
        registry = report["standard_operation_registry"]
        assert len(registry) == 39 and sum(row["native_lowering"] for row in registry) == 34
        assert next(row for row in registry if row["operation"] == "VecReserve")["native_lowering"] is False
        summary = json.loads((stem / "census/summary.json").read_text())
        assert summary["linkedItems"] == len(report["items"])
        with (stem / "census/items.csv").open() as source:
            rows = list(csv.DictReader(source))
        assert len(rows) == len(report["items"])
        assert sum(int(row["emittedItemBytes"]) for row in rows) == report["reachable_item_bytes"]
        for row in report["items"]:
            captured = row["jir_census"]
            if captured is None:
                continue
            types = {ty["id"]: ty for ty in captured["types"]}
            for call in captured["calls"]:
                assert all(ty in types for ty in call["arguments"] + [call["result"]])
            assert all(ty["kind"] is not None and ty["layout"] is not None for ty in types.values())

vec = reports["75_repr_native_vec", "off"]
entry = next(row for row in vec["items"] if row["path"] == "rust_entry")["jir_census"]
standard = [call for call in entry["calls"] if call["semantic"].startswith("Standard(")]
assert standard, "fixture did not retain any exact standard call identities"
assert all(call["callee"]["kind"] == "direct" and call["callee"]["item"] for call in standard)
assert all(call["location"]["file"].endswith("75_repr_native_vec.rs") for call in standard)
assert all(call["location"]["line"] > 0 for call in standard)

lookalikes = reports["108_standard_identity_owner", "off"]
methods = [row for row in lookalikes["items"] if row["path"].startswith("Lookalike::")]
assert len(methods) == 4
assert all(row["jir_census"]["bodySemantic"] == "Ordinary" for row in methods)
assert all(row["jir_census"]["nativeSignature"]["decline"] == "NotAStandardOperation" for row in methods)
names = {row["name"] for row in methods}
calls = [call for row in lookalikes["items"] if row["jir_census"] for call in row["jir_census"]["calls"]
    if call["callee"].get("symbol") in names]
assert len(calls) == 4 and all(call["semantic"] == "Ordinary" for call in calls)
assert any(row["jir_census"] and row["jir_census"]["bodySemantic"] == "Standard(StringWithCapacity)"
    for row in lookalikes["items"]), "real String constructor lost its identity"

box = reports["107_box_pea_composition", "on"]
entry = next(row for row in box["items"] if row["path"] == "box_pea")
assert entry["jir_census"]["bodySemantic"] == "Ordinary"
assert entry["jir_census"]["nativeSignature"]["decline"] == "NotAStandardOperation"
assert any(row["jir_census"] and row["jir_census"]["nativeSignature"].get("operation") == "BoxDrop"
    for row in box["items"])
assert box["jir_lane"]["repr"]["final_emitted"]["applied"] > 0
assert any(call["callee"].get("symbol") and call["callee"]["symbol"] not in entry["link_dependencies"]
    for call in entry["jir_census"]["calls"]), "fixture did not erase a captured call edge"

dead = reports["69_dead_call_dropped", "on"]
assert all(row["path"] != "cold" for row in dead["items"])
assert any(row["path"] == "cold" for row in reports["69_dead_call_dropped", "off"]["items"])
print("operation census: real signatures, origins, native decisions and removed edges agree")
