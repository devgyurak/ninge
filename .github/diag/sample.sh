#!/usr/bin/env bash
# Throwaway CI diagnostics: one snapshot of what the machine is doing. Not part of the gate.
echo "=== $(date -u +%T) loadavg $(cat /proc/loadavg)"
# user nice system idle iowait irq softirq steal: cumulative jiffies over all CPUs
head -1 /proc/stat
for kind in cpu io memory; do
  [ -r "/proc/pressure/$kind" ] && echo "psi $kind: $(grep some "/proc/pressure/$kind")"
done
grep -E 'MemAvailable|SwapFree|Dirty:' /proc/meminfo | tr -s ' ' | tr '\n' ' '; echo
grep -E 'cpu MHz' /proc/cpuinfo | awk '{printf "%s ", $4} END {print "MHz"}'
ps -eo pcpu,pmem,nlwp,etimes,comm --sort=-pcpu | head -9
