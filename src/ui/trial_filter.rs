/// A parsed explorer filter: comma-separated alternatives (OR), each a space-separated list of terms (AND), where a
/// `!term` anywhere removes every trial it matches. Terms are case-insensitive substrings of the trial's subject
/// or trial name, or of any folder above it.
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
        self.matches_path(&[subject], trial)
    }

    /// Like `matches`, with every folder name from the explorer root down to the trial's parent.
    pub fn matches_path<S: AsRef<str>>(&self, folders: &[S], trial: &str) -> bool {
        let names: Vec<String> = folders.iter().map(|f| f.as_ref().to_lowercase()).collect();
        let trial = trial.to_lowercase();
        let has = |term: &String| trial.contains(term.as_str()) || names.iter().any(|n| n.contains(term.as_str()));
        if self.excludes.iter().any(has) {
            return false;
        }
        self.alternatives.is_empty() || self.alternatives.iter().any(|terms| terms.iter().all(has))
    }
}
