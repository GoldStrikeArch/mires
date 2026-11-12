#!/usr/bin/env bash
# Builds a merged, read-only copy of the three verifier corpora under 1bmin/corpus/<set>/.
S=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad
C=$S/1bmin/corpus
mkdir -p $C/r1 $C/r3new $C/fuse
# round-1 + ref_moves (r3/fx is a superset of verify1b/fx)
for f in $S/verify1b/fx/*; do b=$(basename $f); cmp -s $f $S/verify1b-r3/fx/$b || echo "R1 DIFF $b"; done
cp $S/verify1b-r3/fx/* $C/r1/
cp $S/verify1b-r3/newfx/* $C/r3new/
# fusion corpus: fx/*.rs + native/*.expected
for f in $S/verify-fuse/fx/*.rs; do
  b=$(basename $f .rs)
  cp $f $C/fuse/
  if [ "$b" != prelude ]; then
    if [ -f $S/verify-fuse/native/$b.expected ]; then cp $S/verify-fuse/native/$b.expected $C/fuse/; else echo "NO EXPECTED fuse/$b"; fi
    if [ -f $S/verify1b-r3/fusefx/$b.rs ]; then
      cmp -s $f $S/verify1b-r3/fusefx/$b.rs || echo "FUSE RS DIFF $b"
      cmp -s $S/verify-fuse/native/$b.expected $S/verify1b-r3/fusefx/$b.expected || echo "FUSE EXP DIFF $b"
    fi
  fi
done
cmp -s $S/verify-fuse/fx/ref_moves.rs $S/verify1b-r3/fx/ref_moves.rs && echo "ref_moves same in both"
cmp -s $S/verify-fuse/native/ref_moves.expected $S/verify1b-r3/fx/ref_moves.expected && echo "ref_moves.expected same"
# ref_moves is already in r1; drop the duplicate from fuse
rm -f $C/fuse/ref_moves.rs $C/fuse/ref_moves.expected
for d in r1 r3new fuse; do echo "$d: $(ls $C/$d/*.rs | grep -v prelude | wc -l) fixtures"; done
