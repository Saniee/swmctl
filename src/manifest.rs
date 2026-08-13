use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestEntry {
    pub downloaded_at: String,
    pub name: String,
    pub mod_id: String,
    pub file_size: u64,
    pub last_updated: Option<i64>,
    #[serde(default)]
    pub directory_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteMod {
    pub name: String,
    pub mod_id: String,
    pub file_size: u64,
    pub last_updated: Option<i64>,
    pub available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Download,
    Update,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAction {
    pub action: Action,
    pub mod_id: String,
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("could not read manifest {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("could not parse manifest {path}: {source}")]
    Parse {
        path: String,
        source: serde_json::Error,
    },
    #[error("could not serialize manifest: {0}")]
    Serialize(serde_json::Error),
    #[error("could not write manifest {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
}

pub fn load(path: &Path) -> Result<Vec<ManifestEntry>, ManifestError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let contents = fs::read_to_string(path).map_err(|source| ManifestError::Read {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&contents).map_err(|source| ManifestError::Parse {
        path: path.display().to_string(),
        source,
    })
}

pub fn save(path: &Path, entries: &[ManifestEntry]) -> Result<(), ManifestError> {
    let contents = serde_json::to_string(entries).map_err(ManifestError::Serialize)?;
    fs::write(path, contents).map_err(|source| ManifestError::Write {
        path: path.display().to_string(),
        source,
    })
}

pub fn plan(
    requested_ids: &[String],
    remote: &[RemoteMod],
    existing: &[ManifestEntry],
    delete_unrequested: bool,
    delete_unavailable: bool,
) -> Vec<PlannedAction> {
    let mut actions = Vec::new();

    for item in remote {
        if !requested_ids.iter().any(|id| id == &item.mod_id) {
            continue;
        }

        match existing.iter().find(|entry| entry.mod_id == item.mod_id) {
            None => actions.push(PlannedAction {
                action: Action::Download,
                mod_id: item.mod_id.clone(),
            }),
            Some(previous) if item.last_updated > previous.last_updated => {
                actions.push(PlannedAction {
                    action: Action::Update,
                    mod_id: item.mod_id.clone(),
                });
            }
            Some(_) => {}
        }
    }

    for entry in existing {
        let requested = requested_ids.iter().any(|id| id == &entry.mod_id);
        let current = remote.iter().find(|item| item.mod_id == entry.mod_id);
        let should_delete = (!requested && delete_unrequested)
            || (delete_unavailable && current.is_some_and(|item| !item.available));
        if should_delete {
            actions.push(PlannedAction {
                action: Action::Delete,
                mod_id: entry.mod_id.clone(),
            });
        }
    }

    actions.sort_by_key(|action| {
        let priority = match action.action {
            Action::Download | Action::Update => 0,
            Action::Delete => 1,
        };
        let size = remote
            .iter()
            .find(|item| item.mod_id == action.mod_id)
            .map_or(0, |item| item.file_size);
        (priority, u64::MAX - size)
    });
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(id: &str, size: u64, updated: i64) -> RemoteMod {
        RemoteMod {
            name: id.into(),
            mod_id: id.into(),
            file_size: size,
            last_updated: Some(updated),
            available: true,
        }
    }

    #[test]
    fn plans_largest_download_first() {
        let requested = vec!["small".into(), "large".into()];
        let remote = vec![remote("small", 10, 1), remote("large", 20, 1)];
        let actions = plan(&requested, &remote, &[], true, false);

        assert_eq!(actions[0].mod_id, "large");
        assert_eq!(actions[1].mod_id, "small");
    }

    #[test]
    fn plans_update_and_configurable_delete() {
        let existing = vec![
            ManifestEntry {
                downloaded_at: "today".into(),
                name: "old".into(),
                mod_id: "old".into(),
                file_size: 1,
                last_updated: Some(1),
                directory_name: "old".into(),
            },
            ManifestEntry {
                downloaded_at: "today".into(),
                name: "missing".into(),
                mod_id: "missing".into(),
                file_size: 1,
                last_updated: Some(1),
                directory_name: "missing".into(),
            },
        ];
        let remote = vec![remote("old", 10, 2)];
        let requested = vec!["old".into()];
        let actions = plan(&requested, &remote, &existing, true, false);

        assert!(actions.contains(&PlannedAction {
            action: Action::Update,
            mod_id: "old".into()
        }));
        assert!(actions.contains(&PlannedAction {
            action: Action::Delete,
            mod_id: "missing".into()
        }));
    }

    #[test]
    fn saves_minified_json() {
        let path = std::env::temp_dir().join("swmctl-manifest-test.json");
        let entries = vec![ManifestEntry {
            downloaded_at: "today".into(),
            name: "Example".into(),
            mod_id: "123".into(),
            file_size: 42,
            last_updated: None,
            directory_name: "123".into(),
        }];

        save(&path, &entries).expect("manifest should save");
        let contents = fs::read_to_string(&path).expect("manifest should be readable");
        fs::remove_file(path).expect("test manifest should be removable");
        assert!(!contents.contains('\n'));
        assert_eq!(
            load(&std::env::temp_dir().join("missing-swmctl.json")).unwrap(),
            Vec::new()
        );
    }
}
