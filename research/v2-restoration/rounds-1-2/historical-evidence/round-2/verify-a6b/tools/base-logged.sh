#!/usr/bin/env bash
# JSC_LINK_BIN wrapper: the cf80163 linker, with every invocation counted in logs/base-link-calls.log.
echo "$(date +%T) $*" >> /private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6b/logs/base-link-calls.log
exec /private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6b/base-src/target/release/jsc-link "$@"
