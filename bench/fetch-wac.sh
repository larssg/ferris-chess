#!/bin/sh
set -eu

revision=c3da983fff608ded2c1c59687c030aeba6998f8a
reference=${GOCHESS_SOURCE:-../../go/chess}
source_file="$reference/wac.epd"
destination="$(dirname "$0")/data/wac.epd"
temporary="$destination.tmp"
expected=b2552f371d487c48255b8ca76236f8b180ae91ef8f6c336727d3470cf8acaa41

actual_revision=$(git -C "$reference" rev-parse HEAD)
if test "$actual_revision" != "$revision"; then
  echo "GoChess revision mismatch: expected $revision, found $actual_revision" >&2
  exit 1
fi
cp "$source_file" "$temporary"
actual=$(shasum -a 256 "$temporary" | cut -d ' ' -f 1)
if test "$actual" != "$expected"; then
  echo "WAC checksum mismatch: expected $expected, found $actual" >&2
  exit 1
fi
mv "$temporary" "$destination"
echo "installed $destination from GoChess@$revision"
