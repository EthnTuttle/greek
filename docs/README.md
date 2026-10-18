# docs/ — the project website

Static site published with GitHub Pages from `/docs` on branch `master`.

- **No build step.** Plain `*.html` + one `style.css`, system font stacks only.
  Greek text uses the `.greek` class (`Gentium` system stack with serif fallback).
- **No JS, no CDN, no external fonts** — every page must render from `file://`
  offline.
- `assets/` holds the owl icon (`owl.png`, derived from `art/ic_launcher-playstore.png`)
  and real on-device screenshots (`shot-*.png`, captured on a Pixel 6 / Android 17,
  October 2026).
- `.nojekyll` disables Jekyll so Pages serves the files as-is.

## Editing

1. Edit the HTML/CSS in place; every factual claim about features must stay
   checkable against the repo (numbers: 43 letters cards, 24-question check at
   ≥22 to pass, daily new-card cap 10, organic lane ≥85% accuracy over ≥20
   attempts across ≥2 days, tests: 81 in `greek-core`, 93 workspace-wide).
2. Date-stamp edits: pages currently read "October 2026".
3. Before committing, validate (from inside `docs/`):

```sh
python3 -c "
from html.parser import HTMLParser
import pathlib
for p in sorted(pathlib.Path('.').glob('*.html')):
    HTMLParser().feed(p.read_text(encoding='utf-8'))
    print('ok', p)"

# link/attribute check:
python3 - <<'SH'
import pathlib, re, sys
bad = []
for p in pathlib.Path('.').glob('*.html'):
    html = p.read_text(encoding='utf-8')
    for ref in re.findall(r'(?:href|src)=\"([^\"#]+)\"', html):
        if re.match(r'https?://|mailto:', ref):
            continue
        target = (p.parent / ref).resolve()
        if not target.exists():
            bad.append(f'{p}: missing {ref}')
print('links ok' if not bad else '\n'.join(bad))
sys.exit(1 if bad else 0)
SH
```

4. If you add screenshots, copy them into `assets/` and reference them with a
   relative `src`, and add an `alt` text + caption.
