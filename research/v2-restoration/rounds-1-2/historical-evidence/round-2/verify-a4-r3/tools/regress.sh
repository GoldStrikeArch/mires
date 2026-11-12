#!/usr/bin/env bash
# regress.sh: re-run the earlier verifiers' families on the frozen fix compiler (four modes each).
set -uo pipefail
S=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad
V=$S/verify-a4-r3
R2=$S/verify-a4-r2/src
R1=$S/verify-a4/src
run() {
  tag=$1; shift
  rm -rf "$V/out/$tag"
  bash "$V/tools/par.sh" "$tag" "$@" > "$V/logs/par-$tag.log" 2>&1
  echo "$tag: $(head -1 "$V/logs/par-$tag.log")"
}
run R_L2 "$R2"/L2/L2_*.rs
run R_L3 "$R2"/L3/L3_*.rs
run R_CS "$R2"/C/C_*.rs "$R2"/S/s*.rs
run R_W "$R2"/W/w*.rs
run R_W2 "$R2"/W2/w*.rs
run R_FB "$R2"/FB/pre*.rs "$R2"/min/*.rs
run R_R1 "$R1"/v*.rs "$R1"/min/*.rs "$R1"/ref/r*.rs "$R1"/wide/w*.rs "$R1"/wrap/*.rs
echo regress-done
