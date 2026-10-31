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

    let strategy = get_diff_strategy(args, &repo_root)?;
    let commits = match strategy.log_range.as_deref() {
        Some(range) => get_commit_subjects(&[range], repo_root.as_path())?,
        None => Vec::new(),
    };
    let base_commit = match strategy.base_rev.as_deref() {
        Some(rev) => get_commit_subjects(&["-1", rev], repo_root.as_path())?
            .into_iter()
            .next(),
        None => None,
    };
    let uncommitted_files = count_uncommitted_files(&args.common.paths, repo_root.as_path())?;
    let mut data = build_review_data(
        strategy.files,
        strategy.diffs,
        strategy.diff_target,
        &args.common,
        repo_root,
    )?;
    data.commits = commits;
    data.base_commit = base_commit;
    data.uncommitted_files = uncommitted_files;
    data.uncommitted_included = strategy.uncommitted_included;
    Ok(data)
}

struct DiffStrategy {
    files: Vec<String>,
    diffs: HashMap<String, String>,
    diff_target: String,
    /// Revision range for `git log` listing the commits being reviewed
    /// (None when reviewing only uncommitted changes).
    log_range: Option<String>,
    /// The commit the diff is taken against (shown for orientation).
    base_rev: Option<String>,
    /// Whether working-tree changes are part of the diff.
    uncommitted_included: bool,
}

fn get_diff_strategy(args: &GitArgs, repo_root: &std::path::Path) -> Result<DiffStrategy> {
    let mut diffs = HashMap::new();
    let context = args.common.context;

    if args
        .target
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("head") || t == "@")
    {
        if args.exclude_uncommitted {
            crate::print_warning(
                "--exclude-uncommitted has no effect with --target head (it reviews uncommitted changes).",
            );
        }
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
            // `git log` can't take a tree as a range start, so the log range is separate.
            let (diff_range, log_range, target_name, base_rev) = if parent_exists(repo_root)? {
                (
                    "HEAD~1..HEAD".to_string(),
                    "HEAD~1..HEAD".to_string(),
                    "HEAD~1",
                    Some("HEAD~1".to_string()),
                )
            } else {
                // Empty tree hash to support reviewing the initial commit
                let empty_tree = get_empty_tree_hash(repo_root)?;
                (
                    format!("{empty_tree}..HEAD"),
                    "HEAD".to_string(),
                    "initial commit",
                    None,
                )
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

            return Ok(DiffStrategy {
                files: parent_files,
                diffs,
                diff_target: target_name.to_string(),
                log_range: Some(log_range),
                base_rev,
                uncommitted_included: false,
            });
        }

        let untracked_set: HashSet<String> = untracked.iter().cloned().collect();
        files.extend(untracked);
        let base = has_head.then_some("HEAD");
        let diffs = collect_diffs(&files, &untracked_set, base, context, repo_root)?;

        Ok(DiffStrategy {
            files,
            diffs,
            diff_target: "HEAD".to_string(),
            log_range: None,
            base_rev: has_head.then(|| "HEAD".to_string()),
            uncommitted_included: true,
        })
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

        let merge_base = get_merge_base(&target_ref, repo_root)?;

        // Default: merge-base -> working tree (committed + uncommitted + untracked).
        // With --exclude-uncommitted (or if no merge-base), committed changes only.
        let (files, diffs, uncommitted_included) = match merge_base.as_deref() {
            Some(base) if !args.exclude_uncommitted => {
                let mut files = get_git_changed_files(&[base], &args.common.paths, repo_root)?;
                let untracked = get_untracked_files(&args.common.paths, repo_root)?;
                let untracked_set: HashSet<String> = untracked.iter().cloned().collect();
                files.extend(untracked);
                let diffs = collect_diffs(&files, &untracked_set, Some(base), context, repo_root)?;
                (files, diffs, true)
            }
            _ => {
                let diff_range = format!("{target_ref}...");
                let files = get_git_changed_files(&[&diff_range], &args.common.paths, repo_root)?;
                let mut diffs = HashMap::new();
                for file in &files {
                    let diff = get_git_diff(&[diff_range.as_str()], file, context, repo_root)?;
                    diffs.insert(file.clone(), diff);
                }
                (files, diffs, false)
            }
        };
        if files.is_empty() {
            bail!("No changes found compared to '{target_ref}'");
        }

        Ok(DiffStrategy {
            files,
            diffs,
            diff_target,
            log_range: Some(format!("{target_ref}..HEAD")),
            base_rev: merge_base,
            uncommitted_included,
        })
    }
}

/// List commits for the given `git log` args as "<short-hash> <subject>", newest first.
fn get_commit_subjects(log_args: &[&str], repo_root: &std::path::Path) -> Result<Vec<String>> {
    let mut cmd = Command::new("git");
    cmd.args(["log", "--no-color", "--format=%h %s"])
        .args(log_args)
        .current_dir(repo_root);
    let output = run_command(&mut cmd)?;
    if !output.status.success() {
        // Non-fatal: the commit list is informational only.
        log::debug!(
            "git log {log_args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

/// Diff each file against `base` (working tree side). Untracked files, or all
/// files when there is no base commit yet, are rendered as new-file diffs.
fn collect_diffs(
    files: &[String],
    untracked: &HashSet<String>,
    base: Option<&str>,
    context: Option<u32>,
    repo_root: &std::path::Path,
) -> Result<HashMap<String, String>> {
    let mut diffs = HashMap::new();

    for file in files {
        if untracked.contains(file) || base.is_none() {
            let full_path = repo_root.join(file);
            match get_local_file_content(&full_path)? {
                FileContent::Binary => {
                    diffs.insert(file.clone(), "Binary file".to_string());
                }
                FileContent::Deleted => {
                    diffs.insert(file.clone(), String::new());
                }
                content @ FileContent::Directory => {
                    // Untracked nested repo (e.g. a submodule git init'd but
                    // not yet registered) — git reports it as a single path
                    // ending in '/' rather than expanding its contents.
                    log::warn!(
                        "Skipping '{}': untracked directory (likely a nested/submodule repo)",
                        full_path.display()
                    );
                    diffs.insert(file.clone(), content.to_display_string());
                }
                FileContent::Content(_) => {
                    let diff = get_no_index_new_file_diff(repo_root, file, context)?;
                    diffs.insert(file.clone(), diff);
                }
            }
        } else if let Some(base) = base {
            let diff = get_git_diff(&[base], file, context, repo_root)?;
            diffs.insert(file.clone(), diff);
        }
    }

    Ok(diffs)
}

/// Merge-base of `target` and HEAD, i.e. what `git diff <target>...` diffs against.
fn get_merge_base(target: &str, repo_root: &std::path::Path) -> Result<Option<String>> {
    let mut cmd = Command::new("git");
    cmd.args(["merge-base", target, "HEAD"])
        .current_dir(repo_root);
    let output = run_command(&mut cmd)?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8(output.stdout)?.trim().to_string()))
}

/// Number of files with uncommitted changes (staged, unstaged, or untracked),
/// scoped to `paths` when given so the count matches the reviewed files.
fn count_uncommitted_files(paths: &[String], repo_root: &std::path::Path) -> Result<usize> {
    let mut cmd = Command::new("git");
    cmd.args(["status", "--porcelain"]);
    if !paths.is_empty() {
        cmd.arg("--").args(paths);
    }
    cmd.current_dir(repo_root);
    let output = run_command(&mut cmd)?;
    if !output.status.success() {
        return Ok(0);
    }
    Ok(String::from_utf8_lossy(&output.stdout).lines().count())
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
