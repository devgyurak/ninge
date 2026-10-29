"""A deliberately small Rust lexer shared by the ninge hooks.

The hooks need two things a regex over raw source gets wrong: code with comments and
literals removed (so `"std::fs"` in a string or `// if` in a comment does not count), and
the body of each test function (so a `}` inside a string does not end it early).

`strip` blanks comments and string/char literals **in place** — every newline survives and
every other blanked character becomes a space — so offsets and line numbers in the stripped
text are the same as in the original. This is a heuristic, not a parser: it is good enough
for gate checks and errs toward reporting. Python 3.9 compatible, standard library only.
"""

from __future__ import annotations

import re
from typing import NamedTuple

_RAW_RE = re.compile(r'(?:b|c)?r(#*)"')
_IDENT_CHAR_RE = re.compile(r"\w")

# Test attributes this repository recognizes. `proptest!` bodies carry a plain `#[test]`.
_TEST_ATTR_RE = re.compile(r"#\[\s*(?:(?:tokio|async_std)::test|test|rstest|test_case)\b[^\]]*\]")
_FN_RE = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")
_ATTR_RE = re.compile(r"#\[[^\]]*\]")
_FN_PREFIX_RE = re.compile(r"^[\s\w()]*$")


def _blank(text: str) -> str:
    return "".join("\n" if ch == "\n" else " " for ch in text)


def _ident_before(src: str, i: int) -> bool:
    return i > 0 and bool(_IDENT_CHAR_RE.match(src[i - 1]))


def _char_literal_end(src: str, i: int) -> int:
    """End offset of a char literal starting at ``i`` ('x', '\\n', '\\u{..}'), or 0 for a lifetime."""
    n = len(src)
    if i + 1 >= n:
        return 0
    if src[i + 1] == "\\":
        if src[i + 2 : i + 3] == "'":  # '\''
            return i + 4
        end = src.find("'", i + 2)
        return 0 if end == -1 else end + 1
    if i + 2 < n and src[i + 2] == "'":
        return i + 3
    return 0  # a lifetime ('a) or a loop label ('outer:)


def strip(src: str) -> str:
    """Return ``src`` with comments and string/char literals blanked, offsets preserved."""
    out: list[str] = []
    i, n = 0, len(src)
    while i < n:
        ch = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if ch == "/" and nxt == "/":
            end = src.find("\n", i)
            end = n if end == -1 else end
            out.append(_blank(src[i:end]))
            i = end
            continue
        if ch == "/" and nxt == "*":
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif src.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            out.append(_blank(src[i:j]))
            i = j
            continue
        raw = _RAW_RE.match(src, i)
        if raw and not _ident_before(src, i):
            close = src.find('"' + raw.group(1), raw.end())
            j = n if close == -1 else close + 1 + len(raw.group(1))
            out.append(_blank(src[i:j]))
            i = j
            continue
        if ch == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            j = min(j + 1, n)
            out.append(_blank(src[i:j]))
            i = j
            continue
        if ch == "'":
            end = _char_literal_end(src, i)
            if end:
                out.append(_blank(src[i:end]))
                i = end
                continue
        out.append(ch)
        i += 1
    return "".join(out)


def line_of(text: str, offset: int) -> int:
    """1-based line number of ``offset``."""
    return text.count("\n", 0, offset) + 1


def _matching_brace(text: str, open_at: int) -> int:
    depth = 0
    for j in range(open_at, len(text)):
        if text[j] == "{":
            depth += 1
        elif text[j] == "}":
            depth -= 1
            if depth == 0:
                return j
    return -1


class TestFn(NamedTuple):
    name: str
    line: int
    attrs: str  # the attribute block above the fn (stripped text)
    body: str  # the body between the outer braces (stripped text)
    body_line: int  # line of the opening brace


def test_functions(stripped: str) -> list[TestFn]:
    """Every `#[test]`-like function in already-stripped source."""
    found: list[TestFn] = []
    seen: set[int] = set()
    for attr in _TEST_ATTR_RE.finditer(stripped):
        fn = _FN_RE.search(stripped, attr.end())
        if fn is None or fn.start() in seen:
            continue
        # Only attributes and fn qualifiers may sit between the test attribute and `fn`;
        # anything else (a `{`, a `;`, a macro metavariable `fn $name`) means this
        # attribute did not belong to the fn we found.
        between = _ATTR_RE.sub(" ", stripped[attr.end() : fn.start()])
        if not _FN_PREFIX_RE.match(between):
            continue
        open_at = stripped.find("{", fn.end())
        close_at = _matching_brace(stripped, open_at) if open_at != -1 else -1
        if close_at == -1:
            continue
        seen.add(fn.start())
        block_start = max(stripped.rfind(c, 0, attr.start()) for c in ";{}") + 1
        found.append(
            TestFn(
                name=fn.group(1),
                line=line_of(stripped, fn.start()),
                attrs=stripped[block_start : fn.start()],
                body=stripped[open_at + 1 : close_at],
                body_line=line_of(stripped, open_at),
            )
        )
    return found
