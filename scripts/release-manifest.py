#!/usr/bin/env python3
"""Check that a build's manifest matches the tag: release-manifest.py MANIFEST VERSION TARGET SHA."""

import json
import sys

path, version, target, sha = sys.argv[1:]
manifest = json.load(open(path))
expected = {"version": version, "target": target, "source_commit": sha, "source_dirty": False}
wrong = {k: manifest.get(k) for k, v in expected.items() if manifest.get(k) != v}
if wrong:
    sys.exit(f"::error::{path} doesn't match the tag: {wrong}")
