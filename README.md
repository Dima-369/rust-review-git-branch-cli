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
