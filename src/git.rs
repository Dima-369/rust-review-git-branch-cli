use anyhow::{Result, bail};
use std::collections::HashMap;
use std::collections::HashSet;
use std::process::Command;

use crate::cli::GitArgs;
use crate::domain::ReviewData;
use crate::fs::{
    FileContent, build_review_data, get_local_file_content, get_repo_root, run_command,
};

pub fn extract_diff(args: &GitArgs) -> Result<ReviewData> {
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

    let (changed_files, diffs, diff_target) = get_diff_strategy(args, &repo_root)?;
    build_review_data(changed_files, diffs, diff_target, &args.common, repo_root)
}

fn get_diff_strategy(
    args: &GitArgs,
    repo_root: &std::path::Path,
) -> Result<(Vec<String>, HashMap<String, String>, String)> {
    let mut diffs = HashMap::new();
    let context = args.common.context;

    if args.head
        || args
            .target
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case("head"))
    {
        let has_head = head_exists(repo_root)?;

        let mut files = if has_head {
            get_git_changed_files(&["HEAD"], &args.common.paths, repo_root)?
        } else {
            get_staged_files(&args.common.paths, repo_root)?
        };
        let untracked = get_untracked_files(&args.common.paths, repo_root)?;

        if files.is_empty() && untracked.is_empty() {
            if !has_head {
                bail!("No staged or untracked files found. Stage files with 'git add' first.");
            }

            // No uncommitted changes — fall back to the parent commit, like jj does
            let (diff_range, target_name) = if parent_exists(repo_root)? {
                ("HEAD~1..HEAD".to_string(), "HEAD~1")
            } else {
                // Empty tree hash to support reviewing the initial commit
                let empty_tree = get_empty_tree_hash(repo_root)?;
                (format!("{empty_tree}..HEAD"), "initial commit")
            };

            let parent_files =
                get_git_changed_files(&[&diff_range], &args.common.paths, repo_root)?;
            if parent_files.is_empty() {
                bail!("No uncommitted changes found and {target_name} is also empty");
            }

            eprintln!("No uncommitted changes found, using {target_name}\n");

            for file in &parent_files {
                let diff = get_git_diff(&[&diff_range], file, context, repo_root)?;
                diffs.insert(file.clone(), diff);
            }

            return Ok((parent_files, diffs, target_name.to_string()));
        }

        let untracked_set: HashSet<String> = untracked.iter().cloned().collect();
        files.extend(untracked);

        for file in &files {
            if untracked_set.contains(file) || !has_head {
                let full_path = repo_root.join(file);
                match get_local_file_content(&full_path)? {
                    FileContent::Binary => {
                        diffs.insert(file.clone(), "Binary file".to_string());
                    }
                    FileContent::Deleted => {
                        diffs.insert(file.clone(), String::new());
                    }
                    FileContent::Content(_) => {
                        let diff = get_no_index_new_file_diff(repo_root, file, context)?;
                        diffs.insert(file.clone(), diff);
                    }
                }
            } else {
                let diff = get_git_diff(&["HEAD"], file, context, repo_root)?;
                diffs.insert(file.clone(), diff);
            }
        }

        Ok((files, diffs, "HEAD".to_string()))
    } else {
        let current_branch = get_current_git_branch(repo_root)?;

        let target_ref = match args.target.as_deref() {
            None | Some("smart") => detect_smart_git_branch(&current_branch, repo_root)?,
            Some(t) => t.to_string(),
        };

        let diff_target = if args.target.is_none() {
            format!("{target_ref} (smart)")
        } else {
            target_ref.clone()
        };

        if current_branch == target_ref {
            bail!("Already on target ref '{target_ref}'. Nothing to compare.");
        }

        let diff_range = format!("{target_ref}...");
        let files = get_git_changed_files(&[&diff_range], &args.common.paths, repo_root)?;
        if files.is_empty() {
            bail!("No changes found compared to '{target_ref}'");
        }

        for file in &files {
            let diff = get_git_diff(&[diff_range.as_str()], file, context, repo_root)?;
            diffs.insert(file.clone(), diff);
        }

        Ok((files, diffs, diff_target))
    }
}

fn get_current_git_branch(repo_root: &std::path::Path) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.args(["branch", "--show-current"])
        .current_dir(repo_root);
    let output = run_command(&mut cmd)?;
    if !output.status.success() {
        bail!("Failed to get current git branch. Are you in a git repository?");
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn parent_exists(repo_root: &std::path::Path) -> Result<bool> {
    let mut cmd = Command::new("git");
    cmd.args(["rev-parse", "--verify", "HEAD~1"])
        .current_dir(repo_root);
    Ok(run_command(&mut cmd)?.status.success())
}

fn get_empty_tree_hash(repo_root: &std::path::Path) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.args(["hash-object", "-t", "tree", "/dev/null"])
        .current_dir(repo_root);
    let output = run_command(&mut cmd)?;
    if !output.status.success() {
        bail!("Failed to get empty tree hash");
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn head_exists(repo_root: &std::path::Path) -> Result<bool> {
    let mut cmd = Command::new("git");
    cmd.args(["rev-parse", "--verify", "HEAD"])
        .current_dir(repo_root);
    let output = run_command(&mut cmd)?;
    Ok(output.status.success())
}

fn detect_smart_git_branch(current_branch: &str, repo_root: &std::path::Path) -> Result<String> {
    let candidates = ["develop", "master", "main"];
    for candidate in candidates {
        if current_branch == candidate {
            continue;
        }

        let mut cmd = Command::new("git");
        cmd.args(["rev-parse", "--verify", candidate])
            .current_dir(repo_root);
        let output = run_command(&mut cmd)?;
        if output.status.success() {
            return Ok(candidate.to_string());
        }
    }
    bail!(
        "No suitable base branch found. Checked: {}",
        candidates.join(", ")
    );
}

fn get_staged_files(paths: &[String], repo_root: &std::path::Path) -> Result<Vec<String>> {
    let mut args = vec!["diff", "--cached", "--name-only"];

    if !paths.is_empty() {
        args.push("--");
        args.extend(paths.iter().map(String::as_str));
    }

    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(repo_root);
    let output = run_command(&mut cmd)?;
    if !output.status.success() {
        bail!(
            "Failed to get staged files from git: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8(output.stdout)?
        .lines()
        .map(str::to_string)
        .collect())
}

fn get_untracked_files(paths: &[String], repo_root: &std::path::Path) -> Result<Vec<String>> {
    let mut args = vec!["ls-files", "--others", "--exclude-standard", "--full-name"];

    if !paths.is_empty() {
        args.push("--");
        args.extend(paths.iter().map(String::as_str));
    }

    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(repo_root);
    let output = run_command(&mut cmd)?;
    if !output.status.success() {
        bail!(
            "Failed to get untracked files from git: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8(output.stdout)?
        .lines()
        .map(str::to_string)
        .collect())
}

fn get_git_changed_files(
    revision_args: &[&str],
    paths: &[String],
    repo_root: &std::path::Path,
) -> Result<Vec<String>> {
    let mut args = vec!["diff", "--name-only"];
    args.extend_from_slice(revision_args);

    if !paths.is_empty() {
        args.push("--");
        args.extend(paths.iter().map(String::as_str));
    }

    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(repo_root);
    let output = run_command(&mut cmd)?;
    if !output.status.success() {
        bail!(
            "Failed to get changed files from git: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8(output.stdout)?
        .lines()
        .map(str::to_string)
        .collect())
}

fn get_no_index_new_file_diff(
    repo_root: &std::path::Path,
    file: &str,
    context: Option<u32>,
) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.arg("diff");

    if let Some(ctx) = context {
        cmd.arg(format!("--unified={ctx}"));
    }

    cmd.args(["--no-index", "--", "/dev/null", file])
        .current_dir(repo_root);

    let output = run_command(&mut cmd)?;

    if !(output.status.success() || output.status.code() == Some(1)) {
        bail!(
            "Failed to get diff for new file {}: {}",
            file,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8(output.stdout)?)
}

fn get_git_diff(
    revision_args: &[&str],
    file_path: &str,
    context: Option<u32>,
    repo_root: &std::path::Path,
) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.arg("diff");
    cmd.args(revision_args);

    if let Some(ctx) = context {
        cmd.arg(format!("--unified={ctx}"));
    }

    cmd.arg("--").arg(file_path).current_dir(repo_root);

    let output = run_command(&mut cmd)?;
    if !output.status.success() {
        bail!(
            "Failed to get git diff for {}: {}",
            file_path,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8(output.stdout)?)
}
