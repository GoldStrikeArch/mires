#!/usr/bin/env python3
"""finditem.py <json> <needle>: every object in a JSON report that mentions <needle>, with its path."""
import json, sys
data = json.load(open(sys.argv[1]))
needle = sys.argv[2]
def walk(x, path):
    if isinstance(x, dict):
        if any(isinstance(v, str) and needle in v for v in x.values()):
            print('/'.join(path), json.dumps(x)[:700])
        for k, v in x.items():
            walk(v, path + [str(k)])
    elif isinstance(x, list):
        for i, v in enumerate(x):
            walk(v, path + [str(i)])
walk(data, [])
