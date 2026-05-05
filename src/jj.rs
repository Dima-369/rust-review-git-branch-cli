use anyhow::{Result, bail};
use colored::Colorize;
use std::collections::HashMap;
use std::process::Command;

use crate::cli::{GitArgs, JjArgs};
use crate::domain::ReviewData;
use std::path::Path;

use crate::fs::{build_review_data, get_repo_root, run_command};

/// Fall back to using git operations when jj is not available.
/// Maps jj args to equivalent git operations:
///   - jj --head       => git --head (uncommitted/staged changes)
///   - jj (smart)      => git (smart branch detection)
///   - jj -b <target>  => git -b <target>
fn fallback_to_git(args: &JjArgs) -> Result<ReviewData> {
    let git_args = GitArgs {
        common: args.common.clone(),
        target: args.target.clone(),
        head: args.head,
    };

    crate::git::extract_diff(&git_args)
}

/// Try to extract diff using jj. If jj is not installed or there's no jj repo, fall back to git.
fn try_jj_or_fallback(args: &JjArgs, repo_root: &Path) -> Result<ReviewData> {
    // Check if jj is installed and if there's a jj repo via "jj root"
    let jj_root_check = match std::process::Command::new("jj")
        .args(["root"])
        .current_dir(repo_root)
        .output()
    {
        Ok(output) if output.status.success() => {
            // jj repo exists, proceed with jj operations
            let (changed_files, diffs, diff_target) = get_diff_strategy(args, repo_root)?;
            return build_review_data(
                changed_files,
                diffs,
                diff_target,
                &args.common,
                repo_root.to_path_buf(),
            );
        }
        Ok(output) => output,
        Err(_) => {
            // jj binary not found
            eprintln!(
                "{} Jujutsu (jj) not found, falling back to git\n",
                "Warning:".yellow()
            );
            return fallback_to_git(args);
        }
    };

    let stderr = String::from_utf8_lossy(&jj_root_check.stderr);
    if stderr.contains("There is no jj repo") {
        eprintln!(
            "{} Not a JJ repo, falling back to git\n",
            "Warning:".yellow()
        );
        fallback_to_git(args)
    } else {
        bail!("jj root failed: {stderr}");
    }
}

pub fn extract_diff(args: &JjArgs) -> Result<ReviewData> {
    let repo_root = get_repo_root(args.common.force_cwd)?;

    log::debug!(
        "Using repo root: {}{}",
        repo_root.display(),
        if args.common.force_cwd {
            " (forced via --force-cwd)"
        } else {
            ""
        }
    );

    try_jj_or_fallback(args, &repo_root)
}

fn get_diff_strategy(
    args: &JjArgs,
    repo_root: &Path,
) -> Result<(Vec<String>, HashMap<String, String>, String)> {
    let mut diffs = HashMap::new();
    let context = args.common.context;

    if args.head
        || args
            .target
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case("head") || t == "@")
    {
        let files = get_jj_changed_files(None, &args.common.paths, repo_root)?;
        if files.is_empty() {
            let prev_files =
                get_jj_changed_files_from_to("@--", "@-", &args.common.paths, repo_root)?;
            if prev_files.is_empty() {
                bail!("No uncommitted changes found and previous commit is also empty");
            }

            eprintln!("No uncommitted changes found, using previous commit '@-'\n");

            for file in &prev_files {
                let diff = get_jj_diff_from_to("@--", "@-", file, context, repo_root)?;
                diffs.insert(file.clone(), diff);
            }

            Ok((prev_files, diffs, "@-".to_string()))
        } else {
            for file in &files {
                let diff = get_jj_diff(None, file, context, repo_root)?;
                diffs.insert(file.clone(), diff);
            }

            Ok((files, diffs, "@".to_string()))
        }
    } else {
        let target_revision = match args.target.as_deref() {
            None | Some("smart") => detect_smart_jj_revision(repo_root)?,
            Some(t) => t.to_string(),
        };

        let diff_target = if args.target.is_none() {
            format!("{target_revision} (smart)")
        } else {
            target_revision.clone()
        };

        let files = get_jj_changed_files(Some(&target_revision), &args.common.paths, repo_root)?;
        if files.is_empty() {
            bail!("No changes found in the working copy compared to revision '{target_revision}'");
        }

        for file in &files {
            let diff = get_jj_diff(Some(&target_revision), file, context, repo_root)?;
            diffs.insert(file.clone(), diff);
        }

        Ok((files, diffs, diff_target))
    }
}

fn detect_smart_jj_revision(repo_root: &Path) -> Result<String> {
    let candidates = ["develop", "master", "main"];
    for candidate in candidates {
        let mut cmd = Command::new("jj");
        cmd.args(["diff", "--from", candidate, "--to", "@", "--summary"])
            .current_dir(repo_root);
        let output = run_command(&mut cmd)?;
        if output.status.success() && !output.stdout.is_empty() {
            return Ok(candidate.to_string());
        }
    }
    bail!(
        "No suitable base revision found with changes. Checked: {}",
        candidates.join(", ")
    );
}

fn get_jj_changed_files(
    target_revision: Option<&str>,
    paths: &[String],
    repo_root: &Path,
) -> Result<Vec<String>> {
    let mut args = vec!["diff", "--name-only"];
    if let Some(rev) = target_revision {
        args.extend(["--from", rev, "--to", "@"]);
    }

    if !paths.is_empty() {
        args.push("--");
        args.extend(paths.iter().map(String::as_str));
    }

    let mut cmd = Command::new("jj");
    cmd.args(args).current_dir(repo_root);
    let output = run_command(&mut cmd)?;
    if !output.status.success() {
        bail!(
            "Failed to get changed files from jj: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8(output.stdout)?
        .lines()
        .map(str::to_string)
        .collect())
}

fn get_jj_diff(
    target_revision: Option<&str>,
    file_path: &str,
    context: Option<u32>,
    repo_root: &Path,
) -> Result<String> {
    let from_to_args = if let Some(rev) = target_revision {
        vec!["--from", rev, "--to", "@"]
    } else {
        vec![]
    };
    run_jj_diff(&from_to_args, file_path, context, repo_root)
}

fn get_jj_changed_files_from_to(
    from_revision: &str,
    to_revision: &str,
    paths: &[String],
    repo_root: &Path,
) -> Result<Vec<String>> {
    let args = [
        "diff",
        "--name-only",
        "--from",
        from_revision,
        "--to",
        to_revision,
    ];

    let mut cmd = Command::new("jj");
    cmd.args(args).current_dir(repo_root);

    if !paths.is_empty() {
        cmd.arg("--");
        cmd.args(paths.iter().map(String::as_str));
    }

    let output = run_command(&mut cmd)?;
    if !output.status.success() {
        bail!(
            "Failed to get changed files from jj for revision range {}..{}: {}",
            from_revision,
            to_revision,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8(output.stdout)?
        .lines()
        .map(str::to_string)
        .collect())
}

fn get_jj_diff_from_to(
    from_revision: &str,
    to_revision: &str,
    file_path: &str,
    context: Option<u32>,
    repo_root: &Path,
) -> Result<String> {
    let from_to_args = vec!["--from", from_revision, "--to", to_revision];
    run_jj_diff(&from_to_args, file_path, context, repo_root)
}

// Common helper function to run jj diff with specified from/to arguments
fn run_jj_diff(
    from_to_args: &[&str], // Arguments like ["--from", "rev1", "--to", "rev2"]
    file_path: &str,
    context: Option<u32>,
    repo_root: &Path,
) -> Result<String> {
    let mut args = vec!["diff"];
    args.extend(from_to_args);

    // Add context if specified
    let context_string = context.map(|c| c.to_string());
    if let Some(ref ctx_str) = context_string {
        args.extend(["--context", ctx_str]);
    }

    args.extend(["--git", "--", file_path]);

    let mut cmd = Command::new("jj");
    cmd.args(args).current_dir(repo_root);
    let output = run_command(&mut cmd)?;
    if !output.status.success() {
        bail!(
            "Failed to get jj diff for {}: {}",
            file_path,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8(output.stdout)?)
}
