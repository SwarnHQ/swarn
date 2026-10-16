#!/usr/bin/env python3
"""Fail if any crate shipped in a Swarn binary has a license we have not reviewed.

Walks normal and build dependencies (not dev) of every workspace member for the macOS arm64
target. Run from the repo root: python3 -I scripts/license_audit.py
"""

import json
import re
import subprocess
import sys

TARGET = "aarch64-apple-darwin"

# Any license expression made only of these is fine to ship.
PERMISSIVE = {
    "MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "Zlib", "Unicode-3.0", "Unlicense",
    "0BSD", "CC0-1.0", "BSL-1.0", "ISC", "Apache-2.0 WITH LLVM-exception",
}

# Reviewed exceptions, with the reason. Adding a crate here is a decision, so say why.
REVIEWED = {
    "cbindgen": "MPL-2.0; build-time code generator for gpui-pre-apple, not linked into binaries",
    "option-ext": "MPL-2.0; file-level copyleft, used unmodified via dirs-sys (zed-font-kit)",
}


def acceptable(expr: str) -> bool:
    """Evaluate the SPDX expression with each license id replaced by whether it is permissive."""
    tokens = re.findall(r"\(|\)|/|[^\s()/]+(?:\s+WITH\s+[^\s()/]+)?", expr)
    words = {"(": "(", ")": ")", "AND": "and", "OR": "or", "/": "or"}
    # eval only ever sees True, False, and, or and parentheses built here.
    return bool(tokens) and eval(" ".join(words.get(t, str(t in PERMISSIVE)) for t in tokens))


def main() -> int:
    meta = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--filter-platform", TARGET]))
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}

    shipped, stack = set(), list(meta["workspace_members"])
    while stack:
        pid = stack.pop()
        if pid in shipped:
            continue
        shipped.add(pid)
        for dep in nodes[pid]["deps"]:
            if any(k["kind"] in (None, "build") for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])

    failures = []
    third_party = [packages[p] for p in shipped if packages[p]["source"]]
    for p in third_party:
        expr = p["license"] or ""
        if p["name"] in REVIEWED or (expr and acceptable(expr)):
            continue
        failures.append(f"{p['name']} {p['version']}: {expr or 'no license field'}")

    print(f"{len(third_party)} third-party crates checked for {TARGET}")
    for f in sorted(failures):
        print(f"  NOT REVIEWED  {f}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
