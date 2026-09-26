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
    /// Steam's content handle for the installed files, recorded from the Web
    /// API at download time. A different handle in a later run is definitive
    /// proof the remote files changed.
    #[serde(default)]
    pub content_handle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteMod {
    pub name: String,
    pub mod_id: String,
    pub file_size: u64,
    pub last_updated: Option<i64>,
    /// Steam's content handle for the item's current files, taking precedence
    /// over `last_updated` when both are known.
    pub content_handle: Option<String>,
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
            Some(previous) if update_needed(item, previous) => {
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

/// Whether a remote record claims the installed copy is out of date.
///
/// The Steam content handle is the primary signal: it changes exactly when
/// the item's files change, so two equal handles mean the files are the same
/// regardless of what the cached timestamps claim. Timestamps remain the
/// fallback for manifests that predate handle recording, and for items the
/// Web API cannot describe.
fn update_needed(remote: &RemoteMod, previous: &ManifestEntry) -> bool {
    // A record claiming the item is newer than Steam says the item ever was
    // is not a real remote timestamp: it is the local clock, written by an
    // old API-less run that used SteamCMD's cache as if it were remote data.
    // Such an entry cannot be trusted as current and needs one refresh.
    let time_impossible = remote
        .last_updated
        .is_some_and(|remote_time| previous.last_updated > Some(remote_time));

    match &remote.content_handle {
        // The handle is authoritative when both sides know it: files changed
        // iff the handle changed. A description-only edit moves the remote
        // timestamp without touching the handle, and must not trigger a
        // pointless re-download.
        Some(handle) => match &previous.content_handle {
            Some(previous_handle) => previous_handle != handle || time_impossible,
            // The manifest predates handle recording. Keep the timestamp rule
            // until a handle is recorded, so arming handles never forces an
            // immediate re-download of every mod.
            None => time_impossible || remote.last_updated > previous.last_updated,
        },
        // No handle this run (item invisible to the API, or the API was
        // unreachable): timestamps are the only signal left, and an
        // impossible one still forces a refresh.
        None => time_impossible || remote.last_updated > previous.last_updated,
    }
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
            content_handle: None,
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
            content_handle: None,
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

    /// A changed Steam content handle is definitive proof the files changed,
    /// even when the cached timestamps did not move.
    #[test]
    fn plans_update_when_the_content_handle_changes() {
        let existing = vec![ManifestEntry {
            content_handle: Some("HANDLE-1".into()),
            ..entry("old")
        }];
        let remote_items = vec![RemoteMod {
            content_handle: Some("HANDLE-2".into()),
            ..remote("old", 10, 5)
        }];
        let actions = plan(&["old".into()], &remote_items, &existing, false, false);

        assert!(actions.contains(&PlannedAction {
            action: Action::Update,
            mod_id: "old".into()
        }));
    }

    /// Equal handles mean identical files: a newer timestamp without a handle
    /// change is a description or tag edit, which needs no re-download.
    #[test]
    fn equal_handles_suppress_updates_even_with_newer_timestamps() {
        let existing = vec![ManifestEntry {
            content_handle: Some("HANDLE-1".into()),
            last_updated: Some(5),
            ..entry("old")
        }];
        let remote_items = vec![RemoteMod {
            content_handle: Some("HANDLE-1".into()),
            last_updated: Some(9),
            ..remote("old", 10, 5)
        }];
        let actions = plan(&["old".into()], &remote_items, &existing, false, false);

        assert!(!actions.iter().any(|action| action.mod_id == "old"));
    }

    /// Manifests written before handles were recorded keep the timestamp rule
    /// until a handle is recorded, so arming handles never re-downloads every
    /// mod at once.
    #[test]
    fn legacy_entries_without_a_handle_keep_the_timestamp_rule() {
        let existing = vec![ManifestEntry {
            last_updated: Some(5), // same as the remote's timestamp
            ..entry("old")
        }];
        let remote_items = vec![RemoteMod {
            content_handle: Some("HANDLE-1".into()),
            last_updated: Some(5),
            ..remote("old", 10, 5)
        }];
        // Same timestamp, fresh handle: no mass refresh.
        assert!(plan(&["old".into()], &remote_items, &existing, false, false).is_empty());
        // A newer timestamp still updates.
        let newer = vec![RemoteMod {
            last_updated: Some(6),
            ..remote("old", 10, 5)
        }];
        assert!(
            plan(&["old".into()], &newer, &existing, false, false).contains(&PlannedAction {
                action: Action::Update,
                mod_id: "old".into()
            })
        );
    }

    /// A recorded timestamp newer than the item's real update history cannot
    /// come from Steam: it is a local clock written by an old API-less run,
    /// and the entry must refresh once instead of being trusted as current.
    #[test]
    fn refreshes_entries_whose_timestamp_is_impossible() {
        let existing = vec![ManifestEntry {
            last_updated: Some(10),
            ..entry("old")
        }];
        let remote = vec![remote("old", 10, 5)]; // Steam says the item is older

        let actions = plan(&["old".into()], &remote, &existing, false, false);
        assert!(actions.contains(&PlannedAction {
            action: Action::Update,
            mod_id: "old".into()
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
            content_handle: None,
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
