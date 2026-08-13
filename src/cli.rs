use std::fs;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::auth::{CredentialOverrides, config_path, load_config, resolve_credentials};
use crate::manifest::{
    Action, ManifestEntry, PlannedAction, RemoteMod, load as load_manifest, plan,
    save as save_manifest,
};
use crate::metadata::read_acf;
use crate::naming::NameMode;
use crate::placement::{directory_size, place_mod};
use crate::steam_api::{fetch_collection_items, fetch_published_files};
use crate::steamcmd::{SteamCmd, workshop_content_path, workshop_download_path};

#[derive(Debug, Parser)]
#[command(name = "swmctl", version, about = "Synchronize Steam Workshop mods")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Sync(SyncArgs),
}

#[derive(Debug, Args)]
pub struct SyncArgs {
    /// Steam application ID that owns the Workshop items.
    #[arg(long)]
    pub app_id: u32,
    /// Workshop item IDs to download.
    #[arg(required = false)]
    pub mod_ids: Vec<String>,
    /// File containing one Workshop item ID per line. Blank lines and # comments are ignored.
    #[arg(long)]
    pub mod_list: Option<PathBuf>,
    /// Workshop collection ID or collection URL.
    #[arg(long)]
    pub collection: Option<String>,
    /// Path to the SteamCMD executable.
    #[arg(long, default_value = "steamcmd")]
    pub steamcmd: PathBuf,
    /// Directory used by SteamCMD for its temporary Workshop download.
    #[arg(long, default_value = ".swmctl-steamcmd")]
    pub steamcmd_dir: PathBuf,
    /// Directory where managed mods are placed.
    #[arg(long, default_value = ".")]
    pub output: PathBuf,
    /// JSON manifest path.
    #[arg(long)]
    pub manifest: Option<PathBuf>,
    /// Folder naming mode: `id` or `name`.
    #[arg(long, value_parser = parse_name_mode, default_value = "id")]
    pub name_mode: NameMode,
    /// Delete managed mods that are not in the requested ID list.
    #[arg(long)]
    pub delete_unrequested: bool,
    /// Delete managed mods no longer present in Workshop metadata.
    #[arg(long)]
    pub delete_unavailable: bool,
    /// Steam account username.
    #[arg(long, env = "SWMCTL_STEAM_USERNAME")]
    pub username: Option<String>,
    /// Steam account password. Avoid shell history when possible.
    #[arg(long, env = "SWMCTL_STEAM_PASSWORD", hide_env_values = true)]
    pub password: Option<String>,
}

pub fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Commands::Sync(args) => run_sync(args),
    }
}

fn run_sync(args: SyncArgs) -> Result<(), Box<dyn std::error::Error>> {
    let mod_ids = resolve_mod_ids(&args)?;
    if mod_ids.is_empty() {
        return Err("provide mod IDs, --mod-list, or --collection".into());
    }

    let manifest_path = args
        .manifest
        .clone()
        .unwrap_or_else(|| args.output.join("swmctl-manifest.json"));
    let mut entries = load_manifest(&manifest_path)?;
    let metadata_path = args
        .steamcmd_dir
        .join("steamapps/workshop")
        .join(format!("appworkshop_{}.acf", args.app_id));
    let metadata = read_acf(&metadata_path)?;
    let published_files = match fetch_published_files(&mod_ids) {
        Ok(files) => Some(files),
        Err(error) => {
            eprintln!("warning: could not fetch Workshop titles and versions: {error}");
            None
        }
    };
    let remote = build_remote_metadata(&mod_ids, &metadata, published_files.as_ref());
    let actions = if published_files.is_some() {
        plan(
            &mod_ids,
            &remote,
            &entries,
            args.delete_unrequested,
            args.delete_unavailable,
        )
    } else {
        mod_ids
            .iter()
            .map(|mod_id| PlannedAction {
                action: Action::Download,
                mod_id: mod_id.clone(),
            })
            .collect()
    };
    let pending_ids = actions
        .iter()
        .filter(|action| matches!(action.action, Action::Download | Action::Update))
        .map(|action| action.mod_id.clone())
        .collect::<Vec<_>>();

    let config = config_path()
        .map(|path| load_config(&path))
        .transpose()?
        .unwrap_or_default();
    let credentials = resolve_credentials(
        &CredentialOverrides {
            username: args.username,
            password: args.password,
        },
        &config,
    )?;
    if !pending_ids.is_empty() {
        SteamCmd::new(args.steamcmd, credentials).workshop_download(
            args.app_id,
            &pending_ids,
            &args.steamcmd_dir,
        )?;
    }

    for mod_id in pending_ids {
        let source = workshop_content_path(&args.steamcmd_dir, args.app_id, &mod_id);
        if !source.is_dir() && workshop_download_path(&args.steamcmd_dir, &mod_id).exists() {
            return Err(format!(
                "Workshop item {mod_id} is still downloading in {}",
                workshop_download_path(&args.steamcmd_dir, &mod_id).display()
            )
            .into());
        }
        let title = remote
            .iter()
            .find(|item| item.mod_id == mod_id)
            .map_or(mod_id.as_str(), |item| item.name.as_str());
        let destination = place_mod(&source, &args.output, &mod_id, title, args.name_mode)?;
        let size = directory_size(&destination)?;
        let directory_name = destination.file_name().map_or_else(
            || mod_id.clone(),
            |name| name.to_string_lossy().into_owned(),
        );
        entries.retain(|entry| entry.mod_id != mod_id);
        let acf_item = metadata.iter().find(|item| item.mod_id == mod_id);
        let last_updated = remote
            .iter()
            .find(|item| item.mod_id == mod_id)
            .and_then(|item| item.last_updated)
            .or_else(|| acf_item.and_then(|item| item.last_updated));
        entries.push(ManifestEntry {
            downloaded_at: chrono::Utc::now().to_rfc3339(),
            name: title.to_string(),
            mod_id,
            file_size: size,
            last_updated,
            directory_name,
        });
    }

    if args.delete_unrequested || (args.delete_unavailable && published_files.is_some()) {
        let mut retained = Vec::with_capacity(entries.len());
        for entry in entries {
            let requested = mod_ids.iter().any(|mod_id| mod_id == &entry.mod_id);
            let available = remote
                .iter()
                .find(|item| item.mod_id == entry.mod_id)
                .is_some_and(|item| item.available);
            let should_delete = (!requested && args.delete_unrequested)
                || (!available && args.delete_unavailable && published_files.is_some());
            if should_delete {
                let directory_name = if entry.directory_name.is_empty() {
                    &entry.name
                } else {
                    &entry.directory_name
                };
                let path = args.output.join(directory_name);
                if path.exists() {
                    fs::remove_dir_all(path)?;
                }
            } else {
                retained.push(entry);
            }
        }
        entries = retained;
    }
    save_manifest(&manifest_path, &entries)?;
    Ok(())
}

fn build_remote_metadata(
    mod_ids: &[String],
    acf: &[crate::metadata::WorkshopMetadata],
    published_files: Option<&std::collections::HashMap<String, crate::steam_api::PublishedFile>>,
) -> Vec<RemoteMod> {
    mod_ids
        .iter()
        .map(|mod_id| {
            let api_item = published_files.and_then(|files| files.get(mod_id));
            let acf_item = acf.iter().find(|item| item.mod_id == *mod_id);
            RemoteMod {
                name: api_item
                    .filter(|item| !item.title.is_empty())
                    .map_or_else(|| mod_id.clone(), |item| item.title.clone()),
                mod_id: mod_id.clone(),
                file_size: api_item
                    .and_then(|item| item.file_size)
                    .or_else(|| acf_item.map(|item| item.file_size))
                    .unwrap_or_default(),
                last_updated: api_item
                    .and_then(|item| item.time_updated)
                    .or_else(|| acf_item.and_then(|item| item.last_updated)),
                available: published_files.is_none_or(|files| files.contains_key(mod_id)),
            }
        })
        .collect()
}

fn parse_name_mode(value: &str) -> Result<NameMode, String> {
    match value {
        "id" => Ok(NameMode::ModId),
        "name" => Ok(NameMode::Name),
        _ => Err("name mode must be `id` or `name`".into()),
    }
}

fn resolve_mod_ids(args: &SyncArgs) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut ids = args.mod_ids.clone();
    if let Some(path) = &args.mod_list {
        let contents = fs::read_to_string(path)?;
        for (line_number, line) in contents.lines().enumerate() {
            if let Some(id) = parse_mod_id_line(line).map_err(|error| {
                format!(
                    "invalid mod list entry at {}:{}: {error}",
                    path.display(),
                    line_number + 1
                )
            })? {
                ids.push(id);
            }
        }
    }
    if let Some(collection) = &args.collection {
        ids.extend(fetch_collection_items(collection)?);
    }

    let mut unique = Vec::new();
    for id in ids {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    Ok(unique)
}

fn parse_mod_id_line(line: &str) -> Result<Option<String>, &'static str> {
    let value = line.split('#').next().unwrap_or("").trim();
    if value.is_empty() {
        Ok(None)
    } else if value.chars().all(|character| character.is_ascii_digit()) {
        Ok(Some(value.to_string()))
    } else {
        Err("expected a numeric Workshop ID")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mod_list_entries_and_comments() {
        assert_eq!(
            parse_mod_id_line("123 # server mod"),
            Ok(Some("123".into()))
        );
        assert_eq!(parse_mod_id_line("  # comment"), Ok(None));
        assert_eq!(parse_mod_id_line("   "), Ok(None));
    }

    #[test]
    fn rejects_non_numeric_mod_list_entries() {
        assert_eq!(
            parse_mod_id_line("not-a-workshop-id"),
            Err("expected a numeric Workshop ID")
        );
    }
}
