#!/usr/bin/env python3
"""Small, read-only quality gate for the bundled flashcard JSON files."""

from __future__ import annotations

import json
import re
import sys
import unicodedata
from collections import defaultdict
from pathlib import Path

CARDS_DIR = Path(__file__).resolve().parent.parent / "data" / "cards"
GREEK_TOKEN = re.compile(r"[\u0300-\u036f\u0370-\u03ff\u1f00-\u1fff]+")
CIRCUMFLEX = "\u0342"
CIRCUMFLEX_BASES = set("αηιωυΑΗΙΩΥ")

# Lexical quantities cannot be inferred from Unicode alone. These explicit
# entries catch the reported classes without treating every ᾶ/ῖ/ῦ as short.
INVALID_WORD_ACCENTS = {
    "μᾶλα": "use μάλα: the first alpha is short and cannot take a circumflex",
    "πράγμα": "use πρᾶγμα: the accented long penult takes a circumflex",
}

# Used only to compare headwords within one deck; accents remain significant.
LEADING_ARTICLES = {
    "ὁ", "ἡ", "τό", "τὸ", "τόν", "τὸν", "τήν", "τὴν", "τοῦ", "τῆς",
    "τῷ", "τῇ", "οἱ", "αἱ", "τά", "τὰ", "τούς", "τοὺς", "τάς", "τὰς",
}


def normalized_headword(front: str) -> str:
    value = unicodedata.normalize("NFC", front).strip()
    value = re.split(r"\s+(?:—|–)\s+|[,(]", value, maxsplit=1)[0].strip()
    words = value.split()
    if len(words) > 1 and words[0] in LEADING_ARTICLES:
        words = words[1:]
    # lower(), unlike casefold(), does not erase iota-subscript distinctions.
    return unicodedata.normalize("NFC", " ".join(words).lower())


def accent_errors(text: str) -> list[str]:
    """Return impossible accent/quantity errors in words, not bare exemplars."""
    errors: list[str] = []
    for match in GREEK_TOKEN.finditer(unicodedata.normalize("NFC", text)):
        token = match.group(0)
        if len(token) < 2:
            # Chapter 4 deliberately uses bare diacritic/glyph exemplars.
            continue
        folded = token.lower()
        if folded in INVALID_WORD_ACCENTS:
            errors.append(f"{token!r}: {INVALID_WORD_ACCENTS[folded]}")
        decomposed = unicodedata.normalize("NFD", token)
        last_base = ""
        for char in decomposed:
            if not unicodedata.combining(char):
                last_base = char
            elif char == CIRCUMFLEX:
                if last_base in "εοΕΟ":
                    errors.append(f"{token!r}: circumflex on always-short {last_base!r}")
                elif last_base not in CIRCUMFLEX_BASES:
                    errors.append(f"{token!r}: circumflex on non-vowel {last_base!r}")
    return errors


def main() -> int:
    errors: list[str] = []
    duplicate_groups = 0
    duplicate_cards = 0
    total = 0

    paths = sorted(CARDS_DIR.glob("*.json"))
    if not paths:
        print(f"ERROR: no card JSON files found in {CARDS_DIR}", file=sys.stderr)
        return 1

    print("CARD DATA LINT")
    for path in paths:
        try:
            cards = json.loads(path.read_text(encoding="utf-8"))
            if not isinstance(cards, list):
                raise TypeError("top level is not an array")
        except (OSError, ValueError, TypeError) as exc:
            print(f"ERROR {path.name}: cannot parse card array: {exc}", file=sys.stderr)
            errors.append(path.name)
            continue

        print(f"DECK {path.name}: {len(cards)} cards")
        total += len(cards)
        by_headword: dict[str, list[dict]] = defaultdict(list)
        seen_ids: set[str] = set()
        for position, card in enumerate(cards, start=1):
            if not isinstance(card, dict):
                errors.append(f"{path.name} #{position}: not an object")
                continue
            card_id = str(card.get("id", f"#{position}"))
            if card_id in seen_ids:
                errors.append(f"{path.name} {card_id}: duplicate card ID")
            seen_ids.add(card_id)
            for field in ("front", "back"):
                value = card.get(field)
                if not isinstance(value, str) or not value.strip():
                    errors.append(f"{path.name} {card_id}: blank/non-string {field}")
                    continue
                for detail in accent_errors(value):
                    errors.append(f"{path.name} {card_id} {field}: {detail}")
            front = card.get("front")
            if isinstance(front, str) and front.strip():
                by_headword[normalized_headword(front)].append(card)

        for headword, group in sorted(by_headword.items()):
            if len(group) < 2:
                continue
            duplicate_groups += 1
            duplicate_cards += len(group)
            ids = ", ".join(str(card.get("id", "?")) for card in group)
            print(f"WARNING {path.name}: duplicate headword {headword!r} ×{len(group)} [{ids}]")

    print(
        f"SUMMARY: {total} cards; {duplicate_groups} duplicate-headword groups "
        f"({duplicate_cards} cards); {len(errors)} errors"
    )
    if errors:
        for error in errors:
            print(f"ERROR {error}", file=sys.stderr)
        print("RESULT: FAIL (blank fields, invalid accents, duplicate IDs, or parse errors)")
        return 1
    print("RESULT: PASS (duplicate headwords are warnings only)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
