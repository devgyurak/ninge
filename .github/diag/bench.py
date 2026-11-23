#!/usr/bin/env python3
"""Throwaway CI diagnostics: how fast is this runner, alone and with all four CPUs busy?

Not part of the gate and not for merging. Standard library only.
"""

import multiprocessing
import sys
import time


def spin(_: int = 0) -> float:
    """A fixed integer loop: pure CPU, nothing but registers and a tiny heap."""
    start = time.perf_counter()
    value = 0
    for index in range(20_000_000):
        value = (value * 31 + index) & 0xFFFFFFFF
    return time.perf_counter() - start


def copy(_: int = 0) -> float:
    """Copies 256 MiB sixteen times: memory bandwidth and cache."""
    block = bytearray(256 << 20)
    start = time.perf_counter()
    for _round in range(16):
        bytes(block)
    return time.perf_counter() - start


def main() -> int:
    label = sys.argv[1] if len(sys.argv) > 1 else "bench"
    print(f"[{label}] spin x1: {spin():.2f}s")
    with multiprocessing.Pool(4) as pool:
        print(f"[{label}] spin x4: " + " ".join(f"{t:.2f}s" for t in pool.map(spin, range(4))))
    print(f"[{label}] copy x1: {copy():.2f}s")
    with multiprocessing.Pool(4) as pool:
        print(f"[{label}] copy x4: " + " ".join(f"{t:.2f}s" for t in pool.map(copy, range(4))))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
