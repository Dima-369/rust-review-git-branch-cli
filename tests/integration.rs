mod utils;

use predicates::prelude::*;
use std::fs::{self, File};
use std::io::Write;
use utils::{code_reviewer_cmd, setup_git_repo, setup_jj_repo};

// --- Git Tests ---

#[test]
fn test_git_head_changes() {
    let temp = setup_git_repo();
    let root = temp.path();

    // Make a modification (unstaged)
    let file_path = root.join("README.md");
    let mut file = fs::OpenOptions::new().append(true).open(file_path).unwrap();
    writeln!(file, "New line added").unwrap();

    // Run tool with --head
    let mut cmd = code_reviewer_cmd();
    cmd.current_dir(root)
        .args(["git", "--head", "--no-copy-to-clipboard"])
        .assert()
        .success()
        .stdout(predicates::str::contains("diff --git"))
        .stdout(predicates::str::contains("+New line added"))
        .stdout(predicates::str::contains("README.md"));
}

#[test]
fn test_git_head_with_untracked_nested_repo() {
    // Regression test: an untracked nested git repo (e.g. a submodule that was
    // `git init`'d but not yet registered) is reported by
    // `git ls-files --others` as a single directory path ("data/") rather than
    // being expanded into files. Previously this crashed with
    // "Failed to read file '...': Is a directory (os error 21)".
    let temp = setup_git_repo();
    let root = temp.path();

    let nested = root.join("data");
    fs::create_dir(&nested).unwrap();
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(&nested)
        .output()
        .unwrap();
    fs::write(nested.join("inner.txt"), "inner content").unwrap();

    let mut cmd = code_reviewer_cmd();
    cmd.current_dir(root)
        .args(["git", "--head", "--no-copy-to-clipboard"])
        .assert()
        .success()
        .stdout(predicates::str::contains("data/"))
        .stdout(predicates::str::contains("nested repo"));
}

#[test]
fn test_git_smart_branch_detection() {
    let temp = setup_git_repo();
    let root = temp.path();

    // Create a new branch 'feature'
    std::process::Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(root)
        .output()
        .unwrap();

    // Modify file and commit
    let file_path = root.join("README.md");
    let mut file = fs::OpenOptions::new().append(true).open(file_path).unwrap();
    writeln!(file, "Feature changes").unwrap();

    std::process::Command::new("git")
        .args(["commit", "-am", "Feature work"])
        .current_dir(root)
        .output()
        .unwrap();

    // Run tool (default should find master/main and diff against it)
    let mut cmd = code_reviewer_cmd();
    cmd.current_dir(root)
        .args(["git", "--no-copy-to-clipboard"]) // Defaults to smart detection
        .assert()
        .success()
        .stdout(predicates::str::contains("diff --git")) // Should contain diff information
        .stdout(predicates::str::contains("+Feature changes"));
}

#[test]
fn test_git_subdirectory_execution() {
    let temp = setup_git_repo();
    let root = temp.path();

    // Create subdir and a new file inside
    let subdir = root.join("subdir");
    fs::create_dir(&subdir).unwrap();

    let file_path = subdir.join("new.rs");
    File::create(&file_path).unwrap();

    // Stage it
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(root)
        .output()
        .unwrap();

    // Run code-reviewer FROM THE SUBDIRECTORY
    let mut cmd = code_reviewer_cmd();
    cmd.current_dir(&subdir)
        .args(["git", "--head", "--no-copy-to-clipboard"])
        .assert()
        .success()
        .stdout(predicates::str::contains("subdir/new.rs")); // Should correctly resolve path
}

#[test]
fn test_git_context_file() {
    let temp = setup_git_repo();
    let root = temp.path();

    // Create a context file that IS NOT modified (so it's truly a context file)
    let ctx_path = root.join("context.txt");
    let mut file = File::create(&ctx_path).unwrap();
    writeln!(file, "Important context info").unwrap();

    // Commit this context file so it's not considered a changed file
    std::process::Command::new("git")
        .args(["add", "context.txt"])
        .current_dir(root)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "Add context file"])
        .current_dir(root)
        .output()
        .unwrap();

    // Modify readme (this creates a change to review)
    let readme_path = root.join("README.md");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(readme_path)
        .unwrap();
    writeln!(file, "change").unwrap();

    let mut cmd = code_reviewer_cmd();
    cmd.current_dir(root)
        .args([
            "git",
            "--head",
            "--context-file",
            "context.txt",
            "--no-copy-to-clipboard",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("## Context Files"))
        .stdout(predicates::str::contains("Important context info"));
}

// --- Jujutsu Tests ---

#[test]
fn test_jj_working_copy_diff() {
    if std::process::Command::new("jj")
        .arg("--version")
        .output()
        .is_err()
    {
        return; // Skip if jj not installed
    }

    let temp = setup_jj_repo();
    let root = temp.path();

    // Modify file in working copy
    let file_path = root.join("src/lib.rs");
    let mut file = fs::OpenOptions::new().append(true).open(file_path).unwrap();
    writeln!(file, "pub fn test() {{}}").unwrap();

    // Run jj command
    // Note: In setup_jj_repo we created a 'main' bookmark and started a new change off it.
    // 'code-reviewer jj' (smart) should detect 'main' as the target.
    let mut cmd = code_reviewer_cmd();
    cmd.current_dir(root)
        .args(["jj", "--no-copy-to-clipboard"])
        .assert()
        .success()
        .stdout(predicates::str::contains("src/lib.rs"))
        .stdout(predicates::str::contains("+pub fn test() {}"));
}

#[test]
fn test_jj_subdirectory_execution() {
    if std::process::Command::new("jj")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }

    let temp = setup_jj_repo();
    let root = temp.path();
    let subdir = root.join("src"); // this exists from setup

    // Modify file
    let file_path = subdir.join("lib.rs");
    let mut file = fs::OpenOptions::new().append(true).open(file_path).unwrap();
    writeln!(file, "// changes").unwrap();

    // Run from subdirectory
    let mut cmd = code_reviewer_cmd();
    cmd.current_dir(&subdir)
        .args(["jj", "--no-copy-to-clipboard"])
        .assert()
        .success()
        .stdout(predicates::str::contains("src/lib.rs")); // Should show full path relative to repo root
}

#[test]
fn test_jj_specific_path_arg() {
    if std::process::Command::new("jj")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }

    let temp = setup_jj_repo();
    let root = temp.path();

    // Modify two files
    let file1 = root.join("src/lib.rs");
    let mut f1 = fs::OpenOptions::new().append(true).open(file1).unwrap();
    writeln!(f1, "// change 1").unwrap();

    let file2 = root.join("README.md");
    let mut f2 = File::create(file2).unwrap();
    writeln!(f2, "New file").unwrap();

    // Run targeting only lib.rs
    let mut cmd = code_reviewer_cmd();
    cmd.current_dir(root)
        .args(["jj", "src/lib.rs", "--no-copy-to-clipboard"])
        .assert()
        .success()
        .stdout(predicates::str::contains("src/lib.rs"))
        .stdout(predicates::str::contains("README.md").not()); // Should NOT contain readme
}

// --- Ignore File Tests ---

#[test]
fn test_git_ignore_file_default_patterns() {
    let temp = setup_git_repo();
    let root = temp.path();

    // Create a Cargo.lock file that should be ignored by default
    let cargo_lock = root.join("Cargo.lock");
    let mut file = File::create(&cargo_lock).unwrap();
    writeln!(file, "[[package]]\nname = \"test\"").unwrap();
    std::process::Command::new("git")
        .args(["add", "Cargo.lock"])
        .current_dir(root)
        .output()
        .unwrap();

    // Also create another file that should appear in the review
    let other_file = root.join("test.rs");
    let mut file = File::create(&other_file).unwrap();
    writeln!(file, "fn main() {{}}").unwrap();
    std::process::Command::new("git")
        .args(["add", "test.rs"])
        .current_dir(root)
        .output()
        .unwrap();

    // Run tool with --head - Cargo.lock should be ignored by default
    let mut cmd = code_reviewer_cmd();
    cmd.current_dir(root)
        .args(["git", "--head", "--no-copy-to-clipboard"])
        .assert()
        .success()
        .stdout(predicates::str::contains("test.rs")) // Should contain test.rs
        .stdout(predicates::str::contains("Cargo.lock").not()); // Should NOT contain Cargo.lock
}

#[test]
fn test_git_ignore_file_custom_pattern() {
    let temp = setup_git_repo();
    let root = temp.path();

    // Create files to test custom ignore
    let ignored_file = root.join("custom_ignore.txt");
    let mut file = File::create(&ignored_file).unwrap();
    writeln!(file, "should be ignored").unwrap();
    std::process::Command::new("git")
        .args(["add", "custom_ignore.txt"])
        .current_dir(root)
        .output()
        .unwrap();

    // Create another file that should appear
    let visible_file = root.join("visible.txt");
    let mut file = File::create(&visible_file).unwrap();
    writeln!(file, "should be visible").unwrap();
    std::process::Command::new("git")
        .args(["add", "visible.txt"])
        .current_dir(root)
        .output()
        .unwrap();

    // Run with custom ignore pattern
    let mut cmd = code_reviewer_cmd();
    cmd.current_dir(root)
        .args([
            "git",
            "--head",
            "--no-copy-to-clipboard",
            "--ignore-file",
            "custom_ignore.txt",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("visible.txt")) // Should contain visible.txt
        .stdout(predicates::str::contains("custom_ignore.txt").not()); // Should NOT contain custom_ignore.txt
}

#[test]
fn test_git_ignore_file_combined_patterns() {
    let temp = setup_git_repo();
    let root = temp.path();

    // Create files to test both default and custom ignore
    let cargo_lock = root.join("Cargo.lock"); // Should be ignored by default
    let mut file = File::create(&cargo_lock).unwrap();
    writeln!(file, "locked content").unwrap();
    std::process::Command::new("git")
        .args(["add", "Cargo.lock"])
        .current_dir(root)
        .output()
        .unwrap();

    let custom_ignored = root.join("custom.txt"); // Should be ignored by custom pattern
    let mut file = File::create(&custom_ignored).unwrap();
    writeln!(file, "custom content").unwrap();
    std::process::Command::new("git")
        .args(["add", "custom.txt"])
        .current_dir(root)
        .output()
        .unwrap();

    let visible_file = root.join("important.rs"); // Should be visible
    let mut file = File::create(&visible_file).unwrap();
    writeln!(file, "fn main() {{}}").unwrap();
    std::process::Command::new("git")
        .args(["add", "important.rs"])
        .current_dir(root)
        .output()
        .unwrap();

    // Run with custom ignore pattern - both Cargo.lock and custom.txt should be ignored
    let mut cmd = code_reviewer_cmd();
    cmd.current_dir(root)
        .args([
            "git",
            "--head",
            "--no-copy-to-clipboard",
            "--ignore-file",
            "custom.txt",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("important.rs")) // Should contain important.rs
        .stdout(predicates::str::contains("Cargo.lock").not()) // Should NOT contain Cargo.lock (default)
        .stdout(predicates::str::contains("custom.txt").not()); // Should NOT contain custom.txt (custom)
}
