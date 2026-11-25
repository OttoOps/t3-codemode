#!/usr/bin/env bash
set -uo pipefail

bin=$1
start=$2
count=$3
out=$4
shards=${SHARDS:-$(nproc)}
work_root=${WORK_ROOT:-/dev/shm}

if [[ ! $count =~ ^[1-9][0-9]{0,8}$ ]]; then
  echo "seed count must be a decimal integer from 1 to 999999999, got $count" >&2
  exit 2
fi
if [[ -z $start ]]; then
  start=$(( ${RUN_ID:?set RUN_ID or pass a seed start} % 100000000 * count ))
elif [[ ! $start =~ ^(0|[1-9][0-9]{0,17})$ ]]; then
  echo "seed start must be a decimal integer below 10^18, got $start" >&2
  exit 2
fi
test_name=pr_chaos::seeded_sequential_chaos_holds_all_invariants

if (( count < shards )); then
  shards=$count
fi
chunk=$(( (count + shards - 1) / shards ))

work=$(mktemp -d "$work_root/chaos-sweep.XXXXXX")
mkdir -p "$out/logs" "$out/failures"

run_shard() {
  local i=$1 s_start=$2 s_count=$3
  mkdir -p "$work/shard-$i"
  TMPDIR="$work/shard-$i" CHAOS_SEED_START=$s_start CHAOS_SEEDS=$s_count CHAOS_REACH=1 \
    "$bin" "$test_name" --exact --nocapture --test-threads=1 >"$out/logs/shard-$i.log" 2>&1
}

declare -a pids starts counts
for (( i = 0; i < shards; i++ )); do
  s_start=$(( start + i * chunk ))
  s_count=$(( count - i * chunk < chunk ? count - i * chunk : chunk ))
  (( s_count > 0 )) || break
  starts[i]=$s_start
  counts[i]=$s_count
  run_shard "$i" "$s_start" "$s_count" &
  pids[i]=$!
done

failed=()
declare -A shard_failed
for i in "${!pids[@]}"; do
  if ! wait "${pids[i]}"; then
    failed+=("$i")
    shard_failed[$i]=1
  fi
done

shrink_one() {
  local i=$1
  local corpus="$work/corpus-$i" log="$out/logs/shrink-$i.log" seed_env
  mkdir -p "$corpus"
  local seed
  seed=$(grep -m1 -E '^seed=[0-9]+$' "$out/logs/shard-$i.log" | cut -d= -f2)
  if [[ -n $seed ]]; then
    seed_env=(CHAOS_SEED="$seed")
  else
    seed_env=(CHAOS_SEED_START="${starts[i]}" CHAOS_SEEDS="${counts[i]}")
  fi
  env TMPDIR="$work/shard-$i" CHAOS_SHRINK=1 CHAOS_CORPUS_DIR="$corpus" "${seed_env[@]}" \
    "$bin" "$test_name" --exact --nocapture --test-threads=1 >"$log" 2>&1
  local ops
  ops=$(find "$corpus" -name '*.ops' -print -quit)
  if [[ -z $ops ]]; then
    printf "| shard %s | %s | no shrunk .ops written; see \`logs/shrink-%s.log\` |\n" \
      "$i" "${seed:-unknown}" "$i" >>"$work/rows.md"
    return
  fi
  local hash dir
  hash=$(basename "$ops" .ops)
  dir="$out/failures/$hash"
  if [[ -d $dir ]]; then
    echo "${seed:-shard $i}" >>"$dir/seeds.txt"
    return
  fi
  mkdir -p "$dir"
  cp "$ops" "$dir/$hash.ops"
  cp "$log" "$dir/shrink.log"
  echo "${seed:-shard $i}" >"$dir/seeds.txt"
  env TMPDIR="$work/shard-$i" CHAOS_OPS="$dir/$hash.ops" \
    "$bin" "$test_name" --exact --nocapture --test-threads=1 >"$dir/trace.log" 2>&1
  local class
  class=$(grep -m1 '^failure class:' "$log" | sed 's/^failure class: *//')
  printf "| \`%s\` | %s | %s |\n" "$hash" "${seed:-unknown}" "${class:-unknown}" >>"$work/rows.md"
}

for i in "${failed[@]}"; do
  shrink_one "$i"
done

{
  echo "## Swarm sweep"
  echo
  end=$(( start + count - 1 ))
  echo "Seeds \`$start..=$end\` ($count seeds) across $shards shards${CHAOS_SENTINEL:+, sentinel \`$CHAOS_SENTINEL\`}."
  echo
  if (( ${#failed[@]} == 0 )); then
    echo "All shards passed."
  else
    echo "${#failed[@]} shard(s) failed. Each shrunk failure is in the \`chaos-failures\` artifact under \`<hash>/\`."
    echo
    echo "| .ops hash | seed | failure class |"
    echo "| --- | --- | --- |"
    cat "$work/rows.md" 2>/dev/null
    echo
    echo "Unpack the artifact in the repository root and replay one:"
    echo
    first=$(find "$out/failures" -name '*.ops' -print -quit)
    ops_file="${first:+$(basename "$(dirname "$first")")/$(basename "$first")}"
    ops_file=${ops_file:-HASH.ops}
    echo '```sh'
    if [[ -n ${CHAOS_SENTINEL:-} ]]; then
      echo "CHAOS_OPS=$ops_file CHAOS_SENTINEL=$CHAOS_SENTINEL cargo test --locked --features test-sentinels --test all pr_chaos::seeded_sequential_chaos_holds_all_invariants -- --exact --nocapture"
    else
      echo "CHAOS_OPS=$ops_file cargo test --locked --test all pr_chaos::seeded_sequential_chaos_holds_all_invariants -- --exact --nocapture"
    fi
    echo '```'
  fi
  echo
  echo "### Reach hit rates"
  echo
  echo "Hits summed over the passing shards, per 1,000 seeds."
  echo
  passed_seeds=0
  passed_logs=()
  for i in "${!pids[@]}"; do
    if [[ -z ${shard_failed[$i]:-} ]]; then
      passed_seeds=$(( passed_seeds + counts[i] ))
      passed_logs+=("$out/logs/shard-$i.log")
    fi
  done
  if (( ${#passed_logs[@]} > 0 )); then
    "$(dirname "$0")/reach-table.sh" "$passed_seeds" "${passed_logs[@]}"
  fi
} >"$out/summary.md"

rm -rf "$work"
(( ${#failed[@]} == 0 ))
