#!/usr/bin/env python3
"""
Apply curated lemma-gloss overrides to a Morpheus morphology.sqlite database.

The morphology database is built from the Morpheus Greek Unicode XML dumps by
``tools/data-pipeline``. Its short glosses contain lemmatizer artifacts
(Latin stubs such as ``sum``/``ego``, transliterations such as ``gi-geno``,
or artifact strings such as ``manfaced``). Because the original XML inputs
are no longer available, a full rebuild is impossible; this script fixes the
glosses at the SQL level.

Usage:
    python3 apply_overrides.py <morphology.sqlite> <overrides.tsv>

TSV format (UTF-8):
    headword<TAB>pos<TAB>gloss

Blank lines and lines starting with ``#`` are ignored. ``pos`` must match the
``lemmas.pos`` value stored in the database (note: most verbs are stored as
``unknown`` because the Morpheus postag prefix for them is unmapped).

Behaviour:
  * Everything runs in a single transaction; on any hard error it rolls
    back and the database is left untouched.
  * Each TSV row first tries a ``headword + pos`` match
    (``UPDATE lemmas SET definition=? WHERE headword=? COLLATE NOCASE
    AND pos=? COLLATE NOCASE``).
  * If that matches no row, it falls back to a headword-only match and the
    row is counted as a ``pos-miss``.
  * If that also matches nothing, the row is a ``headword-miss`` (likely a
    typo or unicode normalisation problem in the TSV) and is reported.
  * Homograph lemmas that share ``headword + pos`` (e.g. the ``sum``/``ibo``
    pairs of εἰμί/εἶμι compounds) are all updated by one TSV row; the report
    shows how many lemma rows each TSV row touched.
  * After the updates, the FTS5 index over (headword, definition) is
    rebuilt inside the same transaction (``lemmas_fts`` is an
    external-content table, so updates are not propagated automatically).
  * After commit, if the database is in WAL mode the WAL is checkpointed
    with ``TRUNCATE`` so the main file is byte-final and safe to copy into
    ``android/app/src/main/assets/``.

Idempotent: running the script twice applies the same updates again; no
rows are added or duplicated, lemma/form counts are unchanged.

Exit status: 0 when every TSV row matched at least one lemma; 1 when one or
more TSV rows matched nothing (headword-miss).
"""

import sqlite3
import sys

DDL_FTS_REBUILD = "INSERT INTO lemmas_fts(lemmas_fts) VALUES('rebuild')"


def load_tsv(path):
    rows = []
    with open(path, encoding="utf-8") as fh:
        for lineno, raw in enumerate(fh, 1):
            line = raw.rstrip("\n")
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) != 3 or not all(p.strip() for p in parts):
                raise ValueError(
                    f"{path}:{lineno}: expected 3 non-empty tab-separated "
                    f"fields (headword/pos/gloss), got {parts!r}"
                )
            rows.append((lineno, parts[0].strip(), parts[1].strip(), parts[2].strip()))
    return rows


def main():
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    db_path, tsv_path = sys.argv[1], sys.argv[2]

    rows = load_tsv(tsv_path)
    print(f"Loaded {len(rows)} override rows from {tsv_path}")

    conn = sqlite3.connect(db_path)
    conn.isolation_level = None  # manual transaction control
    cur = conn.cursor()

    lemma_count = cur.execute("SELECT COUNT(*) FROM lemmas").fetchone()[0]
    form_count = cur.execute("SELECT COUNT(*) FROM forms").fetchone()[0]
    journal_mode = cur.execute("PRAGMA journal_mode").fetchone()[0]
    print(f" lemmas: {lemma_count}, forms: {form_count}, journal_mode: {journal_mode}")

    applied = 0            # TSV rows that matched >= 1 lemma
    lemmas_touched = 0     # total lemma rows updated
    pos_misses = []        # fell back to headword-only match
    headword_misses = []   # matched nothing at all

    try:
        cur.execute("BEGIN IMMEDIATE")
        for lineno, headword, pos, gloss in rows:
            cur.execute(
                "UPDATE lemmas SET definition=? "
                "WHERE headword=? COLLATE NOCASE AND pos=? COLLATE NOCASE",
                (gloss, headword, pos),
            )
            n = cur.rowcount
            via = "headword+pos"
            if n == 0:
                cur.execute(
                    "UPDATE lemmas SET definition=? "
                    "WHERE headword=? COLLATE NOCASE",
                    (gloss, headword),
                )
                n = cur.rowcount
                via = "headword-only"
            if n == 0:
                headword_misses.append((lineno, headword, pos, gloss))
                print(f"  MISS  line {lineno}: {headword} [{pos}] -> no lemma matched")
                continue
            if via == "headword-only":
                pos_misses.append((lineno, headword, pos, gloss))
            applied += 1
            lemmas_touched += n
            extra = f" ({n} lemmas)" if n != 1 else ""
            print(f"  ok    line {lineno}: {headword} [{pos}] via {via}{extra}")

        # lemmas_fts is an external-content FTS5 table over (headword,
        # definition); rebuild it so searches see the new glosses.
        cur.execute(DDL_FTS_REBUILD)
        cur.execute("COMMIT")
    except BaseException:
        cur.execute("ROLLBACK")
        conn.close()
        raise

    if journal_mode.lower() == "wal":
        busy = cur.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()[0]
        print(f" wal_checkpoint(TRUNCATE): busy={busy}")
    conn.execute("PRAGMA optimize")
    conn.close()

    print()
    print(f"Applied: {applied}/{len(rows)} TSV rows ({lemmas_touched} lemma rows updated)")
    print(f"pos-misses (headword-only fallback): {len(pos_misses)}")
    for lineno, hw, pos, _ in pos_misses:
        print(f"    line {lineno}: {hw} [pos '{pos}' not found]")
    print(f"headword-misses (no lemma at all): {len(headword_misses)}")
    for lineno, hw, pos, _ in headword_misses:
        print(f"    line {lineno}: {hw} [{pos}]")

    return 1 if headword_misses else 0


if __name__ == "__main__":
    sys.exit(main())
