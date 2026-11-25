#!/usr/bin/env bash
set -euo pipefail

seeds=$1
shift

echo "| tier | point | hits | per 1k seeds |"
echo "| --- | --- | ---: | ---: |"
awk -v seeds="$seeds" '
  /Reach summary:$/ { in_reach = 1; next }
  in_reach && /^  \[T[12]\] / {
    tier = substr($1, 2, 2); name = $2; hits[name] += $3; tiers[name] = tier
    if (!(name in order)) { order[name] = ++n; names[n] = name }
    next
  }
  in_reach { in_reach = 0 }
  END {
    for (k = 1; k <= n; k++) {
      name = names[k]
      rate = seeds > 0 ? hits[name] * 1000 / seeds : 0
      flag = hits[name] == 0 ? " **(never hit)**" : ""
      printf "| %s | `%s`%s | %d | %.2f |\n", tiers[name], name, flag, hits[name], rate
    }
  }' "$@"
