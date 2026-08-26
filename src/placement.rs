use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::naming::{NameMode, directory_name};

#[derive(Debug, Error)]
pub enum PlacementError {
    #[error("downloaded mod source does not exist: {0}")]
    MissingSource(PathBuf),
    #[error("could not create output directory {path}: {source}")]
    CreateOutput { path: PathBuf, source: io::Error },
    #[error("could not stage mod at {path}: {source}")]
    Stage { path: PathBuf, source: io::Error },
    #[error("could not remove existing mod directory {path}: {source}")]
    RemoveExisting { path: PathBuf, source: io::Error },
    #[error("could not place mod at {path}: {source}")]
    Place { path: PathBuf, source: io::Error },
}

/// Move a freshly downloaded mod into the output directory.
///
/// The new copy is staged alongside its destination first, so an existing
/// install is only replaced once the replacement is known to be in place. A
/// failure at any point leaves the previous install intact.
pub fn place_mod(
    source: &Path,
    output: &Path,
    mod_id: &str,
    name: &str,
    mode: NameMode,
    prefix: &str,
) -> Result<PathBuf, PlacementError> {
    if !source.is_dir() {
        return Err(PlacementError::MissingSource(source.to_path_buf()));
    }
    fs::create_dir_all(output).map_err(|source| PlacementError::CreateOutput {
        path: output.to_path_buf(),
        source,
    })?;

    let destination = output.join(directory_name(mode, mod_id, name, prefix));
    let staged = sibling(&destination, ".swmctl-staging");
    let _ = fs::remove_dir_all(&staged);

    // Staging inside `output` means the swap below is always same-filesystem.
    move_dir(source, &staged).map_err(|error| PlacementError::Stage {
        path: staged.clone(),
        source: error,
    })?;

    if !destination.exists() {
        return fs::rename(&staged, &destination)
            .map(|()| destination.clone())
            .map_err(|error| {
                let _ = fs::remove_dir_all(&staged);
                PlacementError::Place {
                    path: destination,
                    source: error,
                }
            });
    }

    let previous = sibling(&destination, ".swmctl-previous");
    let _ = fs::remove_dir_all(&previous);
    fs::rename(&destination, &previous).map_err(|error| {
        let _ = fs::remove_dir_all(&staged);
        PlacementError::RemoveExisting {
            path: destination.clone(),
            source: error,
        }
    })?;

    match fs::rename(&staged, &destination) {
        Ok(()) => {
            let _ = fs::remove_dir_all(&previous);
            Ok(destination)
        }
        Err(error) => {
            // Put the user's original install back before reporting failure.
            let _ = fs::rename(&previous, &destination);
            let _ = fs::remove_dir_all(&staged);
            Err(PlacementError::Place {
                path: destination,
                source: error,
            })
        }
    }
}

fn sibling(destination: &Path, suffix: &str) -> PathBuf {
    let mut name = destination
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    destination.with_file_name(name)
}

/// Rename `source` to `destination`, falling back to a copy when the two are on
/// different filesystems (`--steamcmd-dir` on another mount is a normal setup,
/// and `rename` cannot cross mounts).
fn move_dir(source: &Path, destination: &Path) -> io::Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(rename_error) => match copy_dir(source, destination) {
            Ok(()) => {
                fs::remove_dir_all(source)?;
                Ok(())
            }
            Err(_) => {
                let _ = fs::remove_dir_all(destination);
                Err(rename_error)
            }
        },
    }
}

fn copy_dir(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

pub fn directory_size(path: &Path) -> io::Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total += directory_size(&entry.path())?;
        } else {
            total += metadata.len();
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("swmctl-placement-{suffix}"));
        fs::create_dir_all(&path).expect("test directory should be created");
        path
    }

    #[test]
    fn places_and_sizes_a_mod() {
        let root = temp_dir();
        let source = root.join("source");
        let output = root.join("mods");
        fs::create_dir_all(&source).expect("source should be created");
        fs::write(source.join("mod.txt"), b"1234").expect("file should be written");

        let destination = place_mod(&source, &output, "123", "Möd Name", NameMode::Name, "")
            .expect("mod should be placed");
        assert_eq!(destination.file_name().unwrap(), "mod_name");
        assert_eq!(directory_size(&destination).unwrap(), 4);

        fs::remove_dir_all(root).expect("test directory should be removable");
    }

    #[test]
    fn replaces_an_existing_install_and_leaves_no_scratch_directories() {
        let root = temp_dir();
        let source = root.join("source");
        let output = root.join("mods");
        fs::create_dir_all(&source).expect("source should be created");
        fs::write(source.join("mod.txt"), b"new").expect("file should be written");

        let existing = output.join("123");
        fs::create_dir_all(&existing).expect("existing install should be created");
        fs::write(existing.join("old.txt"), b"old").expect("file should be written");

        let destination = place_mod(&source, &output, "123", "Mod", NameMode::ModId, "")
            .expect("mod should be placed");
        assert_eq!(
            fs::read_to_string(destination.join("mod.txt")).unwrap(),
            "new"
        );
        assert!(!destination.join("old.txt").exists());

        let leftovers = fs::read_dir(&output)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(leftovers, vec!["123".to_string()]);

        fs::remove_dir_all(root).expect("test directory should be removable");
    }

    /// Regression: a failed placement must never leave the user without the
    /// install they already had. `output` nested inside `source` makes the
    /// final rename fail, standing in for a cross-filesystem failure.
    #[test]
    fn a_failed_placement_keeps_the_previous_install() {
        let root = temp_dir();
        let source = root.join("source");
        let output = source.join("out");
        fs::create_dir_all(&output).expect("output should be created");
        fs::write(source.join("new.txt"), b"new").expect("file should be written");

        let existing = output.join("123");
        fs::create_dir_all(&existing).expect("existing install should be created");
        fs::write(existing.join("important.pbo"), b"installed").expect("file should be written");

        let result = place_mod(&source, &output, "123", "Mod", NameMode::ModId, "");

        assert!(result.is_err(), "placement was expected to fail");
        assert_eq!(
            fs::read_to_string(existing.join("important.pbo")).unwrap(),
            "installed",
            "the previous install must survive a failed placement"
        );

        fs::remove_dir_all(root).expect("test directory should be removable");
    }

    #[test]
    fn copies_when_rename_is_unavailable() {
        let root = temp_dir();
        let source = root.join("source");
        let nested = source.join("inner");
        fs::create_dir_all(&nested).expect("source should be created");
        fs::write(source.join("a.txt"), b"aa").expect("file should be written");
        fs::write(nested.join("b.txt"), b"bbb").expect("file should be written");

        let destination = root.join("copied");
        copy_dir(&source, &destination).expect("copy should succeed");

        assert_eq!(fs::read_to_string(destination.join("a.txt")).unwrap(), "aa");
        assert_eq!(
            fs::read_to_string(destination.join("inner/b.txt")).unwrap(),
            "bbb"
        );

        fs::remove_dir_all(root).expect("test directory should be removable");
    }
}
