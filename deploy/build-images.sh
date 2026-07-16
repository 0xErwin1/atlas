#!/usr/bin/env bash
set -euo pipefail

# Shared by deploy.sh (which also transfers and restarts) and the
# `build-images` devenv script (build only). Keeping the two podman build
# invocations in one place is the point: they must never drift apart.
: "${TAG:?set TAG to the image tag to build}"
REPO_ROOT="$(git rev-parse --show-toplevel)"

echo "==> Building atlas-server:${TAG}"
podman build -t "atlas-server:${TAG}" -f "${REPO_ROOT}/deploy/Containerfile.server" "${REPO_ROOT}"

echo "==> Building atlas-web:${TAG}"
podman build -t "atlas-web:${TAG}" -f "${REPO_ROOT}/deploy/Containerfile.web" "${REPO_ROOT}"
