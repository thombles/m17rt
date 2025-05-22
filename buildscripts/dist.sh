#!/bin/bash
set -euxo pipefail
cd "$(git rev-parse --show-toplevel)"

PLATFORM=$1
TAG=$2
source buildscripts/init.sh "${PLATFORM}"

# TODO
