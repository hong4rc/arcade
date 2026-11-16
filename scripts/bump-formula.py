#!/usr/bin/env python3
"""Point a Homebrew formula at an Arcade release.

usage: bump-formula.py FORMULA TAG SHA256SUMS

Rewrites every release url to TAG and sets the sha256 on the line after each url
from SHA256SUMS.txt (the checksum file the release publishes). A download that is not in the
checksum file (a release built for fewer targets) keeps its url and its sha256 as they are, so
it stays on the release it was on; the list of those is printed.
"""
import re
import sys

formula, tag, sums_path = sys.argv[1:4]

sums = {}
for line in open(sums_path):
    parts = line.split()
    if len(parts) == 2:
        sums[parts[1].lstrip("*")] = parts[0]

url_re = re.compile(r'^(\s*url ")(https://github\.com/hong4rc/[^/]+/releases/download/)[^/]+/([^"]+)(")$')
sha_re = re.compile(r'^(\s*sha256 ")[0-9a-f]{64}(")$')

lines = open(formula).read().split("\n")
asset = None
keep = False
bumped = 0
kept = []
for i, line in enumerate(lines):
    m = url_re.match(line)
    if m:
        asset = m.group(3)
        keep = asset not in sums
        if keep:
            kept.append(asset)
        else:
            lines[i] = f"{m.group(1)}{m.group(2)}{tag}/{asset}{m.group(4)}"
        continue
    m = sha_re.match(line)
    if m and asset:
        if not keep:
            lines[i] = f"{m.group(1)}{sums[asset]}{m.group(2)}"
            bumped += 1
        asset = None
        keep = False

if bumped == 0:
    sys.exit("no url / sha256 pairs found in the checksum file; nothing to bump")
open(formula, "w").write("\n".join(lines))
print(f"bumped {bumped} downloads to {tag}")
for asset in kept:
    print(f"kept {asset} as it was (not in {sums_path})")
