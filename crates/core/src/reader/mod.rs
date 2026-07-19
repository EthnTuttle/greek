use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextSection {
    pub id: String,
    pub title: String,
    pub author: String,
    pub text: String,
    pub tokens: Vec<Token>,
    pub difficulty: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub surface: String,
    pub whitespace_after: bool,
}

pub struct Reader;

impl Reader {
    /// Get available text sections (hardcoded samples for MVP)
    pub fn get_sections() -> Vec<TextSection> {
        let texts = vec![
            (
                "aesop-north-wind-and-sun",
                "The North Wind and the Sun (Βορέας καὶ Ἥλιος)",
                "Aesop (public domain)",
                "Βορέας καὶ Ἥλιος περὶ δυνάμεως ἤριζον· ἔδοξε δὲ αὐτοῖς ἐκείνῳ τὴν νίκην δοῦναι, ὃς ἂν αὐτῶν ὁδοιπόρον ἐκδύσῃ. καὶ ὁ Βορέας ἀρξάμενος σφοδρὸς ἦν· τοῦ δὲ ἀνθρώπου ἀντεχομένου τῆς ἐσθῆτος, μᾶλλον ἐπέκειτο. ὁ δὲ ὑπὸ τοῦ ψύχους καταπονούμενος μᾶλλον καὶ ἑτέραν ἐσθῆτα προσελάμβανεν, ἕως ἀποκαμὼν ὁ Βορέας τῷ Ἡλίῳ παρέδωκεν. ὁ δὲ τὸ μὲν πρῶτον μετρίως προσέλαμψε· τοῦ δὲ ἀνθρώπου τὰ ἱμάτια ἀποτιθεμένου, σφοδρότερον τὴν θέρμην ἐπέτεινεν, ἕως ὑπὸ τῆς ἀλέας μηκέτι ἀντέχων ἀποδυσάμενος ἀπῄει.",
                0.3, // Easy - classic beginner fable
            ),
            (
                "xenophon-anabasis-1-1-1",
                "Anabasis 1.1.1",
                "Xenophon",
                "Δαρείου καὶ Παρυσάτιδος γίγνονται παῖδες δύο, πρεσβύτερος μὲν Ἀρταξέρξης, νεώτερος δὲ Κῦρος· ἐπεὶ δὲ ἠσθένει Δαρεῖος καὶ ὑπώπτευε τελευτὴν τοῦ βίου, ἐβούλετο τὼ παῖδε ἀμφοτέρω παρεῖναι.",
                0.7, // Intermediate
            ),
            (
                "lysias-1-opening",
                "On the Murder of Eratosthenes (Opening)",
                "Lysias",
                "Περὶ πολλοῦ ἂν ποιησαίμην, ὦ ἄνδρες, τὸ τοιούτους ὑμᾶς ἐμοὶ δικαστὰς περὶ τούτου τοῦ πράγματος γενέσθαι, οἷοίπερ ἂν ὑμῖν αὐτοῖς εἴητε τοιαῦτα πεπονθότες.",
                0.8, // Advanced
            ),
        ];

        texts
            .into_iter()
            .map(|(id, title, author, text, difficulty)| {
                let tokens = Self::tokenize(text);
                TextSection {
                    id: id.to_string(),
                    title: title.to_string(),
                    author: author.to_string(),
                    text: text.to_string(),
                    tokens,
                    difficulty,
                }
            })
            .collect()
    }

    /// Simple tokenizer: split on whitespace and punctuation.
    /// Greek punctuation includes: . , · ; (Greek question mark) and regular punctuation.
    pub fn tokenize(text: &str) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut current_word = String::new();
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();

        let mut i = 0;
        while i < len {
            let ch = chars[i];

            if ch.is_whitespace() {
                // End current word if any
                if !current_word.is_empty() {
                    tokens.push(Token {
                        surface: current_word.clone(),
                        whitespace_after: true,
                    });
                    current_word.clear();
                }
                i += 1;
            } else if is_punctuation(ch) {
                // End current word if any (without whitespace after)
                if !current_word.is_empty() {
                    tokens.push(Token {
                        surface: current_word.clone(),
                        whitespace_after: false,
                    });
                    current_word.clear();
                }
                // Punctuation is its own token
                let has_whitespace = i + 1 < len && chars[i + 1].is_whitespace();
                tokens.push(Token {
                    surface: ch.to_string(),
                    whitespace_after: has_whitespace,
                });
                i += 1;
            } else {
                current_word.push(ch);
                i += 1;
            }
        }

        // Don't forget the last word
        if !current_word.is_empty() {
            tokens.push(Token {
                surface: current_word,
                whitespace_after: false,
            });
        }

        tokens
    }
}

/// Check if a character is punctuation (including Greek-specific punctuation)
fn is_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '.' | ','
            | ';'
            | ':'
            | '·'      // Greek ano teleia (mid-dot, acts as semicolon/colon)
            | '\u{037E}' // Greek question mark (visually identical to ;)
            | '!'
            | '?'
            | '('
            | ')'
            | '['
            | ']'
            | '—'
            | '-'
            | '"'
            | '\u{201C}' // left double quotation mark (“)
            | '\u{201D}' // right double quotation mark (”)
            | '\u{00AB}' // left-pointing double angle quotation mark («)
            | '\u{00BB}' // right-pointing double angle quotation mark (»)
            | '\''
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_sections() {
        let sections = Reader::get_sections();
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].id, "aesop-north-wind-and-sun");
        assert_eq!(sections[1].id, "xenophon-anabasis-1-1-1");
        assert!(!sections[0].tokens.is_empty());
    }

    #[test]
    fn test_tokenize_simple() {
        let tokens = Reader::tokenize("ὁ Δικαιόπολις");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].surface, "ὁ");
        assert!(tokens[0].whitespace_after);
        assert_eq!(tokens[1].surface, "Δικαιόπολις");
        assert!(!tokens[1].whitespace_after);
    }

    #[test]
    fn test_tokenize_with_punctuation() {
        let tokens = Reader::tokenize("ἐστιν. ὁ");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].surface, "ἐστιν");
        assert!(!tokens[0].whitespace_after); // No space between word and punctuation
        assert_eq!(tokens[1].surface, ".");
        assert!(tokens[1].whitespace_after);
        assert_eq!(tokens[2].surface, "ὁ");
    }

    #[test]
    fn test_tokenize_greek_question() {
        // Greek uses ; as question mark
        let tokens = Reader::tokenize("τίς ἐστιν;");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].surface, "τίς");
        assert_eq!(tokens[1].surface, "ἐστιν");
        assert_eq!(tokens[2].surface, ";");
    }

    #[test]
    fn test_tokenize_comma() {
        let tokens = Reader::tokenize("μικρός, καλός");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].surface, "μικρός");
        assert!(!tokens[0].whitespace_after);
        assert_eq!(tokens[1].surface, ",");
        assert!(tokens[1].whitespace_after);
        assert_eq!(tokens[2].surface, "καλός");
    }

    #[test]
    fn test_tokenize_empty() {
        let tokens = Reader::tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_reconstruct_text() {
        let original = "ὁ Δικαιόπολις Ἀθηναῖός ἐστιν.";
        let tokens = Reader::tokenize(original);
        let reconstructed: String = tokens
            .iter()
            .map(|t| {
                if t.whitespace_after {
                    format!("{} ", t.surface)
                } else {
                    t.surface.clone()
                }
            })
            .collect::<String>()
            .trim_end()
            .to_string();
        assert_eq!(reconstructed, original);
    }
}
