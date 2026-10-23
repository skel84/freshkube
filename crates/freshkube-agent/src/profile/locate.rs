//! Where a GUI app finds the agent's binary. A GUI app doesn't inherit a
//! shell's `PATH`, so the usual install directories are tried before it.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[cfg(windows)]
const FILE: &str = "opencode.exe";
#[cfg(not(windows))]
const FILE: &str = "opencode";

/// The directories to look in, in order: the usual install directories
/// (Homebrew's, `/usr/local/bin`, the installer's `~/.opencode/bin`,
/// `~/.local/bin`), then every directory of `PATH`.
pub fn search_dirs(home: Option<&Path>, path: Option<&OsStr>) -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    if let Some(home) = home {
        dirs.push(home.join(".opencode").join("bin"));
        dirs.push(home.join(".local").join("bin"));
    }
    if let Some(path) = path {
        dirs.extend(std::env::split_paths(path).filter(|dir| dir.is_absolute()));
    }
    dirs
}

/// The binary to run: `chosen`, a file the person picked, if it is one;
/// otherwise the first `opencode` in `dirs`.
pub fn locate(chosen: Option<&Path>, dirs: &[PathBuf]) -> Option<PathBuf> {
    if let Some(chosen) = chosen.filter(|path| runnable(path)) {
        return Some(chosen.to_path_buf());
    }
    dirs.iter().map(|dir| dir.join(FILE)).find(|p| runnable(p))
}

fn runnable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn install(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let file = dir.join(FILE);
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        file
    }

    #[test]
    fn opencode_is_found_where_a_gui_app_can_find_it() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let path_a = root.path().join("path-a");
        let path_b = root.path().join("path-b");
        let path_var = std::env::join_paths([&path_a, &path_b]).unwrap();
        let dirs = search_dirs(Some(&home), Some(&path_var));
        // The fixed directories come first, then the home ones, then PATH in order.
        assert_eq!(dirs[0], Path::new("/opt/homebrew/bin"));
        assert_eq!(dirs[1], Path::new("/usr/local/bin"));
        assert_eq!(dirs[2], home.join(".opencode/bin"));
        assert_eq!(dirs[3], home.join(".local/bin"));
        assert_eq!(&dirs[4..], &[path_a.clone(), path_b.clone()]);

        // Only the search order matters below, so use temporary directories
        // for every slot.
        let slots: Vec<PathBuf> = (0..5).map(|i| root.path().join(format!("d{i}"))).collect();
        let chosen = install(&root.path().join("chosen"));
        let others: Vec<PathBuf> = slots.iter().map(|d| install(d)).collect();
        assert_eq!(locate(Some(&chosen), &slots), Some(chosen.clone()));
        assert_eq!(locate(None, &slots), Some(others[0].clone()));
        std::fs::remove_file(&others[0]).unwrap();
        assert_eq!(locate(None, &slots), Some(others[1].clone()));
        for other in &others[1..4] {
            std::fs::remove_file(other).unwrap();
        }
        assert_eq!(locate(None, &slots), Some(others[4].clone()));
        std::fs::remove_file(&others[4]).unwrap();
        assert_eq!(locate(None, &slots), None);
    }

    #[test]
    fn a_chosen_file_that_is_not_runnable_falls_back_to_the_search() {
        let root = tempfile::tempdir().unwrap();
        let found = install(&root.path().join("bin"));
        let not_executable = root.path().join("plain");
        std::fs::write(&not_executable, "x").unwrap();
        let missing = root.path().join("missing");
        let dirs = [root.path().join("bin")];
        assert_eq!(locate(Some(&not_executable), &dirs), Some(found.clone()));
        assert_eq!(locate(Some(&missing), &dirs), Some(found.clone()));
        assert_eq!(locate(Some(root.path()), &dirs), Some(found));
    }
}
