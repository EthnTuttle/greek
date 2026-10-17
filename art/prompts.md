# Prompts — exact strings spent against ppq.ai

All sent to `POST https://api.ppq.ai/v1/images/generations`, model `nano-banana-2-1`,
`size: "1:1"`, `output_format: "png"`, `n: 1`. Palette hexes come from `BRIEF.md` §1.

## Draft A — owl of Athena (1K)

> flat vector app icon, single centered emblem: stylized owl of Athena front-facing glyph
> inspired by ancient Athenian tetradrachm silver coin, bold chunky silhouette, large circular
> eyes, tiny diamond beak, matte parchment background solid color #FFF8E7, owl emblem solid
> deep umber brown #3E2723 with small warm tan #D4A574 accent discs as eyes, minimal detail,
> no feathers texture, high contrast silhouette, emblem fills about 55 percent of canvas and
> is perfectly centered with generous empty margin on all sides, no text, no letters, no border,
> no frame, no shadow, professional mobile app icon, 1:1 square

## Draft B — capital alpha with acute (1K)

> flat vector app icon, single centered emblem: one capital Greek letter Alpha with acute
> accent, bold classical serif letterform like an inscription, matte parchment background
> solid color #FFF8E7, the letter Alpha in solid deep umber brown #3E2723, small acute accent
> stroke in warm tan #D4A574 floating upper right of the letter, emblem fills about 55 percent
> of canvas perfectly centered with generous empty margin on all sides, no other text, no words,
> no border, no frame, no shadow, high contrast silhouette, professional mobile app icon, 1:1 square

FINAL_PROMPT
## Final — owl, tightened (2K) — the shipped master

> flat vector app icon, single centered emblem: stylized owl of Athena front-facing glyph
> inspired by ancient Athenian tetradrachm silver coin, bold chunky symmetric silhouette
> standing upright, two large circular eyes with cream parchment #FFF8E7 eye discs and small
> warm tan #D4A574 pupils, tiny diamond beak, short rounded ear tufts, simple engraved thin
> cream wing outlines, owl emblem solid deep umber brown #3E2723, matte parchment background
> perfectly flat solid color #FFF8E7, owl exactly horizontally and vertically centered, owl
> fills about 50 percent of canvas height with wide empty parchment margins on every side,
> only three flat colors total, no text, no letters, no border, no frame, no drop shadow,
> no gradient, minimal detail, high contrast silhouette, professional mobile app icon, 1:1 square
END_PROMPT

## Notes for future re-rolls

- Requesting the margins explicitly ("50 percent of canvas height, wide empty margins") is what
  made the emblem land inside the adaptive safe circle without manual reframing.
- Naming two exact background/foreground hexes in the prompt got the model close; exactness is
  still enforced locally by `tools/package.py` (palette snap), because the model emits a
  dithered paper grain rather than mathematically flat fills.
- `nano-banana-2-1` has no `negative_prompt` support — exclusions must be phrased positively
  ("only three flat colors total", "no text").
