#!/bin/sh
set -eu

url=https://raw.githubusercontent.com/official-stockfish/books/master/8moves_v3.pgn.zip
destination="$(dirname "$0")/data/8moves_v3.pgn"
archive="$destination.zip.tmp"
directory="$destination.extract.tmp"
expected_archive=7e1e9dd118b4bb97d8a8b5b8a790c86e21f8509d59a27d2883767d94477be02e
expected_pgn=5835239f88cc2c7511b177c32392a69f3ede21819cf0616f80a7f907cd21d17e

mkdir -p "$(dirname "$destination")" "$directory"
curl -L --fail "$url" -o "$archive"
actual_archive=$(shasum -a 256 "$archive" | cut -d ' ' -f 1)
if test "$actual_archive" != "$expected_archive"; then
  echo "opening archive checksum mismatch: expected $expected_archive, found $actual_archive" >&2
  exit 1
fi
unzip -oq "$archive" -d "$directory"
actual_pgn=$(shasum -a 256 "$directory/8moves_v3.pgn" | cut -d ' ' -f 1)
if test "$actual_pgn" != "$expected_pgn"; then
  echo "opening PGN checksum mismatch: expected $expected_pgn, found $actual_pgn" >&2
  exit 1
fi
mv "$directory/8moves_v3.pgn" "$destination"
rm "$archive"
rmdir "$directory"
echo "installed $destination from official-stockfish/books"
