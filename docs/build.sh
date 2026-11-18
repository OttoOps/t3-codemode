#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

page=docs/src/api.md
{
  cat <<'MD'
# API reference

The site build runs `t3cm types --from tests/fixtures/t3-tools.json` and puts
its output below. Each declaration is the `signature` of a `search` catalog
entry, grouped by global. The fixture is a snapshot of T3's tools. A live
service takes its catalog from T3's current `tools/list` and from any upstreams
you configure, so its declarations can differ from this page.

```ts
MD
  cargo run --locked --quiet --bin t3cm -- types --from tests/fixtures/t3-tools.json
  echo '```'
} > "$page.tmp"
mv "$page.tmp" "$page"

"${MDBOOK:-mdbook}" build docs
