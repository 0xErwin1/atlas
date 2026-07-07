#!/usr/bin/env bash
set -euo pipefail

# Required: ssh target reachable over the WireGuard VPN, e.g. user@10.0.0.5
: "${ATLAS_DEPLOY_HOST:?set ATLAS_DEPLOY_HOST (ssh target over the VPN, e.g. user@10.0.0.5)}"
TAG="${ATLAS_DEPLOY_TAG:-latest}"
REPO_ROOT="$(git rev-parse --show-toplevel)"

echo "==> Building atlas-server:${TAG}"
podman build -t "atlas-server:${TAG}" -f "${REPO_ROOT}/deploy/Containerfile.server" "${REPO_ROOT}"

echo "==> Building atlas-web:${TAG}"
podman build -t "atlas-web:${TAG}" -f "${REPO_ROOT}/deploy/Containerfile.web" "${REPO_ROOT}"

echo "==> Transferring images to ${ATLAS_DEPLOY_HOST}"
# Ship each image in its own save|load. `podman save img1 img2` (without
# --multi-image-archive) collapses both tags onto the FIRST image and drops the
# second, so the two must be transferred separately.
for img in "atlas-server:${TAG}" "atlas-web:${TAG}"; do
  echo "  -> ${img}"
  podman save "${img}" | gzip | ssh "${ATLAS_DEPLOY_HOST}" 'gunzip | podman load'
done

echo "==> Restarting units on ${ATLAS_DEPLOY_HOST}"
ssh "${ATLAS_DEPLOY_HOST}" 'systemctl --user restart container-atlas-backend container-atlas-mcp container-atlas-web'

echo "==> Done."
