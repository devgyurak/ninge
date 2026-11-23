#!/usr/bin/env bash
# Throwaway CI diagnostics, round 2: which build of one ROM test is slow on this CPU, and where
# does it spend its time? Not part of the gate and not for merging.
set -uo pipefail

TEST="given_mmdisp_rom_when_run_for_24_frames_then_both_screens_match_golden"
PROMOTE="-C llvm-args=-do-counter-promotion=true -C llvm-args=-speculative-counter-promotion-to-loop=true"
export LLVM_PROFILE_FILE="${RUNNER_TEMP:-/tmp}/probe-%p.profraw"

echo "### machine"
echo "model: $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2-)"
echo "vm size: $(curl -s --max-time 3 -H Metadata:true 'http://169.254.169.254/metadata/instance/compute/vmSize?api-version=2021-02-01&format=text' || echo '?')   virt: $(systemd-detect-virt || true)"
echo "flags of interest: $(grep -m1 '^flags' /proc/cpuinfo | tr ' ' '\n' | grep -E '^(erms|fsrm|fsrs|fzrm|avx2|avx512f|bmi2|movdiri|serialize|amx_tile|apx_f)$' | tr '\n' ' ')"
echo "glibc: $(ldd --version | head -1)"
python3 .github/diag/bench.py bench

# One timed run of the test binary alone: no nextest, no other test beside it.
run() { # label, binary, [env assignments...]
  local label="$1" binary="$2"; shift 2
  local out; out=$( { env "$@" /usr/bin/time -f '%e s wall, %U s user, %S s sys' "$binary" "$TEST" --exact >/dev/null; } 2>&1 | tail -1 )
  printf '%-52s %s\n' "$label" "$out"
}
binary() { find "$1" -maxdepth 1 -type f -name 'mmdisp-*' -executable | head -1; }

echo "### builds"
RUSTFLAGS="$PROMOTE" cargo llvm-cov nextest -p ninge-runner --test mmdisp --locked --no-report -E "test(=$TEST)" 2>&1 | grep -E 'Finished|PASS|FAIL|error' | tail -3
GATE=$(binary target/llvm-cov-target/debug/deps)
CARGO_TARGET_DIR=target/probe-plaincov cargo llvm-cov nextest -p ninge-runner --test mmdisp --locked --no-report -E "test(=$TEST)" 2>&1 | grep -E 'Finished|PASS|FAIL|error' | tail -3
PLAINCOV=$(binary target/probe-plaincov/llvm-cov-target/debug/deps)
cargo test -p ninge-runner --test mmdisp --locked --no-run 2>&1 | grep -E 'Finished|error' | tail -1
PLAIN=$(binary target/debug/deps)
CARGO_PROFILE_DEV_OPT_LEVEL=3 CARGO_TARGET_DIR=target/probe-o3 cargo test -p ninge-runner --test mmdisp --locked --no-run 2>&1 | grep -E 'Finished|error' | tail -1
PLAIN3=$(binary target/probe-o3/debug/deps)
echo "gate=$GATE"; echo "plaincov=$PLAINCOV"; echo "plain=$PLAIN"; echo "plain3=$PLAIN3"
# arm64 Linux calls a helper for every atomic operation unless LSE is assumed at compile time.
LSE=""
if [ "$(uname -m)" = "aarch64" ]; then
  echo "lse in /proc/cpuinfo: $(grep -m1 -c -w atomics /proc/cpuinfo)"
  RUSTFLAGS="$PROMOTE -C target-feature=+lse" CARGO_TARGET_DIR=target/probe-lse cargo llvm-cov nextest -p ninge-runner --test mmdisp --locked --no-report -E "test(=$TEST)" 2>&1 | grep -E 'Finished|PASS|FAIL|error' | tail -3
  LSE=$(binary target/probe-lse/llvm-cov-target/debug/deps)
  echo "lse=$LSE"
fi

echo "### timings (one test, alone)"
run "coverage + counter promotion (the gate's build)" "$GATE"
run "coverage + counter promotion, again" "$GATE"
run "coverage, no counter promotion" "$PLAINCOV"
run "no coverage, opt-level 1" "$PLAIN"
run "no coverage, opt-level 3" "$PLAIN3"
if [ -n "$LSE" ]; then
  run "coverage + promotion, +lse (inline atomics)" "$LSE"
  for i in 1 2 3 4; do run "coverage +lse, 4 in parallel #$i" "$LSE" & done; wait
fi
run "gate build, glibc without ERMS/FSRM" "$GATE" GLIBC_TUNABLES=glibc.cpu.hwcaps=-ERMS,-FSRM
run "gate build, glibc rep-movsb threshold huge" "$GATE" GLIBC_TUNABLES=glibc.cpu.x86_rep_movsb_threshold=0x4000000
run "gate build, glibc non-temporal threshold huge" "$GATE" GLIBC_TUNABLES=glibc.cpu.x86_non_temporal_threshold=0x40000000

echo "### four at once (as nextest runs them)"
for i in 1 2 3 4; do run "gate build, 4 in parallel #$i" "$GATE" & done; wait
for i in 1 2 3 4; do run "no coverage opt-level 1, 4 in parallel #$i" "$PLAIN" & done; wait

echo "### where the gate's build spends its time (stack samples)"
command -v gdb >/dev/null 2>&1 || sudo apt-get install -y gdb >/dev/null 2>&1
"$GATE" "$TEST" --exact >/dev/null 2>&1 &
PID=$!
sleep 1
: > "${RUNNER_TEMP:-/tmp}/stacks.txt"
for _ in $(seq 1 40); do
  kill -0 "$PID" 2>/dev/null || break
  sudo gdb -p "$PID" -batch -ex 'thread apply all bt 3' 2>/dev/null | grep -E '^#[0-2] ' >> "${RUNNER_TEMP:-/tmp}/stacks.txt"
done
kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null
echo "innermost frame, by samples:"
grep -E '^#0 ' "${RUNNER_TEMP:-/tmp}/stacks.txt" | sed -E 's/^#0 +(0x[0-9a-f]+ in )?//; s/ \(.*//' | sort | uniq -c | sort -rn | head -12
echo "caller (frame 1), by samples:"
grep -E '^#1 ' "${RUNNER_TEMP:-/tmp}/stacks.txt" | sed -E 's/^#1 +(0x[0-9a-f]+ in )?//; s/ \(.*//' | sort | uniq -c | sort -rn | head -8

echo "### hardware counters, if this VM exposes them"
if command -v perf >/dev/null 2>&1 || sudo apt-get install -y linux-tools-common "linux-tools-$(uname -r)" >/dev/null 2>&1; then
  sudo perf stat -e task-clock,cycles,instructions,branches,branch-misses "$GATE" "$TEST" --exact 2>&1 >/dev/null | grep -vE '^\s*$' | tail -12
else
  echo "perf is not available"
fi
