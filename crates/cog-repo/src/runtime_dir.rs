//! The runtime-dir walk [`crate::RevokedKeys::default_path`] needs.
//!
//! `$WEFTOS_RUNTIME_DIR` wins and is not absolutized. Otherwise the nearest
//! project `.weftos/runtime` wins, then `<home>/.weftos/run` when that root
//! shows a daemon and `<home>/.clawft` does not, then `<home>/.clawft`.
//! Process-global user profile and child profile are not read: nothing in
//! this repository sets them. A test that would land on a real home root
//! panics unless the environment variable is set.

use std::path::{Path, PathBuf};

/// Environment variable that pins the runtime root.
pub const RUNTIME_DIR_ENV: &str = "WEFTOS_RUNTIME_DIR";

const SOCKET_NAME: &str = "kernel.sock";
const LOCK_FILE_NAME: &str = "kernel.lock";

/// `$HOME` or `%USERPROFILE%`. Empty values count as unset.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// `revoked_hosts.json` inside the resolved runtime root.
pub(crate) fn revoked_hosts_file() -> PathBuf {
    let env = std::env::var(RUNTIME_DIR_ENV).ok();
    let overridden = env.as_deref().is_some_and(|value| !value.trim().is_empty());
    let cwd = std::env::current_dir().ok();
    let home = home_dir();
    let root = resolve_root(env.as_deref(), cwd.as_deref(), home.as_deref());
    refuse_real_runtime_in_tests(&root, overridden);
    root.join("revoked_hosts.json")
}

/// Resolve a runtime root from explicit inputs. Empty `env` counts as unset.
pub(crate) fn resolve_root(env: Option<&str>, cwd: Option<&Path>, home: Option<&Path>) -> PathBuf {
    if let Some(dir) = env.map(str::trim).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(project) = cwd.and_then(|dir| find_project_dir(dir, home)) {
        return project.join(".weftos").join("runtime");
    }
    if let Some(user) = home.and_then(prefer_user_root) {
        return user;
    }
    home.map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir)
        .join(".clawft")
}

/// Panic when a test would read a real runtime root.
pub(crate) fn refuse_real_runtime_in_tests(root: &Path, overridden: bool) {
    if !cfg!(debug_assertions)
        || overridden
        || std::env::var_os("WEFTOS_ALLOW_REAL_HOME_IN_TESTS").is_some()
        || !is_test_process()
    {
        return;
    }
    let canon = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let tmp = std::env::temp_dir();
    if !(root.starts_with(&tmp) || canon(root).starts_with(canon(&tmp))) {
        panic!(
            "a test resolved the runtime root {} without a WEFTOS_RUNTIME_DIR override; \
             that is a real runtime dir. Set WEFTOS_RUNTIME_DIR, or point HOME at a tempdir. \
             WEFTOS_ALLOW_REAL_HOME_IN_TESTS=1 opts out",
            root.display()
        );
    }
}

fn is_test_process() -> bool {
    if cfg!(test) {
        return true;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    exe.parent().and_then(|path| path.file_name()).is_some_and(|name| name == "deps")
        || exe.components().any(|component| component.as_os_str().to_string_lossy().starts_with("rustdoctest"))
}

fn same_dir(left: &Path, right: &Path) -> bool {
    left == right
        || match (left.canonicalize(), right.canonicalize()) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
}

fn is_project_dir(dir: &Path) -> bool {
    let weftos = dir.join(".weftos");
    if !weftos.is_dir() {
        return false;
    }
    weftos.join("project.toml").is_file()
        || weftos.join("weave.toml").is_file()
        || dir.join("weave.toml").is_file()
        || weftos.join("runtime").is_dir()
        || dir.join(".git").exists()
}

fn find_project_dir(cwd: &Path, home: Option<&Path>) -> Option<PathBuf> {
    for dir in cwd.ancestors() {
        if home.is_some_and(|home| same_dir(dir, home)) {
            return None;
        }
        if is_project_dir(dir) {
            return Some(dir.to_path_buf());
        }
    }
    None
}

fn socket_answers(root: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::net::UnixStream::connect(root.join(SOCKET_NAME)).is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        false
    }
}

pub(crate) fn prefer_user_root(home: &Path) -> Option<PathBuf> {
    let user = home.join(".weftos").join("run");
    let legacy = home.join(".clawft");
    if socket_answers(&user) {
        return Some(user);
    }
    if socket_answers(&legacy) {
        return None;
    }
    let user_marked = user.join(SOCKET_NAME).exists() || user.join(LOCK_FILE_NAME).exists();
    (user_marked && !legacy.join(SOCKET_NAME).exists()).then_some(user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_wins_and_is_not_absolutized() {
        let root = resolve_root(Some("  /tmp/weft-cog-runtime  "), None, None);
        assert_eq!(root, PathBuf::from("/tmp/weft-cog-runtime"));
    }

    #[test]
    fn a_project_runtime_wins_over_home_and_home_itself_is_not_a_project() {
        let home = tempfile::tempdir().unwrap();
        let project = home.path().join("proj");
        std::fs::create_dir_all(project.join(".weftos")).unwrap();
        std::fs::write(project.join(".weftos").join("project.toml"), "").unwrap();
        let nested = project.join("crates/x");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(
            resolve_root(Some(" "), Some(&nested), Some(home.path())),
            project.join(".weftos").join("runtime")
        );
        std::fs::create_dir_all(home.path().join(".weftos")).unwrap();
        std::fs::write(home.path().join("weave.toml"), "").unwrap();
        assert_eq!(
            resolve_root(None, Some(home.path()), Some(home.path())),
            home.path().join(".clawft")
        );
    }

    #[test]
    fn a_marked_user_root_wins_when_legacy_has_no_socket() {
        let home = tempfile::tempdir().unwrap();
        assert!(prefer_user_root(home.path()).is_none());
        let user = home.path().join(".weftos").join("run");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join(LOCK_FILE_NAME), "").unwrap();
        assert_eq!(prefer_user_root(home.path()).unwrap(), user);
    }

    #[test]
    fn a_test_without_an_override_refuses_a_real_root() {
        let refused = std::panic::catch_unwind(|| refuse_real_runtime_in_tests(Path::new("/Users"), false));
        assert!(refused.is_err());
        refuse_real_runtime_in_tests(Path::new("/Users"), true);
        refuse_real_runtime_in_tests(&std::env::temp_dir().join("weft-cog-runtime-test"), false);
    }
}
