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
    /// `true` when Steam positively reported the item as deleted. An item that
    /// is merely invisible to an unauthenticated request is not unavailable —
    /// SteamCMD may still fetch it with credentials.
    pub deleted: bool,
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

/// Write the manifest atomically. It is the only record of which directories
/// are managed, so a partial write must never replace a good one.
pub fn save(path: &Path, entries: &[ManifestEntry]) -> Result<(), ManifestError> {
    let contents = serde_json::to_string(entries).map_err(ManifestError::Serialize)?;
    let temporary = path.with_extension("json.tmp");
    let write = |source| ManifestError::Write {
        path: path.display().to_string(),
        source,
    };

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(write)?;
    }
    fs::write(&temporary, contents).map_err(write)?;
    fs::rename(&temporary, path).map_err(|source| {
        let _ = fs::remove_file(&temporary);
        write(source)
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
        // An entry with no remote record was not asked about this run; that is
        // "unknown", never "unavailable".
        let should_delete = (!requested && delete_unrequested)
            || (delete_unavailable && current.is_some_and(|item| item.deleted));
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
            deleted: false,
        }
    }

    fn entry(id: &str) -> ManifestEntry {
        ManifestEntry {
            downloaded_at: "today".into(),
            name: id.into(),
            mod_id: id.into(),
            file_size: 1,
            last_updated: Some(1),
            directory_name: id.into(),
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
        let existing = vec![entry("old"), entry("missing")];
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

    /// Regression for the `--delete-unavailable` data-loss bug: a mod that is
    /// simply absent from this run's request list has no remote record, and
    /// must never be mistaken for one Steam reported as deleted.
    #[test]
    fn delete_unavailable_spares_mods_that_were_merely_not_requested() {
        let existing = vec![entry("999")];
        let requested = vec!["111".to_string()];
        let remote = vec![remote("111", 1, 1)];

        let actions = plan(&requested, &remote, &existing, false, true);

        assert!(
            !actions
                .iter()
                .any(|action| action.action == Action::Delete && action.mod_id == "999"),
            "an unrequested mod must not be deleted by --delete-unavailable"
        );
    }

    #[test]
    fn delete_unavailable_removes_mods_steam_reports_as_deleted() {
        let existing = vec![entry("gone")];
        let requested = vec!["gone".to_string()];
        let remote = vec![RemoteMod {
            deleted: true,
            ..remote("gone", 1, 1)
        }];

        let actions = plan(&requested, &remote, &existing, false, true);

        assert!(actions.contains(&PlannedAction {
            action: Action::Delete,
            mod_id: "gone".into()
        }));
    }

    /// A login-gated item is invisible to the unauthenticated details request,
    /// but SteamCMD can still fetch it with credentials.
    #[test]
    fn delete_unavailable_spares_items_that_are_only_invisible() {
        let existing = vec![entry("private")];
        let requested = vec!["private".to_string()];
        let remote = vec![remote("private", 1, 1)];

        let actions = plan(&requested, &remote, &existing, false, true);

        assert!(!actions.iter().any(|action| action.action == Action::Delete));
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
