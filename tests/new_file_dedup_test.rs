mod utils;

use predicates::prelude::*;
use std::fs::File;
use std::io::Write;
use utils::{code_reviewer_cmd, setup_git_repo};

#[test]
fn test_new_file_full_contents_skipped_for_git_added_file() {
    let temp = setup_git_repo();
    let root = temp.path();

    // Create a brand-new file and stage it
    let new_file = root.join("brand_new.rs");
    let mut f = File::create(&new_file).unwrap();
    writeln!(f, "fn hello() {{}}\nfn world() {{}}").unwrap();

    std::process::Command::new("git")
        .args(["add", "brand_new.rs"])
        .current_dir(root)
        .output()
        .unwrap();

    // Run with --head (no --diff-only). The diff is a "new file" diff,
    // so "## Full File Contents" should NOT repeat the file.
    let mut cmd = code_reviewer_cmd();
    let output = cmd
        .current_dir(root)
        .args(["git", "--head", "--no-copy-to-clipboard"])
        .output()
        .expect("failed to run");

    let stdout = String::from_utf8(output.stdout).unwrap();

    // The diff section should exist and contain the new file diff
    assert!(
        stdout.contains("## Detailed File Diffs"),
        "Missing diffs section"
    );
    assert!(
        stdout.contains("+fn hello()"),
        "Diff should contain the added lines"
    );

    // The full contents section should NOT include this file since the diff
    // already contains the entire file (new-file diff).
    // Count how many times the function body appears — it should appear only once
    // (in the diff), not twice (diff + full contents).
    let occurrences = stdout.matches("fn hello()").count();
    assert_eq!(
        occurrences, 1,
        "New-file content appears {occurrences} times; expected exactly 1 (only in the diff). \
         The full file contents section should be skipped for new files to avoid duplication."
    );
}

#[test]
fn test_untracked_file_has_proper_diff_headers() {
    let temp = setup_git_repo();
    let root = temp.path();

    // Create an untracked file (do NOT git add)
    let new_file = root.join("untracked.rs");
    let mut f = File::create(&new_file).unwrap();
    writeln!(f, "fn untracked() {{}}").unwrap();

    let mut cmd = code_reviewer_cmd();
    cmd.current_dir(root)
        .args(["git", "--head", "--no-copy-to-clipboard"])
        .assert()
        .success()
        .stdout(predicates::str::contains("untracked.rs"))
        // A proper git diff --no-index produces canonical headers.
        // The current code generates fake diffs without these headers.
        .stdout(
            predicates::str::contains("--- /dev/null")
                .or(predicates::str::contains("new file mode")),
        );
}

#[test]
fn test_new_file_on_branch_full_contents_skipped() {
    let temp = setup_git_repo();
    let root = temp.path();

    // Create a feature branch
    std::process::Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(root)
        .output()
        .unwrap();

    // Add a completely new file and commit it
    let new_file = root.join("new_feature.rs");
    let mut f = File::create(&new_file).unwrap();
    writeln!(f, "pub fn feature_code() {{}}\npub fn more_code() {{}}").unwrap();

    std::process::Command::new("git")
        .args(["add", "new_feature.rs"])
        .current_dir(root)
        .output()
        .unwrap();

    std::process::Command::new("git")
        .args(["commit", "-m", "Add new feature file"])
        .current_dir(root)
        .output()
        .unwrap();

    // Run against the base branch — the diff for new_feature.rs is a "new file" diff
    let output = code_reviewer_cmd()
        .current_dir(root)
        .args(["git", "--no-copy-to-clipboard"])
        .output()
        .expect("failed to run");

    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(
        stdout.contains("new_feature.rs"),
        "Output should mention the new file"
    );

    let occurrences = stdout.matches("feature_code()").count();
    assert_eq!(
        occurrences, 1,
        "New-file content appears {occurrences} times; expected exactly 1 (only in the diff). \
         Full file contents should be skipped for new files."
    );
}
