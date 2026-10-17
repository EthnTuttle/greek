# ART BRIEF — Ancient Greek Learning App · Launcher Icon + Play Store Asset

Date: 2026-10-16 · Status: direction locked, generation in progress

## 1. Brand palette (extracted verbatim from app source)

| Token | Hex | Source | Role for icon |
|---|---|---|---|
| ParchmentLight | `#FFF8E7` | Color.kt / colors.xml `parchment_light` | icon background (main parchment) |
| ParchmentDark | `#F5E6D3` | Color.kt | parchment shading / vignette ring |
| AmberAccent | `#D4A574` | Color.kt / colors.xml `amber_accent` | warm accent (highlights only — too low-contrast vs parchment for the emblem itself) |
| DeepBrown | `#3E2723` | Color.kt / colors.xml `deep_brown` | **emblem primary color** (umber; contrast vs #FFF8E7 ≈ 12.6:1) |
| OliveDark | `#6B7A3F` | Color.kt | not used in icon (low contrast, off-silhouette) |
| DarkBackground | `#1A1410` | Color.kt | dark-theme reference only |

Typography: Greek text = serif (`FontFamily.Serif`, Type.kt) → letterform-based art must be a bold serif.
Current placeholder to replace: `mipmap-anydpi-v26/ic_launcher.xml` references `@color/ic_launcher_background` (#5C6BC0 indigo) and `@color/ic_launcher_foreground` (#FFFFFF) from `values/ic_launcher_colors.xml` — both solid colors, no art. Final XML must point at real drawables.

## 2. Concept evaluation (48dp legibility = deciding test)

| Concept | Verdict | Why |
|---|---|---|
| **Owl of Athena glyph** | **PRIMARY** | Single bold silhouette; Athenaze is set in Attica — the owl (Athenian tetradrachm) *is* Athens/learning; survives 48dp with zero fine detail; non-text, unique shape language among edu apps. |
| Capital Alpha Ά (with acute) | FALLBACK | Clean, instantly "Greek", but letterforms are generic (every dictionary app); use only if owl reads poorly. Serif Α in DeepBrown would be trivially producible by hand. |
| Ionic column capital | REJECT | Volutes/abacus = fine detail that vanishes below 24dp; reads as architecture app at small size. |
| Laurel + Alpha | REJECT (as primary) | Two competing elements; laurel leaf detail dies at 48dp. Allowed only as optional large-size flourish — skipped. |
| Meander border | REJECT | Adds noise at small size; violates "no border" icon guidance; risks clipping under adaptive masks (circle/squircle/teardrop). |

Justification (app-icon best practice): a launcher icon must be one bold, high-contrast silhouette on a
quiet field — recognizable at 48dp with no text, no border, no gradients that mud at small size, and
no detail finer than ~24dp. The owl glyph is a single dark shape (CONTRAST 12.6:1 on #FFF8E7) with a
semantic match to Athenaze/Athens that no competitor letterform has. Fallback Ά exists as the safety net.

## 3. Style scaffold for generation

"flat vector app icon, single centered emblem, matte parchment background #FFF8E7, deep umber #3E2723
<CONCEPT> emblem with subtle warm tan #D4A574 accent, minimal detail, high contrast silhouette, no text,
no border, professional mobile app icon, 1:1"

Post-processing guarantees exact palette: generated art is palette-snapped to the hexes above so the
deliverables are exactly on-theme regardless of model drift.

## 4. Geometry spec (Android adaptive icon)

- Full adaptive canvas: 108×108dp. All four edges can be cropped by OEM masks to the inner
  **72dp safe circle** (diameter ⅔ of canvas, centered) → emblem must live inside it.
- Layers at 4×dp: **432×432px**. Safe circle at that scale = centered 288×288px disc.
- Foreground: emblem on transparent, all opaque pixels inside the 288px disc.
- Background: full-bleed 432×432 parchment (no character art — parallax layer).
- Monochrome (Android 13/API 33+ themed icons): same silhouette, alpha-only, filled on tint.

## 5. Deliverables (all under `art/` in this worktree)

| Path | Size | Content | Destined for `android/app/src/main/res/` |
|---|---|---|---|
| `masters/icon-master.png` | 2048×2048 | final full-bleed square master | (source of all below; not shipped) |
| `adaptive/ic_launcher_foreground.png` | 432×432 RGBA | emblem only, transparent padding | `mipmap-xxxhdpi/ic_launcher_foreground.png` + scaled fd/bg sets below |
| `adaptive/ic_launcher_background.png` | 432×432 opaque | parchment field | `mipmap-xxxhdpi/ic_launcher_background.png` |
| `adaptive/ic_launcher_monochrome.png` | 432×432 RGBA | solid-alpha emblem silhouette | `mipmap-xxxhdpi/ic_launcher_monochrome.png` |
| `playstore/ic_launcher-playstore.png` | 512×512 opaque | full-bleed square, art inside safe area | Play Console listing asset |
| `legacy/mipmap-mdpi/ic_launcher.png` | 48×48 | full-bleed square | `mipmap-mdpi/ic_launcher.png` |
| `legacy/mipmap-hdpi/ic_launcher.png` | 72×72 | ↑ | `mipmap-hdpi/ic_launcher.png` |
| `legacy/mipmap-xhdpi/ic_launcher.png` | 96×96 | ↑ | `mipmap-xhdpi/ic_launcher.png` |
| `legacy/mipmap-xxhdpi/ic_launcher.png` | 144×144 | ↑ | `mipmap-xxhdpi/ic_launcher.png` |
| `legacy/mipmap-xxxhdpi/ic_launcher.png` | 192×192 | ↑ | `mipmap-xxxhdpi/ic_launcher.png` |
| `adaptive/densities/ic_launcher_foreground-{density}.png` | 108/162/216/324/432 | per-density foreground (mdpi→xxxhdpi) | matching `mipmap-*/ic_launcher_foreground.png` |
| `adaptive/densities/ic_launcher_background-{density}.png` | 108/162/216/324/432 | per-density background | matching `mipmap-*/ic_launcher_background.png` |
| `adaptive/densities/ic_launcher_monochrome-{density}.png` | 108/162/216/324/432 | per-density monochrome | matching `mipmap-*/ic_launcher_monochrome.png` |
| `res/mipmap-anydpi-v26/ic_launcher.xml` | — | adaptive-icon XML referencing `@mipmap/ic_launcher_background`, `@mipmap/ic_launcher_foreground`, `@mipmap/ic_launcher_monochrome` | replaces existing file (currently both slots are `@color/…`) |
| `res/CHANGELOG.md` | — | res-change manifest incl. deletion note for `values/ic_launcher_colors.xml` | applied at copy step |

Adaptive layer files ship at **all five densities** (mdpi 108 / hdpi 162 / xhdpi 216 / xxhdpi 324 /
xxxhdpi 432 px) so no runtime scaling blur. Legacy squares are independent art (full-bleed, no mask).

See `res/CHANGELOG.md` for the exact copy commands and `pipeline-notes.md` for generation provenance.
