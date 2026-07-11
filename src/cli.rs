use clap::{Args, Parser, Subcommand};

// --- Clap Structs ---

#[derive(Parser)]
#[command(name = "code-reviewer")]
#[command(
    version,
    about = "Generates AI-friendly code review prompts from Git or Jujutsu diffs"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Generate a review prompt using Git
    Git(GitArgs),
    /// Generate a review prompt using Jujutsu (jj)
    Jj(JjArgs),
}

#[derive(Args, Clone)]
pub struct CommonArgs {
    /// Number of context lines to show in diff
    #[arg(short = 'u', long)]
    pub context: Option<u32>,

    /// Show only diff, not full file content
    #[arg(long, default_value_t = false)]
    pub diff_only: bool,

    /// Copy output to clipboard (enabled by default)
    #[arg(long, default_value_t = true)]
    pub copy_to_clipboard: bool,

    /// Do not copy output to clipboard
    #[arg(long, conflicts_with = "copy_to_clipboard")]
    pub no_copy_to_clipboard: bool,

    /// Print only file count and token count to stdout
    #[arg(long, default_value_t = false)]
    pub stats_only: bool,

    /// Path to a text file containing custom prompt instructions
    #[arg(short = 'p', long)]
    pub prompt_file: Option<String>,

    /// Skip the default prompt text (only output diff + optional file contents)
    #[arg(long, conflicts_with = "prompt_file")]
    pub ignore_prompt: bool,

    /// Additional context files to include in the prompt (repeatable flag)
    #[arg(short = 'a', long = "context-file", value_name = "PATH")]
    pub context_files: Vec<String>,

    /// Recursively append all files from directories (repeatable, excludes files already in diff)
    #[arg(short = 'A', long = "append-dir", value_name = "DIR_PATH")]
    pub append_dirs: Vec<String>,

    /// Regex pattern to match file paths for context files (paths are relative to repo root, repeatable)
    #[arg(short = 'r', long = "context-file-regex", value_name = "PATTERN")]
    pub context_file_regex: Vec<String>,

    /// Maximum number of files to include from regex matching
    #[arg(long = "max-regex-files", value_name = "COUNT", default_value_t = 50)]
    pub max_regex_files: usize,

    /// Maximum number of files to include from each directory path
    #[arg(long = "max-dir-files", value_name = "COUNT", default_value_t = 100)]
    pub max_dir_files: usize,

    /// Files to ignore from the review (repeatable flag, defaults to common files like Cargo.lock)
    /// Supports glob patterns with `.gitignore` semantics: `*` does not cross path
    /// separators (`/`), so use `**` for recursive matching. Patterns match anywhere
    /// in the tree, e.g. `mock-data/*.json` ignores any `.json` directly under a
    /// `mock-data/` directory at any depth.
    #[arg(short = 'i', long = "ignore-file", value_name = "PATH_PATTERN")]
    pub ignore_files: Vec<String>,

    /// Specific files or directories to review
    #[arg(trailing_var_arg = true)]
    pub paths: Vec<String>,

    /// Print executed shell commands for debugging
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,

    /// Force use of current working directory instead of detecting via git/jj
    #[arg(long)]
    pub force_cwd: bool,
}

#[derive(Args, Clone)]
pub struct GitArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// Target ref to compare against: branch, tag, or commit (default: smart detection of develop/master/main)
    #[arg(short = 'b', long, conflicts_with = "head")]
    pub target: Option<String>,

    /// Review only uncommitted changes (current working directory diff)
    #[arg(long, default_value_t = false, conflicts_with = "target")]
    pub head: bool,
}

#[derive(Args, Clone)]
pub struct JjArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// Target revision to compare against (default: smart detection of develop/master/main)
    #[arg(short, long, conflicts_with = "head")]
    pub target: Option<String>,

    /// Review changes in the current working revision (@)
    #[arg(long, default_value_t = false, conflicts_with = "target")]
    pub head: bool,
}

pub fn get_default_ignore_patterns() -> Vec<String> {
    vec![
        "Cargo.lock".to_string(),
        ".gitignore".to_string(),
        ".gitmodules".to_string(),
        "composer.lock".to_string(),
        ".DS_Store".to_string(),
        "Thumbs.db".to_string(),
        "node_modules".to_string(),
        "target".to_string(),
        ".idea".to_string(),
        ".vscode".to_string(),
        ".vim".to_string(),
        ".nvim".to_string(),
        "dist".to_string(),
        "build".to_string(),
        "out".to_string(),
        "vendor".to_string(),
        "Pods".to_string(),
        "pnpm-lock.yaml".to_string(),
        // Version control directories
        ".git".to_string(),
        ".svn".to_string(),
        ".hg".to_string(),
        ".jj".to_string(),
        "CVS".to_string(),
        ".bzr".to_string(),
    ]
}

// CLI-related helper functions
impl Cli {
    pub fn parse() -> Self {
        <Self as Parser>::parse()
    }
}
