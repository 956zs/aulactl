#!/usr/bin/env bash
# Packs integrations/osu-render-plugin into dist/aula-keys-osu-render-plugin-<version>.zip
# (plugin.json at the top) for `npm run plugin -- install <zip>` in osu-render.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
src="$repo/integrations/osu-render-plugin"
version=$(sed -n 's/^ *"version": *"\([^"]*\)".*/\1/p' "$src/plugin.json" | head -1)
[ -n "$version" ] || { echo "no version in $src/plugin.json" >&2; exit 1; }
mkdir -p "$repo/dist"
out="$repo/dist/aula-keys-osu-render-plugin-$version.zip"
rm -f "$out"
(cd "$src" && zip -q -X "$out" plugin.json index.mjs README.md)
echo "$out"
