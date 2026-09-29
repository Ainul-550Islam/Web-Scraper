#!/usr/bin/env python3
"""Check every relative markdown link in the repo resolves to a real file
(and anchors inside docs/ point at real headings). Exits non-zero on breakage
— wired into `make links` / CI so doc refactors can't leave dangling links.

Usage: python3 tools/check_links.py [root]  (default: repo root)
"""
from __future__ import annotations

import os
import re
import sys

LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")
HEADING = re.compile(r"^(#{1,6})\s+(.*)$", re.M)


def slugify(text: str) -> str:
    t = text.strip().lower()
    t = re.sub(r"`|\*|_|\[|\]", "", t)
    t = re.sub(r"[^\w\s-]", "", t)
    return re.sub(r"\s+", "-", t).strip("-")


def headings_of(path: str) -> set[str]:
    try:
        src = open(path, encoding="utf-8").read()
    except OSError:
        return set()
    return {slugify(m.group(2)) for m in HEADING.finditer(src)}


def main(root: str) -> int:
    root = os.path.abspath(root)
    bad: list[str] = []
    checked = 0
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [
            d for d in dirnames if d not in {"node_modules", ".git", "dist", "target"}
        ]
        for fn in filenames:
            if not fn.endswith(".md"):
                continue
            md = os.path.join(dirpath, fn)
            src = open(md, encoding="utf-8").read()
            rel = os.path.relpath(md, root)
            for m in LINK.finditer(src):
                target = m.group(1)
                if target.startswith(("http://", "https://", "mailto:")):
                    continue
                path_part, _, anchor = target.partition("#")
                if path_part == "" and not anchor:
                    continue
                resolved = os.path.normpath(os.path.join(dirpath, path_part))
                checked += 1
                if anchor and not path_part:
                    anchors = headings_of(md)
                    if anchor not in anchors:
                        bad.append(f"{rel}: missing anchor #{anchor}")
                    continue
                if not os.path.exists(resolved):
                    bad.append(f"{rel}: broken link → {target}")
                    continue
                if anchor and os.path.isfile(resolved) and resolved.endswith(".md"):
                    if anchor not in headings_of(resolved):
                        bad.append(f"{rel}: missing anchor #{anchor} in {target}")
    print(f"check_links: {checked} relative link(s) checked")
    for b in bad:
        print(f"  BROKEN {b}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else "."))
