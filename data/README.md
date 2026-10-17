# Morphology data

## Provenance — read this first

`morphology.sqlite` is derived from the **Morpheus Greek Unicode** XML dumps
(Perseus morphology/lemmatizer output), **not from LSJ**. Each lemma carries a
single *short gloss* that originated as the lemmatizer's one-line hint
(Morpheus re-used LSJ *short-def* style strings and headword stubs). Older
documentation claimed the glosses came from LSJ — that is wrong; no LSJ text was
ever ingested into this database.

The original XML inputs (`/tmp/morpheus-data/MorpheusGreek*.xml`, ~958k
analyses) are no longer available, so **a full rebuild is not possible**. All
corrections to lemma glosses happen at the SQL level via curated overrides
(see below).

Builder: `tools/data-pipeline` (Rust). Do not re-run it — without the XML
inputs it would only produce an empty database.

## Known quirks of the stored data

- `lemmas.frequency_rank` is **not** a corpus-frequency rank. The pipeline set
  it to the number of distinct inflected forms stored per lemma
  (`UPDATE lemmas SET frequency_rank = (SELECT COUNT(*) FROM forms ...)`).
  Rank 1 = one form (not "most frequent"); ποιέω (1209 forms) sorts first,
  while ὁ/καί sort low. Treat it as "paradigm size".
- `lemmas.pos` for most verbs is `unknown`, because the pipeline's
  postag→pos mapping has no entry for several Morpheus postag prefixes.
  Nouns/adjectives/pronouns are mostly tagged correctly.
- Homograph lemmas exist as multiple `lemmas` rows sharing one headword
  (e.g. the εἰμί-compound "to be" / εἶμι-compound "to go" pairs, and
  λέγω¹/²/³). There is no column that distinguishes them; lookups should not
  assume `headword` is unique.
- ~2,000 lemmas ship without any gloss (`definition` empty).

## Gloss overrides (mechanism)

Curated corrections live in `data/overrides/lemma_glosses.tsv`:

```
headword<TAB>pos<TAB>gloss
```

`pos` must equal the stored `lemmas.pos` value (usually `unknown` for verbs;
see quirks above). One TSV row updates **every** lemma row matching
`headword + pos`, so homograph pairs (e.g. πάρειμι "to be present" /
"to pass by") are given a combined gloss in both rows by design.

Apply (or re-apply) with:

```sh
python3 tools/data-pipeline/overrides/apply_overrides.py \
    data/morphology.sqlite data/overrides/lemma_glosses.tsv
```

The script:

- applies all updates in one transaction (rollback on error),
- reports per-row matches, pos-misses (falls back to headword-only match)
  and headword-misses (no such lemma — usually a TSV typo), and exits 1 if
  any headword matched nothing,
- rebuilds the `lemmas_fts` FTS5 index (which covers `definition`) inside the
  same transaction,
- checkpoints the WAL with `TRUNCATE` afterwards, so the main file is final,
- is idempotent: re-running applies the same updates, never duplicates.

After changing the database, refresh the Android asset copy:

```sh
cp data/morphology.sqlite android/app/src/main/assets/morphology.sqlite
cmp data/morphology.sqlite android/app/src/main/assets/morphology.sqlite
```

Both files are git-ignored (binary, regenerable/copied), so they must be
distributed outside git.

### Current coverage

The initial override set (80 TSV rows → 88 lemma rows) covers:

- every wrong or materially misleading gloss found by auditing the **top 200
  lemmas** sorted by `frequency_rank DESC` (e.g. εἰμί "I have" → "to be;
  to exist", γίγνομαι "gi-geno" → "to become…", πράσσω "to pass over" →
  "to do, accomplish…", τρέφω "to thicken" → "to nourish, rear…"),
- high-visibility words outside the top 200 (ἄνθρωπος "manfaced" →
  "man, human being", λόγος, χώρα), and
- artifacts found by scanning all 36k lemmas for empty glosses, Latin stubs
  (`sum`, `ego`, `ibo`, `videor mihi`), and fragments (`sc`, `sw`).

Known-unfixed: the remaining ~2,000 of 2,057 empty definitions, long-tail
stale glosses below rank ~300 in the paradigm-size ordering, and the
un-splittable homograph glosses noted above.

## Future work: real LSJ ingestion

To replace Morpheus short glosses with proper dictionary entries, ingest the
LSJ Greek–English lexicon directly:

- Source: the **Perseus TEI XML edition of LSJ**
  (`greek_lsj`, distributed by the Perseus Digital Library,
  https://github.com/PerseusDL/lexica — CITE/CTS canons).
- Licensing: the underlying lexicon is the 9th edition of Liddell–Scott–Jones
  (1940), which is in the **public domain**; the Perseus TEI markup is
  published under a permissive Creative Commons license. Either way the text
  may be redistributed with the app, but verify the exact CC variant ship the
  NOTICE file if required.
- Suggested approach: add a new ingestion step that maps Perseus headwords to
  `lemmas` rows (match on NFC-normalized headword, keep `pos` agreement),
  stores the LSJ short definition in a new column (keep Morpheus' gloss as
  fallback), and rebuilds `lemmas_fts`. The overrides TSV then remains as a
  final curation layer on top.
