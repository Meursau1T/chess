#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_DIR="$ROOT/.cache/pikafish-src"
ENGINE_DIR="$ROOT/engine"

mkdir -p "$ROOT/.cache" "$ENGINE_DIR"

if [[ ! -d "$SOURCE_DIR/.git" ]]; then
  git clone --depth 1 https://github.com/official-pikafish/Pikafish.git "$SOURCE_DIR"
else
  git -C "$SOURCE_DIR" pull --ff-only
fi

JOBS="${JOBS:-}"
if [[ -z "$JOBS" ]]; then
  if command -v sysctl >/dev/null 2>&1; then
    JOBS="$(sysctl -n hw.logicalcpu 2>/dev/null || printf '2')"
  elif command -v nproc >/dev/null 2>&1; then
    JOBS="$(nproc)"
  else
    JOBS=2
  fi
fi

make -C "$SOURCE_DIR/src" -j"$JOBS" build ARCH=native
cp "$SOURCE_DIR/src/pikafish" "$ENGINE_DIR/pikafish"
chmod +x "$ENGINE_DIR/pikafish"
cp "$SOURCE_DIR/Copying.txt" "$ENGINE_DIR/PIKAFISH-LICENSE.txt"

for network in "$SOURCE_DIR"/src/*.nnue; do
  if [[ -f "$network" ]]; then
    cp "$network" "$ENGINE_DIR/"
  fi
done

printf 'Pikafish 已安装到 %s\n' "$ENGINE_DIR/pikafish"
