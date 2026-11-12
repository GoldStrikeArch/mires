#!/usr/bin/env bash
# build_mine.sh: build the worktree, its five sysroots, then the full fixture matrix (arm "mine").
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef
cd $W
bash scripts/build.sh --locked > $V/logs/build-mine.log 2>&1; echo "build exit=$?"
shasum -a 256 target/release/librustc_codegen_mires.dylib
bash $V/bin/sysroots.sh $W mine
bash $V/bin/matrix.sh $W mine
