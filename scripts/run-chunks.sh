#!/usr/bin/env bash
#
# Runs a program the backend split into chunks (`-Cllvm-args=js-chunk=..`).
#
#   scripts/run-chunks.sh <program.js> <name> <chunk>:<entry> [<chunk>:<entry> ...]
#
# It is scripts/run-esm.sh for a program that came out as several files. The
# differences are the two the split makes:
#
#   * there is no one program to import. Each chunk is its own module, and it is
#     the chunk that exports the entry point the caller asked it for, so the
#     generated main imports each entry from its own chunk and calls them in the
#     order they were given.
#   * the chunks reach each other, and the shared chunk, through *relative*
#     specifiers, so their file names have to survive into the run directory
#     unchanged. Each run therefore gets a directory of its own (build/esm/<name>)
#     holding the chunk files under exactly the names the backend wrote, the
#     shared chunk if there is one, and the ES module shim.
#
# No import map is involved, and that is the point of the relative specifier: the
# same files resolve under node and in a browser without one. `<name>` keeps two
# suites with the same fixture names apart.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

if [ "$#" -lt 3 ]; then
	echo "usage: run-chunks.sh <program.js> <name> <chunk>:<entry> ..." >&2
	exit 2
fi

program=$1
name=$2
shift 2

if [ ! -f "$program" ]; then
	echo "run-chunks.sh: no such program: $program" >&2
	exit 1
fi

from=$(cd -- "$(dirname -- "$program")" && pwd)
dir="$root/build/esm/$name"
shim="$root/runtime/shim.js"
esm_shim="$root/build/esm/shim.js"

# A run directory is rebuilt from scratch: a chunk left over from an earlier
# compilation would resolve, and would be the wrong file.
rm -rf "$dir"
mkdir -p "$dir"
printf '{ "type": "module" }\n' >"$dir/package.json"

# Derived once per change of the source, not once per test. Shared with
# run-esm.sh, which derives it the same way.
mkdir -p "$root/build/esm"
if [ ! -f "$esm_shim" ] || [ "$shim" -nt "$esm_shim" ]; then
	if ! node "$script_dir/make-esm-shim.mjs" "$esm_shim"; then
		echo "run-chunks.sh: could not derive the ES module shim from $shim" >&2
		exit 1
	fi
fi
cp "$esm_shim" "$dir/shim.js"

main="$dir/main.js"
: >"$main"
calls=""

for pair in "$@"; do
	chunk=${pair%%:*}
	entry=${pair#*:}
	if [ "$chunk" = "$pair" ] || [ -z "$entry" ]; then
		echo "run-chunks.sh: '$pair' is not <chunk>:<entry>" >&2
		exit 2
	fi
	if [ ! -f "$from/$chunk.js" ]; then
		echo "run-chunks.sh: the backend wrote no chunk '$chunk' beside $program" >&2
		exit 1
	fi
	cp "$from/$chunk.js" "$dir/$chunk.js"
	echo "import { $entry } from \"./$chunk.js\";" >>"$main"
	calls="$calls$entry();"$'\n'
done

# Anything the chunks import that is not itself an entry point: the shared chunk,
# and any further file a chunk names relatively. Copied by following the
# specifiers rather than by guessing the shared chunk's name, so a chunk that
# imports something else fails loudly here instead of at run time.
for pass in 1 2 3; do
	missing=0
	for file in "$dir"/*.js; do
		while read -r spec; do
			spec=${spec#./}
			if [ -n "$spec" ] && [ ! -f "$dir/$spec" ]; then
				if [ ! -f "$from/$spec" ]; then
					echo "run-chunks.sh: a chunk imports './$spec', which the backend did not write" >&2
					exit 1
				fi
				cp "$from/$spec" "$dir/$spec"
				missing=1
			fi
		done < <(sed -n 's/^import .*from "\(\.\/[^"]*\)";$/\1/p' "$file")
	done
	[ "$missing" -eq 0 ] && break
done

printf '%s' "$calls" >>"$main"

node "$main"
