#!/usr/bin/env python3
"""Merges the line coverage of the CI test shards and checks the floor.

    python3 scripts/merge_coverage.py --floor <percent> <directory>   # exit 1: under the floor

CI splits the test run into shards (NINGE_TEST_SHARD in ./scripts/verify.sh). A shard runs a
part of the tests, so its coverage alone says nothing about the floor; it writes
`shard-<k>-of-<n>.lcov` instead. This reads those files from `<directory>`, counts a line as
covered when any shard ran it, and compares the merged line coverage with the floor. A local
run is the one shard of one and goes through here too, so the floor has one definition and one
implementation; ./scripts/verify.sh hands it COVERAGE_FLOOR, which is stated there only.

A line is a source line with a `DA` record, counted once. cargo-llvm-cov's own summary (what
`--fail-under-lines` reads) counts a line once for every function it belongs to, so the lines
of a closure count twice, and its percentage is a little lower than this one for the same run.

It fails closed. No report, a missing shard, shards that disagree on `<n>`, a file that is not
a shard report, or a report without a single line record is an error, never a pass: a shard
whose tests failed uploads nothing, and the floor must not hold over what is left.

Standard library only.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

SHARD_RE = re.compile(r"^shard-([1-9][0-9]*)-of-([1-9][0-9]*)\.lcov$")

# file -> line -> whether any shard ran it
Lines = dict[str, dict[int, bool]]


def shard_problem(names: list[str]) -> str | None:
    """Why `names` are not one complete set of shard reports, or `None` when they are. Pure."""
    if not names:
        return "no shard report found"
    strangers = sorted(name for name in names if not SHARD_RE.match(name))
    if strangers:
        return f"not a shard report: {', '.join(strangers)}"
    shards = [(int(k), int(n)) for k, n in (SHARD_RE.match(name).groups() for name in names)]
    totals = sorted({n for _, n in shards})
    if len(totals) != 1:
        return f"shards disagree on how many there are: {', '.join(str(n) for n in totals)}"
    missing = sorted(set(range(1, totals[0] + 1)) - {k for k, _ in shards})
    if missing:
        return f"missing shard {', '.join(str(k) for k in missing)} of {totals[0]}"
    if len(shards) != totals[0]:
        return f"expected {totals[0]} shard reports, found {len(shards)}"
    return None


def merge(reports: list[str]) -> Lines:
    """Union the `DA:<line>,<hits>` records of LCOV reports: a line any report ran is covered. Pure."""
    lines: Lines = {}
    for report in reports:
        source: dict[int, bool] | None = None
        for record in report.splitlines():
            if record.startswith("SF:"):
                source = lines.setdefault(record[3:], {})
            elif record.startswith("DA:") and source is not None:
                number, hits = record[3:].split(",")[:2]
                source[int(number)] = source.get(int(number), False) or int(hits) > 0
            elif record == "end_of_record":
                source = None
    return lines


def line_coverage(lines: Lines) -> tuple[int, int]:
    """`(covered, total)` lines over every file. Pure."""
    total = sum(len(source) for source in lines.values())
    covered = sum(1 for source in lines.values() for ran in source.values() if ran)
    return covered, total


def under_floor(covered: int, total: int, floor: float) -> bool:
    """Whether `covered` of `total` lines is under `floor` percent. No lines at all is. Pure."""
    return total == 0 or covered * 100 / total < floor


def main(argv: list[str]) -> int:
    if len(argv) != 4 or argv[1] != "--floor":
        print(__doc__.strip().splitlines()[2].strip(), file=sys.stderr)
        return 2
    try:
        floor = float(argv[2])
    except ValueError:
        print(f"merge_coverage: the floor must be a number, got {argv[2]!r}", file=sys.stderr)
        return 2

    directory = Path(argv[3])
    paths = sorted(path for path in directory.glob("*") if path.is_file()) if directory.is_dir() else []
    problem = shard_problem([path.name for path in paths])
    if problem:
        print(f"merge_coverage: {problem} in {directory}", file=sys.stderr)
        return 1

    reports = [path.read_text(encoding="utf-8") for path in paths]
    empty = [path.name for path, report in zip(paths, reports) if line_coverage(merge([report]))[1] == 0]
    if empty:
        print(f"merge_coverage: no line records in {', '.join(empty)}", file=sys.stderr)
        return 1

    covered, total = line_coverage(merge(reports))
    percent = covered * 100 / total
    print(f"  {covered} of {total} lines covered over {len(paths)} shard(s), {percent:.2f}%")
    if under_floor(covered, total, floor):
        print(f"merge_coverage: line coverage {percent:.2f}% is under the {floor:g}% floor", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
