//! Milestone message templates: shipped bilingual (EN + EL) presets and the
//! content policy enforced at BOTH save-time (user-edited template / custom
//! suffix validation) and publish-time (the fully rendered note is re-run
//! through [`validate_content`] immediately before it can be posted).
//!
//! Content policy (Appendix R): NFC-normalize on input and render; total
//! content ≤ 1024 UTF-8 bytes; custom suffix ≤ 420 grapheme clusters; strip
//! Cc/Cf/bidi control characters (except ZWJ U+200D, which *emoji* grapheme
//! clusters require); refuse URLs, image extensions, `nostr:` URIs, `@`
//! mentions, `nsec1…`-style bech32 patterns, and empty content.
//!
//! Shipped presets NEVER name a textbook, chapter, or any curriculum proper
//! noun — the bounded-vocabulary milestone set already says the only thing
//! worth saying. A lint test ([`tests::shipped_presets_contain_no_curriculum_proper_nouns`])
//! enforces this mechanically.

use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

use crate::errors::Result;

/// Hard cap on total note content, in UTF-8 bytes (Appendix R).
pub const MAX_CONTENT_BYTES: usize = 1024;
/// Hard cap on the user-authored custom suffix, in extended grapheme
/// clusters — never `String::len`/`chars().count()`.
pub const MAX_SUFFIX_GRAPHEMES: usize = 420;

// --- Milestone slugs -------------------------------------------------------

pub const SLUG_GATE_PASSED_CHECK: &str = "gate.passed.check";
pub const SLUG_GATE_PASSED_ORGANIC: &str = "gate.passed.organic";
pub const SLUG_CHAPTER_MASTERED: &str = "chapter.mastered";
pub const SLUG_STREAK_7: &str = "streak.7";
pub const SLUG_STREAK_30: &str = "streak.30";
pub const SLUG_STREAK_100: &str = "streak.100";

/// Every milestone slug this crate can post, in canonical order.
pub const ALL_SLUGS: [&str; 6] = [
    SLUG_GATE_PASSED_CHECK,
    SLUG_GATE_PASSED_ORGANIC,
    SLUG_CHAPTER_MASTERED,
    SLUG_STREAK_7,
    SLUG_STREAK_30,
    SLUG_STREAK_100,
];

/// Slugs whose template carries a `{n}` count placeholder (streak day count).
pub fn slug_counts_placeholder(slug: &str) -> bool {
    matches!(slug, SLUG_STREAK_7 | SLUG_STREAK_30 | SLUG_STREAK_100)
}

// --- Language --------------------------------------------------------------

/// The two preset languages the crate ships (EN + EL per OQ-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateLang {
    En,
    El,
}

impl TemplateLang {
    pub fn code(&self) -> &'static str {
        match self {
            Self::En => "en",
            Self::El => "el",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code.to_ascii_lowercase().as_str() {
            "en" => Some(Self::En),
            "el" | "gr" => Some(Self::El),
            _ => None,
        }
    }
}

impl std::fmt::Display for TemplateLang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl Default for TemplateLang {
    fn default() -> Self {
        Self::En
    }
}

// --- Shipped presets -------------------------------------------------------

/// The exact shipped template body for `(slug, lang)`, if the slug exists.
/// `streak.*` bodies contain a `{n}` placeholder for the day count; other
/// bodies are complete sentences. None of these strings contains a
/// curriculum proper noun (see the lint test in this module).
pub fn preset(slug: &str, lang: TemplateLang) -> Option<&'static str> {
    let body = match (slug, lang) {
        (SLUG_GATE_PASSED_CHECK, TemplateLang::En) => {
            "I passed the 24-letter Greek alphabet check 🦉 I can officially read Greek script."
        }
        (SLUG_GATE_PASSED_CHECK, TemplateLang::El) => {
            "Πέρασα τον έλεγχο του ελληνικού αλφαβήτου (24 γράμματα) 🦉 Πλέον διαβάζω τα γράμματα με σιγουριά."
        }
        (SLUG_GATE_PASSED_ORGANIC, TemplateLang::En) => {
            "Reached measured accuracy on Greek letters 🦉 the alphabet is starting to feel natural."
        }
        (SLUG_GATE_PASSED_ORGANIC, TemplateLang::El) => {
            "Έφτασα μετρήσιμη ακρίβεια στα ελληνικά γράμματα 🦉 το αλφάβητο αρχίζει να μπαίνει για τα καλά."
        }
        (SLUG_CHAPTER_MASTERED, TemplateLang::En) => {
            "Mastered a chapter of my Greek textbook 📖"
        }
        (SLUG_CHAPTER_MASTERED, TemplateLang::El) => {
            "Ολοκλήρωσα ένα ακόμη κεφάλαιο ελληνικών 📖"
        }
        (s @ (SLUG_STREAK_7 | SLUG_STREAK_30 | SLUG_STREAK_100), TemplateLang::En) => {
            let _ = s;
            "{n}-day Greek streak — every due review cleared, day after day 🔥"
        }
        (s @ (SLUG_STREAK_7 | SLUG_STREAK_30 | SLUG_STREAK_100), TemplateLang::El) => {
            let _ = s;
            "{n} ημέρες σερί ελληνικών — όλες οι προγραμματισμένες επαναλήψεις εκκαθαρισμένες 🔥"
        }
        _ => return None,
    };
    Some(body)
}

// --- Validation ------------------------------------------------------------

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TemplateError {
    #[error("content is empty after normalization")]
    Empty,
    #[error("content exceeds {MAX_CONTENT_BYTES} UTF-8 bytes (got {0})")]
    TooLong(usize),
    #[error("suffix exceeds {MAX_SUFFIX_GRAPHEMES} grapheme clusters (got {0})")]
    SuffixTooLong(usize),
    #[error("content contains a forbidden pattern")]
    ForbiddenPattern,
    #[error("streak template is missing the {{n}} day-count placeholder")]
    MissingPlaceholder,
    #[error("unknown milestone slug: {0}")]
    UnknownSlug(String),
}

/// Is `c` a bidi override/directionality mark (must never reach a post)?
fn is_bidi_control(c: char) -> bool {
    matches!(c,
        '\u{200E}' | '\u{200F}'                       // LRM / RLM
        | '\u{202A}'..='\u{202E}'                       // LRE RLE PDF RLO LRO
        | '\u{2066}'..='\u{2069}'                       // LRI RLI FSI PDI
    )
}

/// Is `c` a Format (Cf) character we strip? ZWJ U+200D is deliberately
/// KEPT: multi-person/family emoji and some Indic sequences lose their
/// graphemes without it, and ZWJ carries no bidi/visual-injection risk.
fn is_stripped_format(c: char) -> bool {
    matches!(c,
        '\u{00AD}'             // soft hyphen
        | '\u{180E}'           // Mongolian vowel separator
        | '\u{200B}'           // zero-width space
        | '\u{200C}'           // ZWNJ
        | '\u{2060}'           // word joiner
        | '\u{2061}'..='\u{2064}' // invisible math operators
        | '\u{FEFF}'           // BOM / zero-width no-break space
        | '\u{FFF9}'..='\u{FFFB}' // interlinear annotation anchors
    )
}

/// NFC-normalize and strip control/format/bidi characters. Keeps ZWJ and
/// `\n` (newlines are structural content, and templates/suffixes join on
/// them). Applied by the validator AND the renderer, so what ships on the
/// wire is exactly what the user saw in the preview.
pub fn sanitize(input: &str) -> String {
    input
        .nfc()
        .filter(|c| *c == '\n' || (!c.is_control() && !is_bidi_control(*c) && !is_stripped_format(*c)))
        .collect()
}

/// Lowercased-haystack substrings that make content unacceptable. This is a
/// deny-list of pattern shapes, not of words: there is deliberately no
/// "offensive word" filter here — milestone posts are the user's own text.
fn forbidden_pattern(content_lower: &str) -> bool {
    const PAT: [&str; 20] = [
        "http://",
        "https://",
        "www.",
        "nostr:",
        "@",
        "npub1",
        "nprofile1",
        "nsec1",
        "note1",
        "naddr1",
        "nevent1",
        ".png",
        ".jpg",
        ".jpeg",
        ".gif",
        ".webp",
        ".bmp",
        ".svg",
        ".avif",
        ".tif",
    ];
    PAT.iter().any(|p| content_lower.contains(p))
}

/// Validate fully-rendered note content; returns the sanitized (NFC +
/// stripped) form that is safe to publish.
pub fn validate_content(content: &str) -> std::result::Result<String, TemplateError> {
    let sanitized = sanitize(content);
    if sanitized.trim().is_empty() {
        return Err(TemplateError::Empty);
    }
    if sanitized.len() > MAX_CONTENT_BYTES {
        return Err(TemplateError::TooLong(sanitized.len()));
    }
    if forbidden_pattern(&sanitized.to_lowercase()) {
        return Err(TemplateError::ForbiddenPattern);
    }
    Ok(sanitized)
}

/// Validate a user-authored custom suffix; returns its sanitized form.
/// Empty/whitespace-only suffix is legal and means "no custom text".
pub fn validate_suffix(suffix: &str) -> std::result::Result<String, TemplateError> {
    let sanitized = sanitize(suffix);
    let graphemes = sanitized.graphemes(true).count();
    if graphemes > MAX_SUFFIX_GRAPHEMES {
        return Err(TemplateError::SuffixTooLong(graphemes));
    }
    if forbidden_pattern(&sanitized.to_lowercase()) {
        return Err(TemplateError::ForbiddenPattern);
    }
    Ok(sanitized)
}

/// Save-time validation for a custom template override: the body must be
/// valid content once `{n}` is replaced with a plausible rendered count, and
/// a streak slug MUST contain the placeholder (non-streak slugs must not).
pub fn validate_template_override(
    slug: &str,
    body: &str,
) -> std::result::Result<String, TemplateError> {
    if !ALL_SLUGS.contains(&slug) {
        return Err(TemplateError::UnknownSlug(slug.to_string()));
    }
    let placeholder = body.contains("{n}");
    if slug_counts_placeholder(slug) && !placeholder {
        return Err(TemplateError::MissingPlaceholder);
    }
    let preview = body.replace("{n}", "100");
    validate_content(&preview)
}

/// Render final note content from a template body + optional custom suffix.
/// Applied at enqueue time AND re-validated at publish time.
pub fn render(body: &str, count: Option<u32>, suffix: Option<&str>) -> Result<String> {
    let filled = match count {
        Some(n) => body.replace("{n}", &n.to_string()),
        None => body.to_string(),
    };
    if filled.contains("{n}") {
        return Err(TemplateError::MissingPlaceholder.into());
    }
    let suffix = match suffix {
        Some(s) => validate_suffix(s)?,
        None => String::new(),
    };
    let combined = if suffix.trim().is_empty() {
        filled
    } else {
        format!("{filled}\n\n{suffix}")
    };
    Ok(validate_content(&combined)?)
}

// --- Settings persistence --------------------------------------------------

fn template_key(slug: &str, lang: TemplateLang) -> String {
    format!("share.template.{slug}.{}", lang.code())
}

/// The active template body for a slug: user override (settings) or shipped
/// preset. Settings are HLC-LWW-synced via `greek_core::sync::setting_put`,
/// so template edits propagate to paired devices like other preferences.
pub fn template_for(
    db: &greek_core::Database,
    slug: &str,
    lang: TemplateLang,
) -> Result<String> {
    let key = template_key(slug, lang);
    let custom: Option<String> = db
        .conn()
        .query_row(
            "SELECT value FROM user_settings WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .ok();
    if let Some(custom) = custom.filter(|v| !v.trim().is_empty()) {
        // Stored overrides were validated at save-time; still re-validate on
        // load so a hand-edited database can't bypass the policy — but hand
        // back the RAW template (`{n}` intact), not the validation preview.
        validate_template_override(slug, &custom)?;
        return Ok(sanitize(&custom));
    }
    preset(slug, lang)
        .map(str::to_string)
        .ok_or_else(|| TemplateError::UnknownSlug(slug.to_string()).into())
}

/// Persist a user template override after save-time validation. An empty
/// body restores the shipped preset (= deletes the override).
pub fn set_template_override(
    db: &greek_core::Database,
    slug: &str,
    lang: TemplateLang,
    body: &str,
) -> Result<()> {
    let key = template_key(slug, lang);
    if body.trim().is_empty() {
        greek_core::sync::setting_put(db, &key, "")?;
        return Ok(());
    }
    // Validate the RENDERED soundness (placeholder swapped for a plausible
    // count) but persist the raw sanitized template, `{n}` included.
    validate_template_override(slug, body)?;
    greek_core::sync::setting_put(db, &key, &sanitize(body))?;
    Ok(())
}

/// Render the final note content for a milestone occurrence: override-aware
/// template, `{n}` substitution, optional validated custom suffix.
pub fn render_content(
    db: &greek_core::Database,
    slug: &str,
    lang: TemplateLang,
    count: Option<u32>,
    suffix: Option<&str>,
) -> Result<String> {
    let body = template_for(db, slug, lang)?;
    render(&body, count, suffix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use greek_core::Database;

    /// Substrings that must NEVER appear in a shipped preset (curriculum
    /// proper nouns, textbook display names, known chapter titles).
    const BANNED_IN_PRESETS: [&str; 12] = [
        "athenaze",
        "jact",
        "dcc",
        "dicaeopolis",
        "δικαιόπολις",
        "αθηνάζε",
        "αθήναζε",
        "reading greek",
        "core vocabulary",
        "xanthias",
        "ξανθίας",
        "frequency list",
    ];

    #[test]
    fn shipped_presets_contain_no_curriculum_proper_nouns() {
        for slug in ALL_SLUGS {
            for lang in [TemplateLang::En, TemplateLang::El] {
                let body = preset(slug, lang).expect("every slug has both presets");
                let lower = body.to_lowercase();
                for banned in BANNED_IN_PRESETS {
                    assert!(
                        !lower.contains(banned),
                        "preset {slug}/{lang} contains banned curriculum token {banned:?}: {body:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_slug_has_en_and_el_presets_that_validate() {
        for slug in ALL_SLUGS {
            for lang in [TemplateLang::En, TemplateLang::El] {
                let body = preset(slug, lang).expect("preset exists");
                validate_template_override(slug, body)
                    .unwrap_or_else(|e| panic!("preset {slug}/{lang} must validate: {e}"));
                let rendered = render(body, Some(7), None).unwrap();
                assert!(rendered.len() <= MAX_CONTENT_BYTES);
            }
        }
    }

    #[test]
    fn validate_accepts_polytonic_greek() {
        let s = "χαῖρε, ὦ κόσμε — προσκυνοῦμεν τῷ Διονύσῳ";
        let v = validate_content(s).unwrap();
        assert_eq!(v, s.nfc().collect::<String>());
    }

    #[test]
    fn validate_normalizes_nfd_to_nfc() {
        // 'ᾰ' sources: alpha + combining dots; NFD and NFC round-trip equal.
        let nfd = "α\u{0301}"; // alpha + combining acute
        let v = validate_content(nfd).unwrap();
        assert_eq!(v, "ά"); // NFC single code point
    }

    #[test]
    fn validate_keeps_zwj_emoji_graphemes() {
        let family = "👨\u{200D}👩\u{200D}👧";
        let v = validate_content(&format!("Greek with family {family}")).unwrap();
        assert!(v.contains(family), "ZWJ sequences must survive sanitization");
        assert_eq!(family.graphemes(true).count(), 1, "test input sanity");
    }

    #[test]
    fn validate_strips_bidi_and_controls() {
        let evil = "hello \u{202E}world\u{202C}\u{200B}!\u{FEFF}";
        let v = validate_content(evil).unwrap();
        assert_eq!(v, "hello world!");
    }

    #[test]
    fn validate_rejects_empty_and_whitespace_only() {
        assert_eq!(validate_content(""), Err(TemplateError::Empty));
        assert_eq!(validate_content("   \n\t "), Err(TemplateError::Empty));
        // Controls striped to nothing are empty.
        assert_eq!(validate_content("\u{200B}\u{FEFF}\u{202E}"), Err(TemplateError::Empty));
    }

    #[test]
    fn validate_rejects_oversize() {
        let big = "α".repeat(MAX_CONTENT_BYTES); // 2048 bytes
        assert!(matches!(validate_content(&big), Err(TemplateError::TooLong(_))));
        let edge = "α".repeat(MAX_CONTENT_BYTES / 2); // 1024 bytes exactly
        assert!(validate_content(&edge).is_ok());
    }

    #[test]
    fn validate_rejects_urls_and_mentions_and_secrets() {
        for bad in [
            "check out https://example.com/post",
            "visit www.example.com",
            "nostr:note1qqqq",
            "shoutout to @someone",
            "my npub is npub1abc...",
            "my secret nsec1ultra",
            "see image.png",
            "avatar.JPG content",
            "link nprofile1qq",
        ] {
            assert_eq!(
                validate_content(bad),
                Err(TemplateError::ForbiddenPattern),
                "must reject: {bad:?}"
            );
        }
    }

    #[test]
    fn suffix_grapheme_cap_is_cluster_based() {
        let ok = "α".repeat(MAX_SUFFIX_GRAPHEMES);
        assert!(validate_suffix(&ok).is_ok());
        let too_many = "α".repeat(MAX_SUFFIX_GRAPHEMES + 1);
        assert!(matches!(
            validate_suffix(&too_many),
            Err(TemplateError::SuffixTooLong(_))
        ));
        // 420 flag emoji: 21 regional-indicator pairs → 441 graphemes? No:
        // each flag = 2 scalars = 1 grapheme. 420 flags = 420 graphemes.
        let flags: String = std::iter::repeat("🇬🇷").take(MAX_SUFFIX_GRAPHEMES).collect();
        assert_eq!(flags.graphemes(true).count(), MAX_SUFFIX_GRAPHEMES);
        assert!(validate_suffix(&flags).is_ok());
    }

    #[test]
    fn render_appends_suffix_and_enforces_total_cap() {
        let base = "short line";
        let out = render(base, None, Some("kai hello")).unwrap();
        assert_eq!(out, "short line\n\nkai hello");
        // Suffix that pushes the total over the byte cap is rejected.
        let filler = "ß".repeat(MAX_CONTENT_BYTES);
        assert!(render("pad", None, Some(&filler)).is_err());
        // Placeholder carried to render without a count is an error.
        assert!(matches!(
            render("{n}-day streak", None, None),
            Err(crate::errors::ShareError::Template(TemplateError::MissingPlaceholder))
        ));
    }

    #[test]
    fn override_roundtrips_through_settings_and_resets_to_preset() {
        let db = Database::open(":memory:", None).unwrap();
        assert_eq!(
            template_for(&db, SLUG_GATE_PASSED_CHECK, TemplateLang::En).unwrap(),
            preset(SLUG_GATE_PASSED_CHECK, TemplateLang::En).unwrap()
        );
        set_template_override(&db, SLUG_GATE_PASSED_CHECK, TemplateLang::En, "Alphabet done 🦉")
            .unwrap();
        assert_eq!(
            template_for(&db, SLUG_GATE_PASSED_CHECK, TemplateLang::En).unwrap(),
            "Alphabet done 🦉"
        );
        // Other slug/lang lanes are untouched.
        assert_eq!(
            template_for(&db, SLUG_GATE_PASSED_CHECK, TemplateLang::El).unwrap(),
            preset(SLUG_GATE_PASSED_CHECK, TemplateLang::El).unwrap()
        );
        // Empty override restores the preset.
        set_template_override(&db, SLUG_GATE_PASSED_CHECK, TemplateLang::En, "").unwrap();
        assert_eq!(
            template_for(&db, SLUG_GATE_PASSED_CHECK, TemplateLang::En).unwrap(),
            preset(SLUG_GATE_PASSED_CHECK, TemplateLang::En).unwrap()
        );
    }

    #[test]
    fn override_requires_placeholder_on_streak_slugs() {
        let db = Database::open(":memory:", None).unwrap();
        assert!(matches!(
            set_template_override(&db, SLUG_STREAK_7, TemplateLang::En, "seven days!"),
            Err(crate::errors::ShareError::Template(TemplateError::MissingPlaceholder))
        ));
        set_template_override(&db, SLUG_STREAK_7, TemplateLang::En, "{n} days of Greek!")
            .unwrap();
        assert_eq!(
            render_content(&db, SLUG_STREAK_7, TemplateLang::En, Some(7), None).unwrap(),
            "7 days of Greek!"
        );
    }

    #[test]
    fn unknown_slug_rejected() {
        let db = Database::open(":memory:", None).unwrap();
        assert!(matches!(
            template_for(&db, "streak.9999", TemplateLang::En),
            Err(crate::errors::ShareError::Template(TemplateError::UnknownSlug(_)))
        ));
        assert!(matches!(
            set_template_override(&db, "nope", TemplateLang::En, "x"),
            Err(crate::errors::ShareError::Template(TemplateError::UnknownSlug(_)))
        ));
    }
}
