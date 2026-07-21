#!/bin/sh
set -eu

revision=c3da983fff608ded2c1c59687c030aeba6998f8a
reference=${GOCHESS_SOURCE:-../../go/chess}
output=${GOCHESS_BIN:-bench/bin/gochess-c3da983}
cache=${TMPDIR:-/tmp}/ferrischess-go-build-cache

actual=$(git -C "$reference" rev-parse HEAD)
if test "$actual" != "$revision"; then
  echo "GoChess revision mismatch: expected $revision, found $actual" >&2
  exit 1
fi

mkdir -p "$(dirname "$output")" "$cache"
env GOCACHE="$cache" go -C "$reference" test ./...
env GOCACHE="$cache" go -C "$reference" build -trimpath -o "$(pwd)/$output" .
echo "built $output from GoChess@$revision"
