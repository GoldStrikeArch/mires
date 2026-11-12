#!/usr/bin/env bash
# lean-one.sh <src.rs>: one compile for tools/lean-ab.sh (environment set there).
name=$(basename "$1" .rs)
if ! SKIP_BUILD=1 bash scripts/compile_core.sh "$1" "$AB_DIR/out/$name/program.js" > "$AB_DIR/out/$name.log" 2>&1; then
  echo "$name" >> "$AB_DIR/failed.txt"
fi
