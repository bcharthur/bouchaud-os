use std::{env, fs, path::PathBuf, process::Command};

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    Some(text.trim().to_string())
}

fn rust_string(s: &str) -> String {
    format!("{:?}", s)
}

fn main() {
    println!("cargo:rerun-if-changed=src/browser");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/FETCH_HEAD");

    let merge_hash = git(&["log", "--merges", "-1", "--format=%H", "--", "src/browser"])
        .filter(|s| !s.is_empty())
        .or_else(|| git(&["log", "-1", "--format=%H", "--", "src/browser"]))
        .unwrap_or_else(|| "unknown".to_string());
    let merge_short = merge_hash.chars().take(12).collect::<String>();

    let merge_subject = git(&["log", "-1", "--format=%s", &merge_hash])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "metadata unavailable".to_string());
    let merge_date = git(&["log", "-1", "--date=short", "--format=%cd", &merge_hash])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown-date".to_string());

    let source_hash = git(&["log", "-1", "--format=%H", "--", "src/browser"])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| merge_hash.clone());
    let source_short = source_hash.chars().take(12).collect::<String>();
    let source_date = git(&["log", "-1", "--date=short", "--format=%cd", &source_hash])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown-date".to_string());

    let generated = format!(
        "pub const NAUTILE_SOURCE_PATH: &str = \"src/browser\";\n\
         pub const NAUTILE_MERGE_COMMIT: &str = {};\n\
         pub const NAUTILE_MERGE_SHORT: &str = {};\n\
         pub const NAUTILE_MERGE_DATE: &str = {};\n\
         pub const NAUTILE_MERGE_SUBJECT: &str = {};\n\
         pub const NAUTILE_SOURCE_COMMIT: &str = {};\n\
         pub const NAUTILE_SOURCE_SHORT: &str = {};\n\
         pub const NAUTILE_SOURCE_DATE: &str = {};\n",
        rust_string(&merge_hash),
        rust_string(&merge_short),
        rust_string(&merge_date),
        rust_string(&merge_subject),
        rust_string(&source_hash),
        rust_string(&source_short),
        rust_string(&source_date),
    );

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    fs::write(out.join("nautile_version.rs"), generated).expect("write Nautile version metadata");
}
