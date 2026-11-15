//! What this binary is: version and commit, recorded at compile time by `build.rs`.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
pub const LICENSE: &str = env!("CARGO_PKG_LICENSE");
/// Full commit hash; empty when built outside a git checkout.
pub const COMMIT: &str = env!("WALLE_GIT_HASH");
pub const COMMIT_SHORT: &str = env!("WALLE_GIT_SHORT");
/// Commit date as YYYY-MM-DD; empty when unknown.
pub const COMMIT_DATE: &str = env!("WALLE_GIT_DATE");
pub const COMPILER: &str = env!("WALLE_COMPILER");
pub const PROFILE: &str = env!("WALLE_PROFILE");
pub const TARGET: &str = env!("WALLE_TARGET");

/// Built from a working tree with uncommitted changes: the commit alone doesn't describe it.
pub fn dirty() -> bool {
    env!("WALLE_GIT_DIRTY") == "true"
}

/// The commit as shown to people, e.g. `effad86` or `effad86 + local changes`.
pub fn commit_label() -> String {
    if dirty() { format!("{COMMIT_SHORT} + local changes") } else { COMMIT_SHORT.to_string() }
}

/// Plain-text summary for bug reports.
pub fn details() -> String {
    let commit = if COMMIT.is_empty() { COMMIT_SHORT } else { COMMIT };
    let dirty = if dirty() { " (with local changes)" } else { "" };
    format!(
        "walle {VERSION}\ncommit: {commit}{dirty}\ncommit date: {COMMIT_DATE}\nbuild: {PROFILE}, {TARGET}\ncompiler: {COMPILER}\nos: {}",
        std::env::consts::OS
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_name_the_version_and_commit() {
        let d = details();
        assert!(d.contains(VERSION));
        assert!(d.contains(if COMMIT.is_empty() { COMMIT_SHORT } else { COMMIT }));
        assert!(!COMMIT_SHORT.is_empty());
    }
}
