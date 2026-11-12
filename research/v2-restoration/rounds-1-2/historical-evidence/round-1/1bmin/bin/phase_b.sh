#!/usr/bin/env bash
# phase_b.sh: with the worktree in the base state, builds the base artifacts and runs the base suites.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef
bash $V/bin/artifacts.sh base
bash $V/bin/suites.sh $W base
echo PHASE-B-DONE
