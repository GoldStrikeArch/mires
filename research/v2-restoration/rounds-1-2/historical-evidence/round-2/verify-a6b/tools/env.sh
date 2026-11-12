# Shared paths for the A6b verification tools. Source it.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6b
A6=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-acddce572a795adec
BASE_LINK=$V/base-src/target/release/jsc-link
FIX_LINK=$W/target/release/jsc-link
RELINK=$V/tools/relink/target/release/relink
export V A6 W BASE_LINK FIX_LINK RELINK
