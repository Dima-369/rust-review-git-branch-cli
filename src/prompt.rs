use crate::domain::ReviewData;
use crate::fs::get_local_file_content;
use crate::tokenizer::get_token_count;
use anyhow::Result;

const DEFAULT_PROMPT: &str = "You are an expert code reviewer. Please review the following changes and provide feedback on potential bugs, style issues, performance improvements, and adherence to best practices.";

fn diff_looks_like_new_file(diff: &str) -> bool {
    diff.contains("--- /dev/null") || diff.contains("new file mode") || diff.contains("@@ -0,0 +")
}

pub struct PromptResult {
    pub prompt: String,
    pub prompt_tokens: usize,
    pub file_content_tokens: usize,
    /// Files shown as diff-only because they exceeded the `--diff-only-large-files`
    /// line threshold. Each tuple is `(file_path, line_count)`. Used for the stats
    /// print so the user can see which files were trimmed.
    pub large_files_diff_only: Vec<(String, usize)>,
}

/// Generate a code review prompt from ReviewData
pub fn generate(
    review_data: &ReviewData,
    custom_prompt_file: Option<&str>,
    diff_only: bool,
    diff_only_large_files: Option<usize>,
    ignore_prompt: bool,
) -> Result<PromptResult> {
    let mut prompt_part = String::new();
    let mut file_content_tokens = 0;

    let repo_root = &review_data.repo_root;

    if !ignore_prompt {
        let prompt_text = match custom_prompt_file {
            Some(file_path) => std::fs::read_to_string(file_path).unwrap_or_else(|e| {
                crate::print_warning(&format!(
                    "prompt file '{file_path}' not found: {e}. Continuing without any prefix prompt."
                ));
                String::new()
            }),
            None => DEFAULT_PROMPT.to_string(),
        };
        prompt_part.push_str(&prompt_text);
        if !prompt_text.is_empty() {
            prompt_part.push_str("\n\n");
        }
    }

    if !review_data.context_files.is_empty() {
        prompt_part.push_str("## Context Files\n\n");
        for file_path in &review_data.context_files {
            let display_path = pathdiff::diff_paths(file_path, repo_root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| file_path.clone());

            prompt_part.push_str(&format!(">>>> {display_path}\n"));

            let content = get_local_file_content(file_path)?.to_display_string();
            file_content_tokens += get_token_count(&content);

            prompt_part.push_str(&content);
            if !content.ends_with('\n') {
                prompt_part.push('\n');
            }
            prompt_part.push_str("<<<<\n\n");
        }

        prompt_part.push_str("---\n\n");
    }

    // Pre-scan file line counts once so we can both annotate the summary and
    // skip bloated files in the "Full File Contents" section. Line count comes
    // from the working-tree content (what would actually be appended).
    let large_threshold = diff_only_large_files.unwrap_or(usize::MAX);
    let mut large_files_diff_only: Vec<(String, usize)> = Vec::new();
    if !diff_only && large_threshold != usize::MAX {
        for file in &review_data.changed_files {
            if let Some(diff) = review_data.diffs.get(file)
                && diff_looks_like_new_file(diff)
            {
                continue;
            }
            let full_path = repo_root.join(file);
            let line_count = match get_local_file_content(full_path)? {
                crate::fs::FileContent::Content(c) => c.lines().count(),
                // Deleted/binary placeholders are tiny; never treat as large.
                _ => 0,
            };
            if line_count > large_threshold {
                large_files_diff_only.push((file.clone(), line_count));
            }
        }
    }

    prompt_part.push_str("## Summary of Changes\n\n");
    let file_count = review_data.changed_files.len();
    if file_count == 1 {
        prompt_part.push_str("The following 1 file was changed:\n");
    } else {
        prompt_part.push_str(&format!("The following {file_count} files were changed:\n"));
    }
    for file in &review_data.changed_files {
        if let Some(&(_, lines)) = large_files_diff_only.iter().find(|(f, _)| f == file) {
            prompt_part.push_str(&format!(
                "- `{file}` *(diff-only: {lines} lines exceeds threshold {large_threshold})*\n"
            ));
        } else {
            prompt_part.push_str(&format!("- `{file}`\n"));
        }
    }

    prompt_part.push('\n');

    prompt_part.push_str("## Detailed File Diffs\n\n");

    for file in &review_data.changed_files {
        prompt_part.push_str(&format!("### `{file}`\n\n"));
        if let Some(diff) = review_data.diffs.get(file)
            && !diff.trim().is_empty()
        {
            prompt_part.push_str("```diff\n");
            prompt_part.push_str(diff);
            if !diff.ends_with('\n') {
                prompt_part.push('\n');
            }
            prompt_part.push_str("```\n\n");
        }
    }

    // Only include full file contents if not in diff-only mode
    if !diff_only {
        prompt_part.push_str("## Full File Contents\n\n");
        for file in &review_data.changed_files {
            if let Some(diff) = review_data.diffs.get(file)
                && diff_looks_like_new_file(diff)
            {
                continue;
            }
            // Skip files above the large-file threshold (already shown as diff-only).
            if large_files_diff_only.iter().any(|(f, _)| f == file) {
                continue;
            }
            prompt_part.push_str(&format!(">>>> {file}\n"));
            let full_path = repo_root.join(file);
            let file_content = get_local_file_content(full_path)?.to_display_string();

            let mut content_for_stats = file_content.clone();
            if !content_for_stats.ends_with('\n') {
                content_for_stats.push('\n');
            }
            file_content_tokens += get_token_count(&content_for_stats);

            prompt_part.push_str(&file_content);
            if !file_content.ends_with('\n') {
                prompt_part.push('\n');
            }
            prompt_part.push_str("<<<<\n\n");
        }
    }

    if !ignore_prompt {
        prompt_part.push_str(
            "\n---\nTask: Review the changes above based on the instructions provided.\n",
        );
    }

    let total_tokens = get_token_count(&prompt_part);
    let prompt_tokens = total_tokens.saturating_sub(file_content_tokens);

    Ok(PromptResult {
        prompt: prompt_part,
        prompt_tokens,
        file_content_tokens,
        large_files_diff_only,
    })
}
