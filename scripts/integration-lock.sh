#!/usr/bin/env bash
# Serializes commands that activate build/sysroot or mutate the shared build/core corpus.

: "${root:?integration-lock.sh requires the repository root}"
lock_dir="$root/build/.integration-lock"
if [ "${RCGJS_INTEGRATION_LOCK_HELD:-}" = "$lock_dir" ]; then
	return 0 2>/dev/null || exit 0
fi
mkdir -p "$root/build"

while ! mkdir "$lock_dir" 2>/dev/null; do
	owner=""
	if [ -f "$lock_dir/pid" ]; then
		read -r owner <"$lock_dir/pid" || owner=""
	fi
	if [ -z "$owner" ]; then
		sleep 1
		if [ ! -s "$lock_dir/pid" ]; then
			rm -f "$lock_dir/pid"
			rmdir "$lock_dir" 2>/dev/null || true
		fi
		continue
	fi
	if [ -n "$owner" ] && ! kill -0 "$owner" 2>/dev/null; then
		rm -f "$lock_dir/pid"
		rmdir "$lock_dir" 2>/dev/null || true
		continue
	fi
	printf 'integration lock held by pid %s; waiting\n' "${owner:-unknown}" >&2
	sleep 1
done

printf '%s\n' "$$" >"$lock_dir/pid"
export RCGJS_INTEGRATION_LOCK_HELD="$lock_dir"

release_integration_lock() {
	rm -f "$lock_dir/pid"
	rmdir "$lock_dir" 2>/dev/null || true
}
trap release_integration_lock EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
