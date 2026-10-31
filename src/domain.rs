use std::collections::HashMap;
use std::path::PathBuf;

/// Represents the raw data needed for a code review
pub struct ReviewData {
    pub summary: String, // e.g., "Comparing master...feature"
    pub changed_files: Vec<String>,
    pub diffs: HashMap<String, String>, // Map filename -> diff content
    pub context_files: Vec<String>,
    pub repo_root: PathBuf,
    /// Commits covered by the diff ("<short-hash> <subject>"), newest first.
    pub commits: Vec<String>,
    /// The commit the diff is taken against ("<short-hash> <subject>").
    pub base_commit: Option<String>,
    /// Number of files with uncommitted changes (staged, unstaged, or untracked).
    pub uncommitted_files: usize,
    /// Whether those uncommitted changes are part of the diff.
    pub uncommitted_included: bool,
}
