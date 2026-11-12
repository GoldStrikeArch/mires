#!/usr/bin/env bash
# JSC_LINK_BIN wrapper: keeps the manifest and this crate's objects in $KEEP_DIR, then links with
# $REAL_LINK as the compiler would have.
"$RELINK" capture "$2" "$KEEP_DIR" || exit 9
exec "$REAL_LINK" "$@"
