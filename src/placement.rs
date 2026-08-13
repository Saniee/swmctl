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
    #[error("could not remove existing mod directory {path}: {source}")]
    RemoveExisting { path: PathBuf, source: io::Error },
    #[error("could not place mod at {path}: {source}")]
    Place { path: PathBuf, source: io::Error },
}

pub fn place_mod(
    source: &Path,
    output: &Path,
    mod_id: &str,
    name: &str,
    mode: NameMode,
) -> Result<PathBuf, PlacementError> {
    if !source.is_dir() {
        return Err(PlacementError::MissingSource(source.to_path_buf()));
    }
    fs::create_dir_all(output).map_err(|source| PlacementError::CreateOutput {
        path: output.to_path_buf(),
        source,
    })?;

    let destination = output.join(directory_name(mode, mod_id, name));
    if destination.exists() {
        fs::remove_dir_all(&destination).map_err(|source| PlacementError::RemoveExisting {
            path: destination.clone(),
            source,
        })?;
    }
    fs::rename(source, &destination).map_err(|source| PlacementError::Place {
        path: destination.clone(),
        source,
    })?;
    Ok(destination)
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

        let destination = place_mod(&source, &output, "123", "Möd Name", NameMode::Name)
            .expect("mod should be placed");
        assert_eq!(destination.file_name().unwrap(), "mod_name");
        assert_eq!(directory_size(&destination).unwrap(), 4);

        fs::remove_dir_all(root).expect("test directory should be removable");
    }
}
