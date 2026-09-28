use std::path::Path;
use std::process::Command;

const STAMP_INPUTS: &[&str] = &[
    "src",
    "build.rs",
    "Cargo.toml",
    ".cargo/config.toml",
    "sdkconfig.gear-sim.defaults",
];

fn main() {
    embuild::espidf::sysenv::output();
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let dir = Path::new(&dir);
    println!("cargo:rustc-env=DALI_GEAR_SIM_BUILD={}", build_id(dir));
    for input in STAMP_INPUTS {
        println!("cargo:rerun-if-changed={input}");
    }
    if let Some(reflog) = git(dir, &["rev-parse", "--git-path", "logs/HEAD"]) {
        println!("cargo:rerun-if-changed={reflog}");
    }
}

fn build_id(dir: &Path) -> String {
    let Some(sha) = git(dir, &["rev-parse", "--short=8", "HEAD"]) else {
        return "unknown".into();
    };
    match git(dir, &["status", "--porcelain", "--untracked-files=no"]) {
        Some(out) if out.is_empty() => sha,
        _ => format!("{sha}.dirty"),
    }
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
