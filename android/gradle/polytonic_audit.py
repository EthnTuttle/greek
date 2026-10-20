#!/usr/bin/env python3
"""Polytonic coverage audit (Wave 9, DESIGN-greek-keyboard.md Appendix R).

Fails the build when any Greek codepoint (U+0370..U+03FF, U+1F00..U+1FFF)
used by the NFC-normalized deck corpus or the morphology lexicon is NOT
emittable by the in-app keyboard, where "emittable" = every \\uXXXX escape
in KeyboardVariants.kt (EMITTABLE list + combining marks used by helpers).

Usage:
    polytonic_audit.py --variants <KeyboardVariants.kt> --data <repo data dir>

The morphology DB (data/morphology.sqlite) is a gitignored 113MB artifact;
when absent (fresh clone, CI without the data pipeline) the lexicon half is
skipped with a warning instead of failing.
"""

import argparse
import glob
import os
import re
import sqlite3
import sys
import unicodedata

GREEK_RANGES = ((0x0370, 0x03FF), (0x1F00, 0x1FFF))
ESCAPE_RE = re.compile(r"\\u([0-9A-Fa-f]{4})")


def in_greek_range(cp):
    return any(lo <= cp <= hi for lo, hi in GREEK_RANGES)


def scan_text(text, inventory):
    for ch in unicodedata.normalize("NFC", text):
        cp = ord(ch)
        if in_greek_range(cp):
            inventory.add(cp)


def load_emittable(variants_path):
    with open(variants_path, encoding="utf-8") as f:
        src = f.read()
    return {int(m.group(1), 16) for m in ESCAPE_RE.finditer(src)}


def load_inventory(data_dir):
    inventory = set()
    scanned_files = 0

    for path in sorted(glob.glob(os.path.join(data_dir, "cards", "*.json"))):
        with open(path, encoding="utf-8") as f:
            scan_text(f.read(), inventory)
        scanned_files += 1

    overrides_dir = os.path.join(data_dir, "overrides")
    if os.path.isdir(overrides_dir):
        for root, _dirs, files in os.walk(overrides_dir):
            for name in files:
                path = os.path.join(root, name)
                try:
                    with open(path, encoding="utf-8") as f:
                        scan_text(f.read(), inventory)
                    scanned_files += 1
                except (UnicodeDecodeError, OSError):
                    print(f"warning: skipped non-text override {path}", file=sys.stderr)

    db_path = os.path.join(data_dir, "morphology.sqlite")
    db_scanned = False
    if os.path.exists(db_path):
        # immutable=1: never touch WAL/SHM sidecars next to the artifact.
        conn = sqlite3.connect(
            f"file:{db_path}?mode=ro&immutable=1", uri=True)
        try:
            for query in (
                "SELECT form FROM forms",
                "SELECT headword FROM lemmas",
                "SELECT definition FROM lemmas",
            ):
                for (text,) in conn.execute(query):
                    if text:
                        scan_text(text, inventory)
            db_scanned = True
        finally:
            conn.close()
    else:
        print(
            "warning: morphology.sqlite not found under data/ — "
            "lexicon coverage skipped (deck coverage still enforced)",
            file=sys.stderr,
        )
    return inventory, scanned_files, db_scanned


def describe(cp):
    ch = chr(cp)
    try:
        name = unicodedata.name(ch)
    except ValueError:
        name = "<unnamed>"
    return f"U+{cp:04X} {ch} ({name})"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--variants", required=True, help="KeyboardVariants.kt path")
    parser.add_argument("--data", required=True, help="repo data/ directory")
    args = parser.parse_args()

    emittable = load_emittable(args.variants)
    inventory, n_files, db_scanned = load_inventory(args.data)
    covered = inventory & emittable
    missing = sorted(inventory - emittable)

    print(
        f"polytonicAudit: {scanned_files_label(n_files, db_scanned)} — "
        f"{len(inventory)} distinct Greek codepoints in inventory, "
        f"{len(covered)} covered, {len(missing)} uncovered; "
        f"keyboard emits {len(emittable)} codepoints."
    )
    if missing:
        print("polytonicAudit FAILED: keyboard cannot emit:", file=sys.stderr)
        for cp in missing:
            print(f"  {describe(cp)}", file=sys.stderr)
        print(
            "Add the missing NFC codepoints to KeyboardVariants.kt (the popup "
            "variant groups derive from its EMITTABLE list automatically).",
            file=sys.stderr,
        )
        sys.exit(1)


def scanned_files_label(n_files, db_scanned):
    return f"scanned {n_files} deck/override files" + (
        " + morphology.sqlite" if db_scanned else "")


if __name__ == "__main__":
    main()
