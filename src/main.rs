use anyhow::Result;
use colored::Colorize;

use std::process::exit;

use crate::fs::format_path_for_display;

/// Print a consistent warning message to stderr, followed by a blank line.
pub fn print_warning(msg: &str) {
    eprintln!("{} {msg}\n", "Warning:".yellow());
}

mod cli;
mod clipboard;
mod domain;
mod fs;
mod git;
mod jj;
mod prompt;
mod tokenizer;

use cli::Cli;

fn main() {
    let cli = Cli::parse();

    // Determine if verbose was passed in either subcommand
    let verbose = match &cli.command {
        cli::Commands::Git(args) => args.common.verbose,
        cli::Commands::Jj(args) => args.common.verbose,
    };

    // Set default log level based on the verbose flag
    let log_level = if verbose { "debug" } else { "info" };

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level))
        .format_timestamp_secs()
        .init();

    if let Err(e) = run(cli) {
        eprintln!("Error: {e}");
        for cause in e.chain().skip(1) {
            eprintln!("  caused by: {cause}");
        }
        exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let (review_data, common) = match cli.command {
        cli::Commands::Git(args) => {
            let data = git::extract_diff(&args)?;
            (data, args.common)
        }
        cli::Commands::Jj(args) => {
            let data = jj::extract_diff(&args)?;
            (data, args.common)
        }
    };

    let result = prompt::generate(
        &review_data,
        common.prompt_file.as_deref(),
        common.diff_only,
        common.diff_only_large_files,
        common.skip_files_over_tokens,
        common.ignore_prompt,
    )?;

    if common.stats_only {
        print_stats(
            &review_data,
            result.prompt_tokens,
            result.file_content_tokens,
            false,
            &result.large_files_diff_only,
            &result.skipped_files_tokens,
        );
    } else {
        handle_output(
            result,
            common.copy_to_clipboard && !common.no_copy_to_clipboard,
            &review_data,
        )?;
    }

    Ok(())
}

/// Print stats summary and file list
fn print_stats(
    review_data: &crate::domain::ReviewData,
    prompt_tokens: usize,
    file_content_tokens: usize,
    copied: bool,
    large_files_diff_only: &[(String, usize)],
    skipped_files_tokens: &[(String, usize)],
) {
    // Trimmed files are annotated inline in the changed-files list, mirroring
    // how context files already get a `(context file)` suffix. Lists are tiny,
    // so a linear scan beats a HashMap alloc. Both vecs clone their paths
    // straight from `changed_files` (prompt.rs), so the strings match
    // byte-for-byte -- no path normalization needed.
    for file in &review_data.changed_files {
        // Token-skip wins over the line-threshold annotation: when both apply
        // the diff may be gone entirely, so "diff-only" would be a lie.
        if let Some((_, tokens)) = skipped_files_tokens.iter().find(|(f, _)| f == file) {
            println!(
                "  {file} {}",
                format!(
                    "(skipped: {} tokens)",
                    tokenizer::format_token_count(*tokens)
                )
                .yellow()
            );
        } else if let Some((_, lines)) = large_files_diff_only.iter().find(|(f, _)| f == file) {
            println!(
                "  {file} {}",
                format!("(diff-only: {lines} lines)").yellow()
            );
        } else {
            println!("  {file}");
        }
    }
    for file in &review_data.context_files {
        let display_path = format_path_for_display(file, &review_data.repo_root);
        println!("  {display_path} {}", "(context file)".blue());
    }
    println!();
    println!("Diffing against: {}", review_data.summary.blue());
    let copied_str = if copied {
        format!("   {}", "✔ Copied".green())
    } else {
        String::new()
    };
    let total_tokens = prompt_tokens + file_content_tokens;
    println!(
        "Files changed: {}, Tokens: {} (prompt: {}, files: {}){}",
        review_data.changed_files.len(),
        tokenizer::format_token_count(total_tokens),
        tokenizer::format_token_count(prompt_tokens),
        tokenizer::format_token_count(file_content_tokens),
        copied_str
    );
}

/// Handle output to stdout or clipboard
fn handle_output(
    result: prompt::PromptResult,
    copy_to_clipboard: bool,
    review_data: &crate::domain::ReviewData,
) -> Result<()> {
    if copy_to_clipboard {
        clipboard::copy_to_clipboard(&result.prompt)?;
        print_stats(
            review_data,
            result.prompt_tokens,
            result.file_content_tokens,
            true,
            &result.large_files_diff_only,
            &result.skipped_files_tokens,
        );
    } else {
        println!("{}", result.prompt);
    }
    Ok(())
}
