#!/usr/bin/env bash
set -euo pipefail

shards_dir=$1
out=$2
mkdir -p "$out"

for kind in caught missed timeout unviable; do
  find "$shards_dir" -name "$kind.txt" -exec cat {} + 2>/dev/null | sort -u >"$out/$kind.txt"
done

count() { grep -c . "$1" || true; }

{
  echo "## Mutation grading"
  echo
  echo "cargo-mutants over the files in \`.cargo/mutants.toml\`, graded by the \`pr_chaos\` tests."
  echo
  echo "| file | caught | missed | timeout | unviable |"
  echo "| --- | ---: | ---: | ---: | ---: |"
  cut -d: -f1 "$out"/{caught,missed,timeout,unviable}.txt | sort -u | while read -r file; do
    [[ -n $file ]] || continue
    row="| \`$file\`"
    for kind in caught missed timeout unviable; do
      row+=" | $(grep -c "^$file:" "$out/$kind.txt" || true)"
    done
    echo "$row |"
  done
  echo "| **total** | $(count "$out/caught.txt") | $(count "$out/missed.txt") | $(count "$out/timeout.txt") | $(count "$out/unviable.txt") |"
  echo
  echo "Survivors are listed in \`missed.txt\` and \`timeout.txt\` in the \`mutants-survivors\` artifact."
} >"$out/summary.md"
