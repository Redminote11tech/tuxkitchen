#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
version=0.2.1
# Explicit inputs avoid node_modules, target, workspaces and previous packages.
tar -czf "tuxkitchen-${version}.tar.gz" --transform="s,^,tuxkitchen-${version}/," \
  -C .. package.json bun.lock index.html tsconfig.json tsconfig.node.json \
  vite.config.ts src public src-tauri/Cargo.toml src-tauri/Cargo.lock \
  src-tauri/build.rs src-tauri/tauri.conf.json src-tauri/src \
  src-tauri/capabilities src-tauri/icons packaging/tuxkitchen packaging/tuxkitchen.desktop
makepkg "$@"
