/// A parsed explorer filter: comma-separated alternatives (OR), each a space-separated list of terms (AND), where a
/// `!term` anywhere removes every trial it matches. Terms are case-insensitive substrings of the trial's subject
/// or trial name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrialFilter {
    /// Alternatives that each need all of their terms; empty means everything matches before exclusions.
    alternatives: Vec<Vec<String>>,
    excludes: Vec<String>,
}

impl TrialFilter {
    pub fn parse(query: &str) -> Self {
        let mut filter = Self::default();
        for group in query.split(',') {
            let mut required = Vec::new();
            for word in group.split_whitespace() {
                match word.strip_prefix('!') {
                    Some("") => {}
                    Some(excluded) => filter.excludes.push(excluded.to_lowercase()),
                    None => required.push(word.to_lowercase()),
                }
            }
            if !required.is_empty() {
                filter.alternatives.push(required);
            }
        }
        filter
    }

    /// Whether the query filters anything.
    pub fn is_active(&self) -> bool {
        !self.alternatives.is_empty() || !self.excludes.is_empty()
    }

    pub fn matches(&self, subject: &str, trial: &str) -> bool {
        let subject = subject.to_lowercase();
        let trial = trial.to_lowercase();
        let has = |term: &String| subject.contains(term.as_str()) || trial.contains(term.as_str());
        if self.excludes.iter().any(has) {
            return false;
        }
        self.alternatives.is_empty() || self.alternatives.iter().any(|terms| terms.iter().all(has))
    }
}
