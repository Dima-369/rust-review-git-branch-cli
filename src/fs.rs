use anyhow::{Context, Result, bail};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use path_clean::PathClean;
use regex::RegexSet;
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Execute a command and optionally print it for debugging
pub fn run_command(cmd: &mut Command) -> Result<Output> {
    log::debug!("[exec] {cmd:?}");
    cmd.output()
        .with_context(|| format!("Failed to execute: {cmd:?}"))
}

/// Result of reading a local file
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileContent {
    /// File was read successfully
    Content(String),
    /// File was deleted (not found)
    Deleted,
    /// File is binary or contains invalid UTF-8
    Binary,
}

impl FileContent {
    /// Convert to string for display in prompts
    pub fn to_display_string(&self) -> String {
        match self {
            FileContent::Content(s) => s.clone(),
            FileContent::Deleted => "(File deleted)".to_string(),
            FileContent::Binary => "(Binary file or invalid UTF-8 content)".to_string(),
        }
    }
}

/// Read local file content, handling deleted files and binary files gracefully
pub fn get_local_file_content(file_path: impl AsRef<Path>) -> Result<FileContent> {
    let path = file_path.as_ref();
    match fs::read_to_string(path) {
        Ok(content) => Ok(FileContent::Content(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(FileContent::Deleted),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Ok(FileContent::Binary),
        Err(e) => bail!("Failed to read file '{}': {e}", path.display()),
    }
}

/// Get the repository root directory using git rev-parse
pub fn get_repo_root(force_cwd: bool) -> Result<PathBuf> {
    if force_cwd {
        return std::env::current_dir().context("Failed to get current working directory");
    }

    let mut cmd = Command::new("git");
    cmd.args(["rev-parse", "--show-toplevel"]);
    let output = run_command(&mut cmd)?;

    if !output.status.success() {
        // Fallback to jj root if git fails
        let mut jj_cmd = Command::new("jj");
        jj_cmd.args(["root"]);
        let jj_output = run_command(&mut jj_cmd)?;

        if jj_output.status.success() {
            return Ok(PathBuf::from(
                String::from_utf8(jj_output.stdout)?.trim().to_string(),
            ));
        }

        bail!("Failed to determine repository root. Are you in a git/jj repository?");
    }

    Ok(PathBuf::from(
        String::from_utf8(output.stdout)?.trim().to_string(),
    ))
}

/// Normalize a path without resolving symlinks (removes . and .. components)
fn normalize_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.clean()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path).clean())
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

/// Expand tilde (~) in paths to home directory
fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        let home_dir = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        format!("{}{}", home_dir, &path[1..])
    } else {
        path.to_string()
    }
}

/// Validates and expands context file paths
pub fn validate_and_expand_context_files(
    context_files: &[String],
    changed_files: &[String],
    repo_root: &Path,
) -> Result<Vec<String>> {
    let mut validated_files = Vec::new();
    let mut seen_files = HashSet::new();
    let cwd = std::env::current_dir()?;
    let normalized_repo_root = normalize_path(repo_root);

    let changed_files_normalized: HashSet<PathBuf> = changed_files
        .iter()
        .map(|changed_file| normalize_path(&normalized_repo_root.join(changed_file)))
        .collect();

    for file_path in context_files {
        let expanded_path = expand_tilde(file_path);

        let abs_path = if Path::new(&expanded_path).is_absolute() {
            PathBuf::from(&expanded_path).clean()
        } else {
            cwd.join(&expanded_path).clean()
        };

        if !abs_path.exists() {
            bail!("Context file does not exist: {}", abs_path.display());
        }

        let normalized_abs_path = normalize_path(&abs_path);

        if changed_files_normalized.contains(&normalized_abs_path) {
            continue;
        }

        let abs_path_str = abs_path.to_string_lossy().to_string();
        if seen_files.insert(abs_path_str.clone()) {
            validated_files.push(abs_path_str);
        }
    }

    Ok(validated_files)
}

/// Format a file path for display:
/// - If path is within the repo root, show as relative path
/// - Otherwise, abbreviate home directory with ~
pub fn format_path_for_display(path: &str, repo_root: &Path) -> String {
    let path_buf = PathBuf::from(path);

    // Try to strip repo root prefix without syscalls (paths should already be absolute)
    if let Ok(relative) = path_buf.strip_prefix(repo_root) {
        return relative.to_string_lossy().to_string();
    }

    // Abbreviate home directory with ~
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok();
    if let Some(home) = home {
        let home_path = Path::new(&home);
        if let Ok(relative) = path_buf.strip_prefix(home_path) {
            return format!("~/{}", relative.to_string_lossy());
        }
    }

    path.to_string()
}

/// Result of checking if a file is readable text
enum FileReadability {
    /// File is readable text
    Text,
    /// File is binary (contains null bytes)
    Binary,
    /// File cannot be opened (permissions, broken symlink, etc.)
    Unreadable(std::io::Error),
}

/// Check if a file is binary by reading the first 1KB and looking for null bytes
fn check_file_readability(path: &Path) -> FileReadability {
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return FileReadability::Unreadable(e),
    };
    let mut buffer = [0u8; 1024];
    match file.read(&mut buffer) {
        Ok(n) if buffer[..n].contains(&0) => FileReadability::Binary,
        Ok(_) => FileReadability::Text,
        Err(e) => FileReadability::Unreadable(e),
    }
}

/// Check if a file should be included based on regex match and readability
fn should_include_file(file_path: &str, abs_path: &Path, regex_set: &RegexSet) -> bool {
    if !regex_set.is_match(file_path) {
        return false;
    }

    match check_file_readability(abs_path) {
        FileReadability::Text => true,
        FileReadability::Binary => false,
        FileReadability::Unreadable(e) => {
            log::warn!("Cannot read '{}': {}", abs_path.display(), e);
            false
        }
    }
}

/// Find files matching regex patterns using ripgrep
/// Returns absolute paths to matching non-binary files, limited to `max_files`
pub fn find_files_by_regex(
    patterns: &[String],
    max_files: usize,
    repo_root: &Path,
) -> Result<Vec<String>> {
    if patterns.is_empty() {
        return Ok(Vec::new());
    }

    let mut cmd = Command::new("rg");
    cmd.args(["--files"])
        .current_dir(repo_root)
        .stdout(Stdio::piped());

    log::debug!("[spawn] {cmd:?}");

    let mut child = cmd
        .spawn()
        .context("Failed to execute 'rg --files'. Is ripgrep installed?")?;

    let stdout = child.stdout.take().context("Failed to capture rg stdout")?;
    let regex_set = RegexSet::new(patterns).context("Invalid regex pattern")?;

    let mut matched_files = Vec::new();
    let mut limit_reached = false;
    let reader = BufReader::new(stdout);

    for line in reader.lines() {
        if matched_files.len() >= max_files {
            limit_reached = true;
            break;
        }

        let file_path = match line {
            Ok(l) => l,
            Err(_) => continue, // Skip non-UTF8 lines
        };
        let file_path = file_path.trim();
        if file_path.is_empty() {
            continue;
        }

        let abs_path = if Path::new(file_path).is_absolute() {
            PathBuf::from(file_path)
        } else {
            repo_root.join(file_path)
        };

        if should_include_file(file_path, &abs_path, &regex_set) {
            matched_files.push(abs_path.to_string_lossy().to_string());
        }
    }

    // Kill child process if we broke out early
    if limit_reached {
        let _ = child.kill();
        log::warn!(
            "Regex matched more than {max_files} files, truncating. Use --max-regex-files to adjust."
        );
    }

    let _ = child.wait();

    Ok(matched_files)
}

/// Recursively collect all text files from a directory, excluding files already in changed_files
/// and respecting ignore patterns. Uses canonical path tracking to prevent symlink loops.
fn collect_files_from_directory(
    dir_path: &Path,
    changed_files: &HashSet<PathBuf>,
    matcher: &IgnoreMatcher,
    visited: &mut HashSet<PathBuf>,
    max_files: usize,
    files_collected: &mut usize,
    truncated: &mut bool,
) -> Result<Vec<String>> {
    let mut collected_files = Vec::new();

    // Canonicalize to detect symlink loops
    let canonical_path = match dir_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Cannot canonicalize '{}': {}", dir_path.display(), e);
            return Ok(collected_files);
        }
    };

    // Skip if already visited (symlink loop detection)
    if !visited.insert(canonical_path) {
        return Ok(collected_files);
    }

    // Collect and sort entries for deterministic ordering
    // Note: read from dir_path (not canonical_path) to avoid Windows UNC prefix issues
    let mut entries: Vec<_> = fs::read_dir(dir_path)
        .with_context(|| format!("Failed to read directory: {}", dir_path.display()))?
        .filter_map(|e| {
            e.map_err(|err| {
                log::warn!("Skipping entry in '{}': {}", dir_path.display(), err);
            })
            .ok()
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        // Check if we've reached the file limit
        if *files_collected >= max_files {
            *truncated = true;
            return Ok(collected_files);
        }

        let entry_path = entry.path();

        if entry_path.is_dir() {
            // Check if directory should be ignored using full path matching
            // Normalize to forward slashes for cross-platform pattern matching
            let lossy = entry_path.to_string_lossy();
            let path_str = lossy.replace('\\', "/");
            if matcher.is_ignored(&path_str) {
                // Silently skip ignored directories (common case: node_modules, target, etc.)
                continue;
            }

            // Recursively collect files from subdirectories
            collected_files.extend(collect_files_from_directory(
                &entry_path,
                changed_files,
                matcher,
                visited,
                max_files,
                files_collected,
                truncated,
            )?);
        } else if entry_path.is_file() {
            // Check if file should be ignored using full path matching
            // Normalize to forward slashes for cross-platform pattern matching
            let lossy = entry_path.to_string_lossy();
            let path_str = lossy.replace('\\', "/");
            if matcher.is_ignored(&path_str) {
                // Silently skip ignored files
                continue;
            }

            // Check if file is readable text
            match check_file_readability(&entry_path) {
                FileReadability::Text => {
                    // Normalize path for comparison (canonicalize vs normalize_path mismatch)
                    let normalized_entry = normalize_path(&entry_path);

                    // Skip if this file is already in changed_files (will be in diff)
                    if changed_files.contains(&normalized_entry) {
                        continue;
                    }

                    collected_files.push(lossy.into_owned());
                    *files_collected += 1;
                }
                FileReadability::Binary => {
                    // Skip binary files silently
                }
                FileReadability::Unreadable(e) => {
                    log::warn!("Cannot read '{}': {}", entry_path.display(), e);
                }
            }
        }
    }

    Ok(collected_files)
}

/// Options for collecting files from directories
#[derive(Debug)]
pub struct DirCollectOptions<'a> {
    pub matcher: &'a IgnoreMatcher,
    pub max_dir_files: usize,
    pub changed_files: &'a [String],
    pub repo_root: &'a Path,
}

/// Resolve all context files: combine explicit paths, directory paths, and regex-matched files,
/// then validate and expand them
pub fn resolve_all_context_files(
    explicit_files: &[String],
    append_dirs: &[String],
    regex_patterns: &[String],
    max_regex_files: usize,
    opts: DirCollectOptions<'_>,
) -> Result<Vec<String>> {
    let regex_matched_files = find_files_by_regex(regex_patterns, max_regex_files, opts.repo_root)?;

    // Build set of changed files as normalized absolute paths for deduplication
    let normalized_repo_root = normalize_path(opts.repo_root);
    let changed_files_normalized: HashSet<PathBuf> = opts
        .changed_files
        .iter()
        .map(|changed_file| normalize_path(&normalized_repo_root.join(changed_file)))
        .collect();

    // Collect files from append directories (already validated, no need to re-validate)
    let mut context_files = Vec::new();
    let cwd = std::env::current_dir()?;

    for dir_path_str in append_dirs {
        // Expand tilde first
        let expanded_path = expand_tilde(dir_path_str);

        let abs_dir_path = if Path::new(&expanded_path).is_absolute() {
            PathBuf::from(&expanded_path).clean()
        } else {
            cwd.join(&expanded_path).clean()
        };

        if !abs_dir_path.exists() {
            bail!("Directory does not exist: {}", abs_dir_path.display());
        }

        if !abs_dir_path.is_dir() {
            bail!("Path is not a directory: {}", abs_dir_path.display());
        }

        // Track visited canonical paths to prevent symlink loops
        let mut visited: HashSet<PathBuf> = HashSet::new();
        let mut files_collected: usize = 0;
        let mut truncated = false;
        let collected = collect_files_from_directory(
            &abs_dir_path,
            &changed_files_normalized,
            opts.matcher,
            &mut visited,
            opts.max_dir_files,
            &mut files_collected,
            &mut truncated,
        )?;

        // Warn if limit was reached
        if truncated {
            log::warn!(
                "Directory '{}' exceeded file limit ({}), truncating to {} files. Use --max-dir-files to adjust.",
                abs_dir_path.display(),
                opts.max_dir_files,
                files_collected
            );
        }

        context_files.extend(collected);
    }

    // Add regex-matched files (already validated by find_files_by_regex)
    context_files.extend(regex_matched_files);

    // Validate explicit context files separately (they need existence/dedup checks)
    let validated_explicit =
        validate_and_expand_context_files(explicit_files, opts.changed_files, opts.repo_root)?;

    // Combine with deduplication: use HashSet to track seen files
    let mut seen_files: HashSet<String> = HashSet::new();
    let mut all_context_files = Vec::new();

    // Add explicit files first
    for file in validated_explicit {
        if seen_files.insert(file.clone()) {
            all_context_files.push(file);
        }
    }

    // Add directory and regex files, skipping duplicates
    for file in context_files {
        if seen_files.insert(file.clone()) {
            all_context_files.push(file);
        }
    }

    Ok(all_context_files)
}

/// Merges user-provided ignore patterns with default patterns
pub fn merge_ignore_patterns(user_patterns: &[String]) -> Vec<String> {
    if user_patterns.is_empty() {
        crate::cli::get_default_ignore_patterns()
    } else {
        let mut all_patterns = crate::cli::get_default_ignore_patterns();
        all_patterns.extend_from_slice(user_patterns);
        all_patterns
    }
}

/// Returns true if the pattern contains glob metacharacters (`*`, `?`, or `[`).
fn is_glob_pattern(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
}

/// Pre-compiled ignore patterns for efficient matching.
///
/// Glob patterns (containing `*`, `?`, or `[`) are compiled once into a `GlobSet`
/// using `.gitignore`-style semantics: `*` does not cross path separators (`/`),
/// and `**` is required for recursive matching. Each glob is prefixed with `**/`
/// so it matches anywhere in the tree. Literal patterns use path-component matching.
#[derive(Debug)]
pub struct IgnoreMatcher {
    /// Literal patterns for path-component matching (exact, prefix, suffix, component).
    literals: Vec<String>,
    /// Pre-compiled glob patterns; matches if ANY glob matches.
    globs: GlobSet,
}

impl IgnoreMatcher {
    /// Build a matcher from raw ignore patterns. Invalid globs are skipped with a warning.
    pub fn new(patterns: &[String]) -> Self {
        let mut literals = Vec::new();
        let mut builder = GlobSetBuilder::new();

        for pattern in patterns {
            if is_glob_pattern(pattern) {
                // Prefix with **/ so the pattern matches anywhere in the tree,
                // e.g. `mock-data/*.json` also catches `deep/mock-data/foo.json`.
                // (globset's ** matches zero segments, so root-level paths match too.)
                let anchored = if pattern.starts_with("**/") {
                    pattern.clone()
                } else {
                    format!("**/{pattern}")
                };
                match GlobBuilder::new(&anchored).literal_separator(true).build() {
                    Ok(glob) => {
                        builder.add(glob);
                    }
                    Err(e) => log::warn!("Invalid glob ignore pattern {pattern:?}: {e}"),
                }
            } else {
                literals.push(pattern.clone());
            }
        }

        let globs = builder.build().unwrap_or_else(|_| GlobSet::empty());
        IgnoreMatcher { literals, globs }
    }

    /// Returns true if `path` matches any ignore pattern.
    pub fn is_ignored(&self, path: &str) -> bool {
        if self.globs.is_match(path) {
            return true;
        }
        self.literals
            .iter()
            .any(|p| matches_literal_component(path, p))
    }
}

/// Check if a file path matches a literal (non-glob) pattern via path components.
/// Handles exact, prefix, suffix, and middle-component matching. Works on both
/// relative and absolute paths.
fn matches_literal_component(file: &str, pattern: &str) -> bool {
    // Exact match
    if file == pattern {
        return true;
    }

    // Pattern as directory prefix: "pattern/..."
    if let Some(rest) = file.strip_prefix(pattern)
        && rest.starts_with('/')
    {
        return true;
    }

    if let Some(rest) = file.strip_suffix(pattern)
        && rest.ends_with('/')
    {
        return true;
    }

    // Pattern as middle component: ".../pattern/..."
    let needle = format!("/{pattern}/");
    file.contains(&needle)
}

/// Filters out files that match any of the ignore patterns
pub fn filter_ignored_files(files: Vec<String>, matcher: &IgnoreMatcher) -> Vec<String> {
    files
        .into_iter()
        .filter(|file| !matcher.is_ignored(file))
        .collect()
}

/// Build ReviewData from diff results and common args
/// This consolidates the shared logic between git and jj extract_diff functions
pub fn build_review_data(
    changed_files: Vec<String>,
    diffs: std::collections::HashMap<String, String>,
    diff_target: String,
    common: &crate::cli::CommonArgs,
    repo_root: std::path::PathBuf,
) -> Result<crate::domain::ReviewData> {
    let all_ignore_patterns = merge_ignore_patterns(&common.ignore_files);
    let matcher = IgnoreMatcher::new(&all_ignore_patterns);
    let filtered_files = filter_ignored_files(changed_files, &matcher);

    let collect_opts = DirCollectOptions {
        matcher: &matcher,
        max_dir_files: common.max_dir_files,
        changed_files: &filtered_files,
        repo_root: &repo_root,
    };

    let validated_context_files = resolve_all_context_files(
        &common.context_files,
        &common.append_dirs,
        &common.context_file_regex,
        common.max_regex_files,
        collect_opts,
    )?;

    Ok(crate::domain::ReviewData {
        summary: diff_target,
        changed_files: filtered_files,
        diffs,
        context_files: validated_context_files,
        repo_root,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Build an IgnoreMatcher from string-literal patterns (test convenience).
    fn matcher(patterns: &[&str]) -> IgnoreMatcher {
        let pats: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
        IgnoreMatcher::new(&pats)
    }

    #[test]
    fn test_ignore_pattern_skips_directory() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("node_modules")).unwrap();
        fs::write(dir.path().join("node_modules/pkg.js"), "hello").unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

        let mut visited = HashSet::new();
        let mut count = 0;
        let mut truncated = false;
        let result = collect_files_from_directory(
            dir.path(),
            &HashSet::new(),
            &matcher(&["node_modules"]),
            &mut visited,
            100,
            &mut count,
            &mut truncated,
        )
        .unwrap();

        assert_eq!(result.len(), 1);
        assert!(result[0].ends_with("main.rs"));
        assert!(!truncated);
    }

    #[test]
    fn test_truncation_at_max_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("file1.rs"), "content1").unwrap();
        fs::write(dir.path().join("file2.rs"), "content2").unwrap();
        fs::write(dir.path().join("file3.rs"), "content3").unwrap();

        let mut visited = HashSet::new();
        let mut count = 0;
        let mut truncated = false;
        let result = collect_files_from_directory(
            dir.path(),
            &HashSet::new(),
            &matcher(&[]),
            &mut visited,
            2,
            &mut count,
            &mut truncated,
        )
        .unwrap();

        assert_eq!(result.len(), 2);
        assert!(truncated);
        assert_eq!(count, 2);
    }

    #[test]
    fn test_no_truncation_when_exactly_at_limit() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("file1.rs"), "content1").unwrap();
        fs::write(dir.path().join("file2.rs"), "content2").unwrap();

        let mut visited = HashSet::new();
        let mut count = 0;
        let mut truncated = false;
        let result = collect_files_from_directory(
            dir.path(),
            &HashSet::new(),
            &matcher(&[]),
            &mut visited,
            2,
            &mut count,
            &mut truncated,
        )
        .unwrap();

        assert_eq!(result.len(), 2);
        assert!(!truncated);
        assert_eq!(count, 2);
    }

    #[test]
    fn test_changed_files_excluded() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("file1.rs"), "content1").unwrap();
        fs::write(dir.path().join("file2.rs"), "content2").unwrap();

        let mut changed_files = HashSet::new();
        changed_files.insert(normalize_path(&dir.path().join("file1.rs")));

        let mut visited = HashSet::new();
        let mut count = 0;
        let mut truncated = false;
        let result = collect_files_from_directory(
            dir.path(),
            &changed_files,
            &matcher(&[]),
            &mut visited,
            100,
            &mut count,
            &mut truncated,
        )
        .unwrap();

        assert_eq!(result.len(), 1);
        assert!(result[0].ends_with("file2.rs"));
    }

    #[test]
    fn test_deterministic_ordering() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("z_file.rs"), "z").unwrap();
        fs::write(dir.path().join("a_file.rs"), "a").unwrap();
        fs::write(dir.path().join("m_file.rs"), "m").unwrap();

        let mut visited = HashSet::new();
        let mut count = 0;
        let mut truncated = false;
        let result = collect_files_from_directory(
            dir.path(),
            &HashSet::new(),
            &matcher(&[]),
            &mut visited,
            100,
            &mut count,
            &mut truncated,
        )
        .unwrap();

        // Files should be sorted alphabetically
        assert_eq!(result.len(), 3);
        assert!(result[0].ends_with("a_file.rs"));
        assert!(result[1].ends_with("m_file.rs"));
        assert!(result[2].ends_with("z_file.rs"));
    }

    #[test]
    #[cfg(unix)]
    fn test_symlink_loop_detection() {
        let dir = TempDir::new().unwrap();
        let subdir = dir.path().join("subdir");
        fs::create_dir(&subdir).unwrap();
        fs::write(subdir.join("file.rs"), "content").unwrap();

        // Create a symlink back to parent (loop)
        std::os::unix::fs::symlink(dir.path(), subdir.join("parent_link")).unwrap();

        let mut visited = HashSet::new();
        let mut count = 0;
        let mut truncated = false;
        let result = collect_files_from_directory(
            dir.path(),
            &HashSet::new(),
            &matcher(&[]),
            &mut visited,
            100,
            &mut count,
            &mut truncated,
        )
        .unwrap();

        // Should collect file.rs but not loop infinitely
        assert_eq!(result.len(), 1);
        assert!(result[0].ends_with("file.rs"));
    }

    #[test]
    fn test_ignore_matcher_glob() {
        // mock-data/*.json — with literal_separator, * does NOT cross /, so only
        // .json files directly under a mock-data/ dir match (anywhere in the tree
        // thanks to the automatic **/ prefix).
        let is = |file, pats: &[&str]| matcher(pats).is_ignored(file);

        assert!(is("mock-data/foo.json", &["mock-data/*.json"]));
        assert!(is("deep/mock-data/foo.json", &["mock-data/*.json"]));
        assert!(is("api-routes/X/mock-data/get.json", &["mock-data/*.json"]));
        assert!(!is("mock-data/sub/foo.json", &["mock-data/*.json"])); // * doesn't cross /
        assert!(!is("other/foo.txt", &["mock-data/*.json"]));

        // ** for recursive matching into nested subdirs
        assert!(is("mock-data/sub/foo.json", &["mock-data/**/*.json"]));

        // *.json — matches any .json anywhere in the tree (**/ prefix)
        assert!(is("readme.json", &["*.json"]));
        assert!(is("src/readme.json", &["*.json"]));

        // ? matches a single non-separator character
        assert!(is("test.rs", &["test.??"]));
        assert!(is("test.py", &["test.??"]));
        assert!(!is("test.rs", &["test.?"])); // only one char after .

        // Character class
        assert!(is("test.rs", &["test.[rsp]s"]));
        assert!(!is("test.py", &["test.[rsp]s"]));

        // Literal patterns (no glob chars) use path-component matching
        assert!(is("Cargo.lock", &["Cargo.lock"]));
        assert!(is("src/main.rs", &["main.rs"]));
        assert!(is("node_modules/pkg.js", &["node_modules"]));
    }
}
