Supports both `jj` and `git`.

It writes to stdout and copies to clipboard by default.
The output includes the diff of files and the full file content for context.

There is the `--target` flag which is set to `smart` by default and diffs against the first branch found of those: `develop/master/main`

```bash
cargo install --path .

code-reviewer git
code-reviewer jj --prompt-file ~/vero/vero-code-review-prompt.md
code-reviewer jj --head

# This requires the rg binary and runs `rg --files` from the repository root.
# The regex matches against file paths *relative to the repository root* (e.g., src/main.rs).
# It does not search by file content, only file paths.
# Note that it respects .gitignore.
code-reviewer jj --context-file-regex 'Cargo\.toml|Cargo\.lock'
code-reviewer jj --context-file-regex '^src/.*\.rs$'
```

You can specify multiple `--context-file` flags to pass in other files for context.

# Example output

```
Diffing against: master (smart)

  README.md

Files changed: 1, Tokens: ~1,286  ✓ Copied
```

---

# AI SLOP below

# Code Reviewer

A Rust CLI tool that generates AI-friendly code review prompts from Git or Jujutsu (`jj`) diffs. Perfect for getting high-quality code reviews from AI assistants like ChatGPT, Claude, or GitHub Copilot.

## Features

- 🥡 **Multi-VCS Support**: Works seamlessly with both **Git** and **Jujutsu (`jj`)**.
- 🧠 **Smart Base Detection**: Automatically detects and compares against `develop`, `master`, or `main`.
- 🎯 **Targeted Reviews**: Focus on specific files or directories.
- 📝 **AI-Optimized Format**: Uses a hybrid Markdown + diff format for maximum AI comprehension.
- ⚡ **Fast & Reliable**: Direct integration with your version control system.
- 🔧 **Flexible**: Works with any repository and branch/revision structure.

## Installation

### From Source

```bash
git clone <repository-url>
cd code-reviewer
cargo build --release
# The binary will be at target/release/code-reviewer
cargo install --path .
```

The tool will be available as `code-reviewer`.

## Usage

The tool is structured with subcommands for each version control system.

### Git Usage

```bash
# Smart detection - compares current branch against develop/master/main
code-reviewer git

# Specify a target branch
code-reviewer git --branch main

# Review specific files only
code-reviewer git src/lib.rs src/main.rs

# Review a specific directory
code-reviewer git src/

# Combine branch and file targeting
code-reviewer git --branch develop -- src/api/
```

### Jujutsu (`jj`) Usage

```bash
# Smart detection - compares working copy against develop/master/main
code-reviewer jj

# Specify a target revision
code-reviewer jj --target main

# Review specific files only
code-reviewer jj src/lib.rs src/main.rs

# Review a specific directory
code-reviewer jj src/
```

**Note on `jj` diff direction**: The tool generates the diff from a target revision (like `develop`) to your current working copy (` @`), using a command equivalent to `jj diff -f {target} -t @`. This shows the changes you have made, which is standard for a code review.

### CLI Options

#### `git` subcommand

| Flag | Short | Description | Default |
|------|-------|-------------|---------|
| `--branch` | `-b` | Branch to compare against | `smart` |
| `--context` | `-c` | Number of context lines for diff | `git default` |

#### `jj` subcommand

| Flag | Short | Description | Default |
|------|-------|-------------|---------|
| `--target` | `-t` | Revision to compare against | `smart` |

**Positional Arguments (both commands):**
- `[PATHS]...` - Specific files or directories to review (optional)

## Output Format

The tool generates a structured prompt optimized for AI code review:

```markdown
You are an expert code reviewer. Please review the following changes...

## Summary of Changes

Comparing current changes against `develop` (using Jujutsu).

The following files were changed:
- `src/lib.rs`
- `src/main.rs`

## Detailed File Diffs

### `src/lib.rs`

```diff
--- a/src/lib.rs
+++ b/src/lib.rs
 @@ -1,3 +1,5 @@
+use anyhow::Result;
+
 pub fn hello() -> String {
-    "Hello, World!".to_string()
+    "Hello, Rust!".to_string()
 }
```

---
**End of review request.** Please provide your feedback below.
```

## Workflow Integration

```bash
# Save to file (macOS/Linux)
code-reviewer git > review.md

# Copy to clipboard (macOS)
code-reviewer jj | pbcopy

# Copy to clipboard (Linux with xclip)
code-reviewer git | xclip -selection clipboard
```

## Error Handling

The tool provides clear error messages for common scenarios:

- **No changes found**: When there are no differences from the target.
- **Branch/revision not found**: When the specified target doesn't exist.
- **Not a repository**: When run outside a Git or `jj` repository.
