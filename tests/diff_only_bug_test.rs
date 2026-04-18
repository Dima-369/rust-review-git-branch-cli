mod utils;

use utils::{code_reviewer_cmd, setup_git_repo_with_changes};

#[test]
fn test_diff_only_flag_not_working() {
    // Create a temporary git repository with changes for testing
    let temp_dir = setup_git_repo_with_changes();
    let repo_path = temp_dir.path();

    // Change to the repo directory and run the code-reviewer command with --diff-only
    let mut cmd = code_reviewer_cmd();
    cmd.current_dir(repo_path).args([
        "git",
        "--diff-only",
        "--target",
        "develop",
        "--no-copy-to-clipboard",
    ]); // Explicitly specify develop branch to compare against

    let output = cmd.output().expect("Failed to execute code-reviewer");

    eprintln!("Command status: {:?}", output.status);
    eprintln!(
        "Command stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    eprintln!(
        "Command stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The command should succeed, but if it doesn't, print the error and fail
    assert!(
        output.status.success(),
        "Command failed with status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("Could not decode output as UTF-8");

    // The bug is that the --diff-only flag doesn't work properly.
    // According to the help, --diff-only should show only diff, not full file content.
    // However, the flag is currently not implemented in the code.
    // The expected behavior is that with --diff-only, we should NOT see "Full File Contents" section.
    // But since the flag is not implemented, it still shows the full file contents.
    // So the bug is that --diff-only flag is ignored and full content is still included.

    // Run the same command without --diff-only to compare outputs
    let mut cmd_full = code_reviewer_cmd();
    cmd_full
        .current_dir(repo_path)
        .args(["git", "--target", "develop", "--no-copy-to-clipboard"]); // Compare without the --diff-only flag

    let output_full = cmd_full
        .output()
        .expect("Failed to execute code-reviewer without --diff-only");
    assert!(
        output_full.status.success(),
        "Command without --diff-only failed with status: {:?}, stderr: {}",
        output_full.status,
        String::from_utf8_lossy(&output_full.stderr)
    );

    let stdout_full =
        String::from_utf8(output_full.stdout).expect("Could not decode output as UTF-8");

    // The --diff-only flag should show only diffs, not full file contents
    // According to the help: "Show only diff, not full file content"
    // So with --diff-only, we should see "## Detailed File Diffs" but NOT "## Full File Contents"

    let with_diff_only_has_full_contents = stdout.contains("## Full File Contents");
    let with_diff_only_has_diffs = stdout.contains("## Detailed File Diffs");

    let without_diff_only_has_full_contents = stdout_full.contains("## Full File Contents");
    let without_diff_only_has_diffs = stdout_full.contains("## Detailed File Diffs");

    // The bug is that --diff-only doesn't work as expected
    // Expected behavior: --diff-only shows diffs but not full contents
    // Actual behavior (the bug): --diff-only might show neither diffs nor full contents, OR it might show both

    // If --diff-only shows full contents, that's definitely wrong
    if with_diff_only_has_full_contents {
        panic!(
            "Bug confirmed: --diff-only flag is not working. Full file contents are present when they shouldn't be.\nWith --diff-only: {stdout}"
        );
    }

    // If --diff-only doesn't show diffs when regular command does, that's also wrong
    if !with_diff_only_has_diffs && without_diff_only_has_diffs {
        panic!(
            "Bug confirmed: --diff-only flag is not working. Diffs are missing when they should be present.\nWith --diff-only: {stdout}\nWithout --diff-only: {stdout_full}"
        );
    }

    // If both show the same thing (neither diffs nor full contents), that's also wrong
    if (!with_diff_only_has_diffs && !with_diff_only_has_full_contents)
        && (!without_diff_only_has_diffs && !without_diff_only_has_full_contents)
    {
        panic!(
            "Bug confirmed: --diff-only flag is not working. Both commands produce minimal output when --diff-only should show diffs.\nWith --diff-only: {stdout}\nWithout --diff-only: {stdout_full}"
        );
    }

    // The correct behavior should be:
    // --diff-only: has diffs, no full contents
    // regular: has diffs, has full contents
    if !with_diff_only_has_diffs || with_diff_only_has_full_contents {
        panic!(
            "Bug confirmed: --diff-only flag is not working as expected.\nWith --diff-only: Should have diffs but no full contents\nActual: Has diffs: {with_diff_only_has_diffs}, Has full contents: {with_diff_only_has_full_contents}\nOutput: {stdout}"
        );
    }
}
