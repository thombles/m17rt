#!/bin/bash
set -euxo pipefail
cd "$(git rev-parse --show-toplevel)"

TAG=$1

BASENAME="m17rt-${TAG}"
FILENAME="${BASENAME}.tar.xz"

git archive "${TAG}" -o "${FILENAME}" --prefix="${BASENAME}/"

echo "GENERIC_ARTIFACT|${FILENAME}|Source Code"
echo "URL|Git Tag|https://code.octet-stream.net/m17rt/shortlog/refs/tags/${TAG}|${TAG}"
