use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use chrono::Utc;

fn main() {
    // Define the output path for the version.rs file
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = PathBuf::from(out_dir).join("version.rs");

    // Helper function to execute a command and return its output as a String
    fn run_command(args: &[&str]) -> String {
        let output = Command::new(args[0])
            .args(&args[1..])
            .output()
            .expect("Failed to execute command");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    // Execute Git commands
    let git_repo = run_command(&["git", "remote", "get-url", "origin"]);
    let git_hash = run_command(&["git", "log", "-1", "--pretty=format:%H"]);
    let git_author = run_command(&["git", "log", "-1", "--pretty=format:%an <%ae>"]);
    let git_branch = run_command(&["git", "branch", "--show-current"]);
    let git_tag = run_command(&["git", "describe", "--tags", "--abbrev=1"]);
    let git_commit_date = run_command(&["git", "log", "-1", "--pretty=format:%as"]);
    let git_is_dirty = run_command(&["git", "diff", "--shortstat"]);
    let build_date = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // Determine if the repository is dirty
    let git_is_dirty = if git_is_dirty.is_empty() {
        "clean"
    } else {
        "dirty"
    };

    // Write the version information to the version.rs file
    fs::write(
        &dest_path,
        format!(
            r#"
#[allow(dead_code)]
pub const GIT_REPO: &str = "{}";
#[allow(dead_code)]
pub const GIT_HASH: &str = "{}";
#[allow(dead_code)]
pub const GIT_AUTHOR: &str = "{}";
#[allow(dead_code)]
pub const GIT_BRANCH: &str = "{}";
#[allow(dead_code)]
pub const GIT_TAG: &str = "{}";
#[allow(dead_code)]
pub const GIT_COMMIT_DATE: &str = "{}";
#[allow(dead_code)]
pub const GIT_IS_DIRTY: &str = "{}";
#[allow(dead_code)]
pub const BUILD_DATE: &str = "{}";
"#,
            git_repo,
            git_hash,
            git_author,
            git_branch,
            git_tag,
            git_commit_date,
            git_is_dirty,
            build_date
        ),
    )
    .unwrap();
}
