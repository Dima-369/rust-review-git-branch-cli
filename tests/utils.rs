use assert_cmd::Command;
use std::fs::File;
use std::io::Write;
use tempfile::TempDir;

/// Create a command for the code-reviewer binary using assert_cmd
pub fn code_reviewer_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_code-reviewer"))
}

/// Setup a temp dir with a basic git repo
pub fn setup_git_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Init git
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(root)
        .output()
        .expect("Failed to init git");

    // Config user (required for commits)
    std::process::Command::new("git")
        .args(["config", "--local", "user.email", "test@example.com"])
        .current_dir(root)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "--local", "user.name", "Test User"])
        .current_dir(root)
        .output()
        .unwrap();

    // Create initial commit
    let file_path = root.join("README.md");
    let mut file = File::create(file_path).unwrap();
    writeln!(file, "Initial content").unwrap();

    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(root)
        .output()
        .unwrap();

    std::process::Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(root)
        .output()
        .unwrap();

    temp
}

/// Create a git repository with a develop branch and changes for diff testing
#[allow(dead_code)]
pub fn setup_git_repo_with_changes() -> TempDir {
    let temp = setup_git_repo();
    let root = temp.path();

    // Create a develop branch to satisfy the smart branch detection
    let mut git_checkout_b = std::process::Command::new("git");
    git_checkout_b
        .args(["checkout", "-b", "develop"])
        .current_dir(root);
    let output = git_checkout_b
        .output()
        .expect("Failed to create develop branch");
    assert!(output.status.success());

    // Switch back to main
    let mut git_checkout_main = std::process::Command::new("git");
    let result = git_checkout_main
        .args(["checkout", "main"])
        .current_dir(root)
        .output();

    match result {
        Ok(output) => {
            if !output.status.success() {
                // On some systems the default branch might be called 'master'
                let mut git_checkout_master = std::process::Command::new("git");
                git_checkout_master
                    .args(["checkout", "master"])
                    .current_dir(root);
                let output = git_checkout_master
                    .output()
                    .expect("Failed to switch to master");
                assert!(output.status.success());
            }
        }
        Err(_) => {
            // On some systems the default branch might be called 'master'
            let mut git_checkout_master = std::process::Command::new("git");
            git_checkout_master
                .args(["checkout", "master"])
                .current_dir(root);
            let output = git_checkout_master
                .output()
                .expect("Failed to switch to master");
            assert!(output.status.success());
        }
    }

    // Modify the test file in main branch to have changes compared to develop
    std::fs::write(
        root.join("test_file.txt"),
        "line 1\nmodified line 2\nline 3\n",
    )
    .expect("Could not modify test file");

    // Add and commit the modified file to main branch
    let mut git_add_main = std::process::Command::new("git");
    git_add_main
        .args(["add", "test_file.txt"])
        .current_dir(root);
    let output = git_add_main.output().expect("Failed to add file to main");
    assert!(output.status.success());

    let mut git_commit_main = std::process::Command::new("git");
    git_commit_main
        .args(["commit", "-m", "Modify file in main"])
        .current_dir(root);
    let output = git_commit_main
        .output()
        .expect("Failed to commit file to main");
    assert!(output.status.success());

    temp
}

/// Setup a temp dir with a basic jj repo
#[allow(dead_code)]
pub fn setup_jj_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Check if jj is installed, skip if not
    if std::process::Command::new("jj")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("Skipping jj tests: jj binary not found");
        return temp;
    }

    // Init jj (git-backed is standard)
    std::process::Command::new("jj")
        .args(["git", "init"])
        .current_dir(root)
        .output()
        .expect("Failed to init jj");

    // Config user
    std::process::Command::new("jj")
        .args(["config", "set", "--repo", "user.email", "test@example.com"])
        .current_dir(root)
        .output()
        .unwrap();
    std::process::Command::new("jj")
        .args(["config", "set", "--repo", "user.name", "Test User"])
        .current_dir(root)
        .output()
        .unwrap();

    // Create a base commit (on main)
    let file_path = root.join("src/lib.rs");
    std::fs::create_dir_all(root.join("src")).unwrap();
    let mut file = File::create(&file_path).unwrap();
    writeln!(file, "fn main() {{}}").unwrap();

    std::process::Command::new("jj")
        .args(["describe", "-m", "initial"])
        .current_dir(root)
        .output()
        .unwrap();

    std::process::Command::new("jj")
        .args(["bookmark", "create", "-r", "@", "main"])
        .current_dir(root)
        .output()
        .unwrap();

    std::process::Command::new("jj")
        .args(["new", "main"])
        .current_dir(root)
        .output()
        .unwrap();

    temp
}
