# Pipeline Notes — nano-banana via ppq.ai

## Credential discovery
- **Key found: YES.** Stored in `~/.local/share/opencode/auth.json` under `"ppq"` (also referenced
  as `apiKey` inside `~/.config/opencode/opencode.jsonc` provider config). Read-only extraction;
  the key is never echoed to logs — `tools/gen.py` loads it from `$PPQ_API_KEY` or falls back to
  parsing `auth.json` itself.
- No `PPQ_API_KEY` env var existed; `~/.ppq*` / `~/.config/ppq*` do not exist.

## API facts (verified 2026-10-16 against live service + https://ppq.ai/api-docs + /llms.txt)
- Base URL: `https://api.ppq.ai` — OpenAI-compatible.
- Auth: `Authorization: Bearer <key>` (keys prefixed `sk-`).
- Image endpoint: `POST https://api.ppq.ai/v1/images/generations` (synchronous; image bytes inline).
- Image models listed by `GET /v1/models?type=image` (works, HTTP 200).
- nano-banana family present as flat aliases — each with `resolution_options: ["1K","2K","4K"]`,
  `accepts_aspect_ratio`, `accepts_output_format`, `accepts_n`; **no** `negative_prompt` support:
  | alias | upstream | 1K price |
  |---|---|---|
  | `nano-banana-2-1` | Gemini nano-banana 2.1 | $0.046 |
  | `nano-banana-2` | Gemini 3.1 Flash Image | $0.092 |
  | `nano-banana-pro` | Gemini 3 Pro Image | $0.1035 |
- Request JSON: `{ "model", "prompt", "size": "1:1", "resolution": "1K"|"2K"|"4K",
  "output_format": "png", "response_format": "b64_json", "n": 1,
  "client_request_id": <idempotency key> }`
  - `size` = aspect ratio string (NOT OpenAI-style `"1024x1024"`).
  - `response_format: "b64_json"` → image inline as `data[].b64_json` (default = signed URL, 24h TTL).
  - Recovery after dropped connection: `GET /v1/images/generations/by-key/{client_request_id}`.
- Response: `{ "created", "model", "cost", "data": [{ "b64_json" | "url", "content_type" }] }`.
- Errors: standard JSON `{status,statusCode,message}`; 402 = insufficient balance.

## Model choice
User asked for "nano-banana" (Gemini image-gen codename). Chosen: **`nano-banana-2-1`**
(current workhorse, cheapest of the family, 4K-capable). The deprecated original
`gemini-2.5-flash-image` is sunsetted upstream (2026-10-02) and has no ppq flat alias.

## Call budget
MAX 8 image calls. **Actual: 3/8** (2 drafts @1K + 1 final @2K). All succeeded first try;
per-call `cost` returned 0.0 (promo). Details in `art/generation-log.md`.

## Local toolchain
- Pillow 12.1.0 ✅ (primary renderer/post-processor)
- ImageMagick `convert` ✅, inkscape ✅ (available but unused)
- DejaVu Serif Bold ✅ via fc-match (fallback glyph source)
- Nothing installed globally; nothing installed at all.
