#!/bin/sh
# Metal enumerates devices by scanning the directory that holds the running
# executable. Cargo runs test binaries out of target/debug/deps, which holds
# hundreds of thousands of build artifacts, so that scan costs tens of seconds
# per binary before a single test starts. Link the binary into an empty
# directory and run it from there instead.
#
# The working directory is deliberately left alone, so relative paths such as
# the viewer's design file and --explore-record destinations still resolve.
set -u
binary=$1
shift
scratch=$(mktemp -d "${TMPDIR:-/tmp}/procgen-run.XXXXXXXX") || exit 1
isolated=$scratch/$(basename "$binary")
# A hard link costs nothing; copy only when the temporary directory is on
# another volume.
ln "$binary" "$isolated" 2>/dev/null || cp "$binary" "$isolated" || {
    rm -rf "$scratch"
    exit 1
}
"$isolated" "$@"
status=$?
rm -rf "$scratch"
exit $status
