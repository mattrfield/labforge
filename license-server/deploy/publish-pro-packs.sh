#!/bin/sh
# Builds pro lab packs and uploads them to the license server.
#
#   publish-pro-packs.sh <ssh-host> <https://license-domain> <pro-labs-dir> [remote-packs-dir]
#
# <pro-labs-dir> holds one folder per pro lab (<id>/lab.yaml, compose.yaml, ...) and lives OUTSIDE
# this public repo, for example a private labforge-pro-labs repo. Needs python3 and rsync locally.
set -eu

if [ "$#" -lt 3 ]; then
  echo "usage: $0 <ssh-host> <https://license-domain> <pro-labs-dir> [remote-packs-dir]" >&2
  exit 2
fi
HOST=$1
BASE=${2%/}
LABS=$3
REMOTE=${4:-labforge/license-server/deploy/packs}
SCRIPTS="$(cd "$(dirname "$0")/../.." && pwd)/scripts"
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

python3 "$SCRIPTS/build_lab_packs.py" --labs-dir "$LABS" --tier pro --out "$OUT" --base-url "$BASE/v1/packs"

# Packs first, then the index that points at them, so the index never names a missing pack.
rsync -av --exclude index.json "$OUT"/ "$HOST:$REMOTE/"
rsync -av "$OUT/index.json" "$HOST:$REMOTE/index.json"
echo "Published. Check: curl -s $BASE/v1/index | head"
