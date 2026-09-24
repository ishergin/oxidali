use std::path::{Path, PathBuf};
use std::process::Command;

const KNOB_PREFIX: &str = "DALI2RUST_";

const PRODUCTS_DB: &str = "dali-products.json.gz";
const LOCAL_ASSETS_DIR: &str = "assets/local";
const WEB_ASSETS_DIR: &str = "assets/web";

const IMAGE_INPUTS: &[&str] = &[
    "crates",
    "Cargo.toml",
    "Cargo.lock",
    ".cargo/config.toml",
    "sdkconfig.p4.defaults",
];

fn main() {
    for knob in scan_env_knobs(Path::new("src")) {
        println!("cargo:rerun-if-env-changed={knob}");
    }

    let manifest_dir: PathBuf = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    emit_rerun_triggers(&manifest_dir);
    println!("cargo:rustc-env=DALI2RUST_VERSION={}", version(&manifest_dir));
    println!("cargo:rustc-env=PRODUCTS_DB_GZ={}", products_db(&manifest_dir).display());

    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("espidf") {
        embuild::espidf::sysenv::output();
    }
}

fn products_db(manifest_dir: &Path) -> PathBuf {
    let local = manifest_dir.join(LOCAL_ASSETS_DIR).join(PRODUCTS_DB);
    if local.is_file() {
        local
    } else {
        manifest_dir.join(WEB_ASSETS_DIR).join(PRODUCTS_DB)
    }
}

fn version(dir: &Path) -> String {
    git_version(dir).unwrap_or_else(|| {
        let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
        format!("{base}+unknown")
    })
}

fn git_version(dir: &Path) -> Option<String> {
    let major = std::env::var("CARGO_PKG_VERSION_MAJOR").ok()?;
    let minor = std::env::var("CARGO_PKG_VERSION_MINOR").ok()?;
    let count = non_empty(git(dir, &["rev-list", "--count", "HEAD"])?)?;
    let sha = non_empty(git(dir, &["rev-parse", "--short=8", "HEAD"])?)?;
    Some(format!("{major}.{minor}.{count}+{sha}{}", dirty_suffix(dir)))
}

fn dirty_suffix(dir: &Path) -> String {
    match git(dir, &["status", "--porcelain", "--untracked-files=no"]) {
        Some(out) if out.is_empty() => String::new(),
        Some(_) => format!(".dirty{}", build_stamp()),
        None => ".unknown".to_string(),
    }
}

fn build_stamp() -> String {
    Command::new("date")
        .args(["-u", "+%m%dT%H%M"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| format!(".{}", s.trim()))
        .unwrap_or_default()
}

fn emit_rerun_triggers(manifest_dir: &Path) {
    println!("cargo:rerun-if-changed=src");
    for path in git_state_paths(manifest_dir) {
        rerun_if_exists(&path);
    }
    let Some(root) = git(manifest_dir, &["rev-parse", "--show-toplevel"]) else {
        return;
    };
    for input in IMAGE_INPUTS {
        rerun_if_exists(&Path::new(&root).join(input));
    }
}

fn git_state_paths(dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(head) = git(dir, &["rev-parse", "--git-path", "HEAD"]) {
        paths.push(PathBuf::from(head));
    }
    if let Some(refname) = git(dir, &["rev-parse", "--symbolic-full-name", "HEAD"]) {
        if refname.starts_with("refs/") {
            if let Some(p) = git(dir, &["rev-parse", "--git-path", &refname]) {
                paths.push(PathBuf::from(p));
            }
        }
    }
    if let Some(packed) = git(dir, &["rev-parse", "--git-path", "packed-refs"]) {
        paths.push(PathBuf::from(packed));
    }
    if let Some(reflog) = git(dir, &["rev-parse", "--git-path", "logs/HEAD"]) {
        paths.push(PathBuf::from(reflog));
    }
    paths
}

fn rerun_if_exists(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

fn scan_env_knobs(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    collect_from(dir, &mut found);
    found.sort();
    found.dedup();
    found
}

fn collect_from(dir: &Path, out: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_from(&path, out);
        } else if path.extension().map_or(false, |ext| ext == "rs") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                out.extend(knob_names_in(&text));
            }
        }
    }
}

fn knob_names_in(text: &str) -> Vec<String> {
    text.match_indices(KNOB_PREFIX)
        .map(|(at, _)| {
            text[at..]
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                .collect()
        })
        .collect()
}
