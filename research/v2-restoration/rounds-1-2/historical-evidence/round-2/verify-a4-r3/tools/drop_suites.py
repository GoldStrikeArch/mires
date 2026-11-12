#!/usr/bin/env python3
"""drop_suites.py <src.rs> <dst.rs> <id>...: copy a generated family file without the calls to the
named suites (the suites stay defined), to look past a known crash."""
import re, sys
src, dst, ids = sys.argv[1], sys.argv[2], sys.argv[3:]
text = open(src).read()
for i in ids:
    text, n = re.subn(rf"\bs{i}\(\); ?", "", text)
    assert n == 1, (i, n)
open(dst, "w").write(text)
