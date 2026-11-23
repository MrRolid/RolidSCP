#!/bin/bash
# Unpack a container image (amd64) into a folder, without docker or root:
# the layers come straight from the registry.
#
#   scripts/compat/rootfs.sh IMAGE DEST
#
# IMAGE  e.g. debian:12, ubuntu:22.04, fedora:43, opensuse/leap:15.6,
#        archlinux:latest, alpine:3.22 (Docker Hub; "library/" is added
#        for official images)
# DEST   a new folder
set -euo pipefail
[ $# = 2 ] || { sed -n '2,10p' "$0"; exit 2; }
IMAGE=$1 DEST=$2
[ -e "$DEST" ] && { echo "$DEST exists" >&2; exit 1; }
repo=${IMAGE%:*} tag=${IMAGE##*:}
[[ $repo == */* ]] || repo=library/$repo
REG=https://registry-1.docker.io
token=$(curl -fsS "https://auth.docker.io/token?service=registry.docker.io&scope=repository:$repo:pull" |
  python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])')
H=(-H "Authorization: Bearer $token")
ACCEPT='application/vnd.oci.image.index.v1+json,application/vnd.docker.distribution.manifest.list.v2+json,application/vnd.oci.image.manifest.v1+json,application/vnd.docker.distribution.manifest.v2+json'
top=$(curl -fsSL "${H[@]}" -H "Accept: $ACCEPT" "$REG/v2/$repo/manifests/$tag")
digest=$(python3 -c 'import json,sys
m = json.loads(sys.argv[1])
if "manifests" in m:
    print(next(x["digest"] for x in m["manifests"] if x["platform"]["architecture"] == "amd64" and x["platform"]["os"] == "linux"))
else:
    print("")' "$top")
if [ -n "$digest" ]; then
  man=$(curl -fsSL "${H[@]}" -H "Accept: $ACCEPT" "$REG/v2/$repo/manifests/$digest")
else
  man=$top
fi
mkdir -p "$DEST"
for layer in $(python3 -c 'import json,sys; print(" ".join(l["digest"] for l in json.loads(sys.argv[1])["layers"]))' "$man"); do
  echo "layer $layer" >&2
  # No device files and no owners (one user in a user namespace).
  curl -fsSL "${H[@]}" "$REG/v2/$repo/blobs/$layer" | tar -xz -C "$DEST" --no-same-owner --no-same-permissions --exclude='dev/*' 2>/dev/null || true
  # Files deleted by this layer (whiteouts).
  find "$DEST" -name '.wh.*' -print0 | while IFS= read -r -d '' w; do
    rm -rf "$(dirname "$w")/$(basename "$w" | sed 's/^\.wh\.//')" "$w"
  done
done
chmod -R u+rwX "$DEST"
du -sh "$DEST" >&2
