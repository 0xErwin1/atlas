#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)

cd "$repo_root"

nix develop --command tauri-driver --help
nix develop --command WebKitWebDriver --help
nix develop --command Xvfb -help
nix develop --command sh -c '
  set -eu
  display_file=$(mktemp)
  Xvfb -displayfd 1 -screen 0 800x600x24 -nolisten tcp >"$display_file" 2>&1 &
  xvfb_pid=$!
  trap "kill $xvfb_pid 2>/dev/null || true; wait $xvfb_pid 2>/dev/null || true; rm -f $display_file" EXIT

  for _ in $(seq 1 20); do
    test -s "$display_file" && exit 0
    sleep 0.1
  done

  exit 1
'
nix develop --command sh -c '
  set -eu
  output_file=$(mktemp)
  WebKitWebDriver --port=4445 >"$output_file" 2>&1 &
  webdriver_pid=$!
  trap "kill $webdriver_pid 2>/dev/null || true; wait $webdriver_pid 2>/dev/null || true; rm -f $output_file" EXIT

  sleep 0.2
  kill -0 "$webdriver_pid"
'
