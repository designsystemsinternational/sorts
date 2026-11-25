//! The search query of the interactive list: text terms and facet terms (`key:value`).

use crate::facets::{self, ANALYZERS};
use crate::scan::FontFile;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// Lowercase text terms. All must be in the text of a node.
    pub terms: Vec<String>,
    /// Facet terms: (index in `ANALYZERS`, value). All must match a font file.
    pub facets: Vec<(usize, &'static str)>,
    /// Set when a facet term has an unknown or missing value. Then nothing matches.
    pub error: Option<String>,
}

impl Query {
    /// Splits the query at whitespace. A term `key:value` whose key is a facet key is a facet
    /// term (case-insensitive). All other terms, also terms with other `:`, are text terms.
    pub fn parse(text: &str) -> Self {
        let mut query = Self::default();
        for term in text.to_lowercase().split_whitespace() {
            let facet = term.split_once(':').and_then(|(key, value)| Some((facets::position(key)?, value)));
            let Some((index, value)) = facet else {
                query.terms.push(term.to_string());
                continue;
            };
            let analyzer = ANALYZERS[index];
            let values = analyzer.values().join(", ");
            match analyzer.values().iter().find(|v| **v == value) {
                Some(value) => query.facets.push((index, value)),
                None if value.is_empty() => {
                    query.error.get_or_insert(format!("Type a value for {}: {values}.", analyzer.key()));
                }
                None => {
                    query
                        .error
                        .get_or_insert(format!("Unknown value \"{value}\" for {}. Use {values}.", analyzer.key()));
                }
            }
        }
        query
    }

    /// True if the query filters nothing.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.facets.is_empty() && self.error.is_none()
    }

    /// True if all text terms are in the text (case-insensitive).
    pub fn matches_text(&self, text: &str) -> bool {
        let text = text.to_lowercase();
        self.terms.iter().all(|term| text.contains(term.as_str()))
    }

    /// True if the font has all facet values of the query. Never true with an error.
    pub fn matches_facets(&self, font: &FontFile) -> bool {
        self.error.is_none() && self.facets.iter().all(|(index, value)| font.facets.get(*index) == Some(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font(facets: &[&'static str]) -> FontFile {
        FontFile { id: "A/Font.otf".into(), faces: vec![], facets: facets.to_vec(), error: None }
    }

    #[test]
    fn splits_text_terms_and_facet_terms() {
        let query = Query::parse("  Myfont class:serif  Bold SPACING:Mono ");

        assert_eq!(query.terms, ["myfont", "bold"]);
        assert_eq!(query.facets, [(0, "serif"), (1, "mono")]);
        assert_eq!(query.error, None);
    }

    #[test]
    fn a_term_with_an_unknown_key_is_text() {
        let query = Query::parse("ratio:16 c:");

        assert_eq!(query.terms, ["ratio:16", "c:"]);
        assert!(query.facets.is_empty());
    }

    #[test]
    fn an_unknown_value_is_an_error() {
        let query = Query::parse("class:ser");

        assert_eq!(query.error.as_deref(), Some("Unknown value \"ser\" for class. Use serif, sans, other."));
        assert!(!query.matches_facets(&font(&["serif", "mono", "false"])));
    }

    #[test]
    fn a_missing_value_is_an_error_that_shows_the_values() {
        let query = Query::parse("myfont variable:");

        assert_eq!(query.error.as_deref(), Some("Type a value for variable: true, false."));
        assert!(!query.is_empty());
    }

    #[test]
    fn an_empty_query_is_empty() {
        assert!(Query::parse("  ").is_empty());
        assert!(!Query::parse("spacing:mono").is_empty());
    }

    #[test]
    fn matches_text_with_all_terms() {
        let query = Query::parse("sans bold class:sans");

        assert!(query.matches_text("Foundry/Sans-Bold.otf"));
        assert!(!query.matches_text("Foundry/Sans-Regular.otf"));
    }

    #[test]
    fn matches_facets_with_all_facet_terms() {
        let font = font(&["serif", "mono", "false"]);

        assert!(Query::parse("anything").matches_facets(&font));
        assert!(Query::parse("class:serif spacing:mono variable:false").matches_facets(&font));
        assert!(!Query::parse("class:serif variable:true").matches_facets(&font));
    }

    #[test]
    fn an_unreadable_font_matches_no_facet() {
        assert!(!Query::parse("class:other").matches_facets(&font(&[])));
    }
}
