use std::collections::HashMap;
use std::path::PathBuf;

/// Represents the raw data needed for a code review
pub struct ReviewData {
    pub summary: String, // e.g., "Comparing master...feature"
    pub changed_files: Vec<String>,
    pub diffs: HashMap<String, String>, // Map filename -> diff content
    pub context_files: Vec<String>,
    pub repo_root: PathBuf,
}
