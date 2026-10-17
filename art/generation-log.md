# Generation log — nano-banana via ppq.ai

Budget: MAX 8 image API calls. **Used: 3.** Cost field returned by API: $0.00 for all three
(account promo / free-tier routing was active for `nano-banana-2-1` on 2026-10-16; the published
list price would have been $0.046 + $0.046 + $0.0678 = $0.16).

| # | client_request_id | model | res | out | verdict |
|---|---|---|---|---|---|
| 1 | icon-draft-owl-001 | nano-banana-2-1 | 1K | drafts/draft-A-owl.png | strong; slightly large margins-to-emblem ratio, asymmetric wing strokes |
| 2 | icon-draft-alpha-001 | nano-banana-2-1 | 1K | drafts/draft-B-alpha.png | clean but generic — rejected as primary (kept as fallback reference) |
| 3 | icon-final-owl-2k-001 | nano-banana-2-1 | 2K | masters/icon-master-2k.png | **FINAL** — symmetric, 3 flat colors, centered, ~50% margins |

Post-processing (local, no calls): palette snap to exact theme hexes via luminance/warmth
thresholds (the AI master carries per-pixel paper-grain noise), emblem extraction to alpha layer,
silhouette derivation, per-density Lanczos scaling — `tools/package.py`.

## Reproduce

```sh
cd art
PPQ_API_KEY=sk-... python3 tools/gen.py --model nano-banana-2-1 --resolution 2K \
  --seed-id icon-final-owl-2k-001 --out masters/icon-master-2k.png \
  --prompt "$(sed -n '/^FINAL_PROMPT/,/^END_PROMPT/p' prompts.md | sed '1d;$d')"
python3 tools/package.py
```
