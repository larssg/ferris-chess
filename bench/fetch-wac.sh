#!/bin/sh
set -eu

revision=207a88358c414c15eec7933774d25ffe24fd4598
url="https://raw.githubusercontent.com/jdart1/arasan-chess/$revision/tests/wacnew.epd"
destination="$(dirname "$0")/data/wac.epd"
temporary="$destination.tmp"

curl -L --fail --output "$temporary" "$url"
mv "$temporary" "$destination"
echo "installed $destination from arasan-chess@$revision"
