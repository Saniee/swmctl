use std::io;
use std::path::{Component, Path, PathBuf};

/// Resolve `path` against the current directory and remove `.` / `..`
/// components without touching the filesystem.
///
/// SteamCMD resolves a relative `+force_install_dir` against its own
/// installation directory, not the caller's working directory, so a relative
/// staging path lands somewhere swmctl never looks and every item is reported
/// as missing. Passing an absolute path removes the ambiguity.
///
/// `std::fs::canonicalize` is deliberately avoided: it requires the directory
/// to exist already and, on Windows, returns a `\\?\` verbatim path that
/// SteamCMD rejects.
pub fn absolute(path: &Path) -> io::Result<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(lexically_normalize(&joined))
}

fn lexically_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match normalized.components().next_back() {
                Some(Component::Normal(_)) => {
                    normalized.pop();
                }
                // `/..` is `/`; there is nothing above a root or a drive.
                Some(Component::RootDir | Component::Prefix(_)) => {}
                // Keep `..` in a still-relative path, so a fragment that
                // escapes its base survives intact.
                _ => normalized.push(component),
            },
            other => normalized.push(other.as_os_str()),
        }
    }
    if normalized.as_os_str().is_empty() {
        normalized.push(Component::CurDir);
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_absolute_paths_alone() {
        let path = if cfg!(windows) {
            Path::new(r"C:\steamcmd\cache")
        } else {
            Path::new("/steamcmd/cache")
        };
        assert_eq!(absolute(path).unwrap(), path);
    }

    #[test]
    fn resolves_relative_paths_against_the_working_directory() {
        let expected = std::env::current_dir().unwrap().join(".swmctl-steamcmd");
        assert_eq!(absolute(Path::new(".swmctl-steamcmd")).unwrap(), expected);
        assert_eq!(absolute(Path::new("./.swmctl-steamcmd")).unwrap(), expected);
    }

    #[test]
    fn removes_current_and_parent_directory_components() {
        assert_eq!(
            lexically_normalize(Path::new("/tmp/./cache/../staging")),
            Path::new("/tmp/staging")
        );
    }

    #[test]
    fn keeps_parent_components_that_cannot_be_resolved() {
        assert_eq!(
            lexically_normalize(Path::new("../cache")),
            Path::new("../cache")
        );
        assert_eq!(
            lexically_normalize(Path::new("/../cache")),
            Path::new("/cache")
        );
    }

    #[test]
    fn an_empty_path_normalizes_to_the_current_directory() {
        assert_eq!(lexically_normalize(Path::new("")), Path::new("."));
    }
}
