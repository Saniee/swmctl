use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};

use crate::auth::{CredentialOverrides, config_path, load_config, resolve_credentials};
use crate::manifest::{
    Action, ManifestEntry, RemoteMod, load as load_manifest, plan, save as save_manifest,
};
use crate::metadata::read_acf;
use crate::naming::{NameMode, directory_name};
use crate::paths::absolute;
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

#[derive(Args)]
pub struct SyncArgs {
    /// Steam application ID that owns the Workshop items.
    #[arg(long)]
    pub app_id: u32,
    /// Workshop item IDs to download.
    #[arg(required = false, value_parser = parse_mod_id)]
    pub mod_ids: Vec<String>,
    /// File containing one Workshop item ID per line, optionally followed by a
    /// name. Blank lines and # comments are ignored.
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
    /// String prefixed to every mod folder name. Arma servers expect `@`.
    #[arg(long, default_value = "")]
    pub name_prefix: String,
    /// Attempts per run. SteamCMD frequently drops items part-way; only the
    /// items still missing are retried.
    #[arg(long, default_value_t = 3, value_name = "N")]
    pub max_retries: u32,
    /// Seconds to wait between attempts.
    #[arg(long, default_value_t = 30, value_name = "SECONDS")]
    pub retry_delay: u64,
    /// Workshop items handed to a single SteamCMD invocation. A SteamCMD
    /// timeout ends the whole process and discards every item still in flight,
    /// so items are downloaded one at a time by default. Raise it to trade
    /// timeout isolation for fewer SteamCMD startups.
    #[arg(long, default_value_t = 1, value_name = "N")]
    pub batch_size: usize,
    /// Report the planned actions without downloading, moving, or deleting.
    #[arg(long)]
    pub dry_run: bool,
    /// Suppress progress output. Errors and warnings are still reported.
    #[arg(long, short)]
    pub quiet: bool,
    /// Delete managed mods that are not in the requested ID list.
    #[arg(long)]
    pub delete_unrequested: bool,
    /// Delete managed mods that Steam reports as deleted.
    #[arg(long)]
    pub delete_unavailable: bool,
    /// Steam account username.
    #[arg(long, env = "SWMCTL_STEAM_USERNAME")]
    pub username: Option<String>,
    /// Steam account password. Avoid shell history when possible.
    #[arg(long, env = "SWMCTL_STEAM_PASSWORD", hide_env_values = true)]
    pub password: Option<String>,
}

/// Hand-written so the password never reaches a debug print or panic payload.
impl fmt::Debug for SyncArgs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SyncArgs")
            .field("app_id", &self.app_id)
            .field("mod_ids", &self.mod_ids)
            .field("mod_list", &self.mod_list)
            .field("collection", &self.collection)
            .field("steamcmd", &self.steamcmd)
            .field("steamcmd_dir", &self.steamcmd_dir)
            .field("output", &self.output)
            .field("manifest", &self.manifest)
            .field("name_mode", &self.name_mode)
            .field("name_prefix", &self.name_prefix)
            .field("max_retries", &self.max_retries)
            .field("retry_delay", &self.retry_delay)
            .field("batch_size", &self.batch_size)
            .field("dry_run", &self.dry_run)
            .field("quiet", &self.quiet)
            .field("delete_unrequested", &self.delete_unrequested)
            .field("delete_unavailable", &self.delete_unavailable)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

type Error = Box<dyn std::error::Error>;

/// A Workshop ID paired with the name written beside it in a mod list, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedMod {
    pub mod_id: String,
    pub name: Option<String>,
}

pub fn run(cli: Cli) -> Result<(), Error> {
    match cli.command {
        Commands::Sync(args) => run_sync(args),
    }
}

struct Reporter {
    quiet: bool,
}

impl Reporter {
    fn log(&self, message: &str) {
        if !self.quiet {
            eprintln!("[{}] {message}", chrono::Local::now().format("%H:%M:%S"));
        }
    }

    fn warn(&self, message: &str) {
        eprintln!(
            "[{}] warning: {message}",
            chrono::Local::now().format("%H:%M:%S")
        );
    }
}

/// Warnings for requested items Steam will not serve: ones it reports as
/// unavailable, and ones published for a different app. Both surface at
/// download time as an indistinguishable SteamCMD failure, so they are worth
/// naming before the download is attempted.
fn availability_warnings(
    app_id: u32,
    mod_ids: &[String],
    published_files: &HashMap<String, crate::steam_api::PublishedFile>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for mod_id in mod_ids {
        let Some(file) = published_files.get(mod_id) else {
            continue;
        };
        let label = if file.title.is_empty() {
            mod_id.clone()
        } else {
            format!("{mod_id} ({})", file.title)
        };
        if let Some(reason) = file.unavailable_reason() {
            warnings.push(format!(
                "{label}: Steam reports this item as {reason} — SteamCMD will not be able to download it"
            ));
            // An unavailable item has no meaningful app to compare against.
            continue;
        }
        if let Some(item_app) = file.app_id
            && item_app != app_id
        {
            warnings.push(format!(
                "{label}: published for app {item_app}, not --app-id {app_id} — SteamCMD will not be able to download it"
            ));
        }
    }
    warnings
}

/// Reason recorded when SteamCMD exits without leaving any content behind.
const NO_DOWNLOAD: &str = "SteamCMD did not produce a download";

/// Reason recorded when SteamCMD gave up on an item part-way through.
const TIMED_OUT: &str = "SteamCMD timed out downloading this item; the partial download is kept and the next attempt resumes where it stopped";

/// SteamCMD reports `Download item <id> failed (No Connection)` for every item
/// when it is logged in anonymously but the app's Workshop requires an account
/// that owns the app (Arma 3 and most paid titles). The failures look like
/// network trouble, so spell out the likely cause instead.
fn anonymous_failure_hint(
    app_id: u32,
    failures: &[(String, String)],
    anonymous: bool,
) -> Option<String> {
    if !anonymous || failures.is_empty() {
        return None;
    }
    if !failures.iter().all(|(_, reason)| reason == NO_DOWNLOAD) {
        return None;
    }
    let config = config_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "the swmctl config file".into());
    Some(format!(
        "SteamCMD ran anonymously and downloaded nothing for app {app_id}. \
Workshop items for paid apps (Arma 3, app 107410, among them) can only be downloaded by a Steam account that owns the app; \
anonymous attempts fail with \"(No Connection)\". \
Supply credentials with --username/--password, the SWMCTL_STEAM_USERNAME and SWMCTL_STEAM_PASSWORD environment variables, or a [steamcmd] section in {config}."
    ))
}

/// SteamCMD enforces its own download timeout and offers no way to raise it.
/// What is left is to keep resuming: the partial download survives, so each
/// further attempt starts from where the last one stopped.
fn timeout_hint(failures: &[(String, String)], batch_size: usize) -> Option<String> {
    let count = failures
        .iter()
        .filter(|(_, reason)| reason == TIMED_OUT)
        .count();
    if count == 0 {
        return None;
    }
    let mut hint = format!(
        "{count} item(s) timed out. SteamCMD gives up on an item that takes too long, but keeps what it downloaded: \
re-running resumes it, and large items often need several passes. Raise --max-retries to make one run keep trying."
    );
    if batch_size > 1 {
        hint.push_str(" Pass --batch-size 1 so a timeout no longer discards the items downloading alongside it.");
    }
    Some(hint)
}

fn run_sync(mut args: SyncArgs) -> Result<(), Error> {
    let report = Reporter { quiet: args.quiet };

    // SteamCMD resolves a relative `+force_install_dir` against its own
    // installation directory, while swmctl reads the staged files back
    // relative to the working directory. Left relative, the two disagree and
    // every item is reported as "SteamCMD did not produce a download".
    args.steamcmd_dir = absolute(&args.steamcmd_dir).map_err(|error| {
        format!(
            "could not resolve --steamcmd-dir {}: {error}",
            args.steamcmd_dir.display()
        )
    })?;

    let requested = resolve_requested(&args)?;
    if requested.is_empty() {
        return Err("provide mod IDs, --mod-list, or --collection".into());
    }
    let mod_ids = requested
        .iter()
        .map(|item| item.mod_id.clone())
        .collect::<Vec<_>>();

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
            report.warn(&format!(
                "could not fetch Workshop titles and versions: {error}"
            ));
            report.warn("falling back to SteamCMD metadata and the existing manifest");
            None
        }
    };

    if let Some(files) = published_files.as_ref() {
        for warning in availability_warnings(args.app_id, &mod_ids, files) {
            report.warn(&warning);
        }
    }

    let remote = build_remote_metadata(&requested, &metadata, published_files.as_ref());
    // Availability is only knowable when Steam answered.
    let delete_unavailable = args.delete_unavailable && published_files.is_some();
    if args.delete_unavailable && published_files.is_none() {
        report.warn("skipping --delete-unavailable: Workshop metadata is unavailable this run");
    }

    let actions = plan(
        &mod_ids,
        &remote,
        &entries,
        args.delete_unrequested,
        delete_unavailable,
    );

    let mut pending = actions
        .iter()
        .filter(|action| matches!(action.action, Action::Download | Action::Update))
        .map(|action| action.mod_id.clone())
        .collect::<Vec<_>>();
    let doomed = actions
        .iter()
        .filter(|action| action.action == Action::Delete)
        .map(|action| action.mod_id.clone())
        .collect::<Vec<_>>();

    // The script's "in state but not in preset" warning: surface drift even
    // when the user has not opted into deletion.
    if !args.delete_unrequested {
        for entry in &entries {
            if !mod_ids.contains(&entry.mod_id) {
                report.warn(&format!(
                    "{} ({}) is managed but not requested — pass --delete-unrequested to remove it",
                    entry.mod_id, entry.directory_name
                ));
            }
        }
    }

    report.log(&format!("Requested   : {} mod(s)", mod_ids.len()));
    report.log(&format!("To download : {}", pending.len()));
    report.log(&format!("To delete   : {}", doomed.len()));
    report.log(&format!("Output      : {}", args.output.display()));
    report.log(&format!("SteamCMD dir: {}", args.steamcmd_dir.display()));

    if args.dry_run {
        for action in &actions {
            println!("{:?} {}", action.action, action.mod_id);
        }
        report.log("dry run — nothing was downloaded, moved, or deleted");
        return Ok(());
    }

    let credentials = {
        let config = config_path()
            .map(|path| load_config(&path))
            .transpose()?
            .unwrap_or_default();
        resolve_credentials(
            &CredentialOverrides {
                username: args.username.clone(),
                password: args.password.clone(),
            },
            &config,
        )?
    };
    let anonymous = credentials.is_anonymous();
    report.log(&format!(
        "Steam login : {}",
        if anonymous {
            "anonymous"
        } else {
            "authenticated"
        }
    ));
    let steamcmd = SteamCmd::new(args.steamcmd.clone(), credentials).quiet(args.quiet);

    let mut failures: Vec<(String, String)> = Vec::new();
    let attempts = args.max_retries.max(1);
    let batch_size = args.batch_size.max(1);
    report.log(&format!(
        "Batch size  : {batch_size} item(s) per SteamCMD run"
    ));

    for attempt in 1..=attempts {
        if pending.is_empty() {
            break;
        }
        report.log(&format!(
            "Attempt {attempt}/{attempts} — {} mod(s) remaining",
            pending.len()
        ));

        let mut timed_out: Vec<String> = Vec::new();
        for batch in pending.chunks(batch_size) {
            // SteamCMD prints one line when an item starts and nothing at all
            // until it finishes, so a large mod leaves the terminal silent for
            // minutes. Name what is being fetched, and how big it is, before
            // handing over.
            for mod_id in batch {
                report.log(&format!(
                    "Downloading : {}",
                    download_label(remote.iter().find(|item| item.mod_id == *mod_id), mod_id)
                ));
            }
            // A missing or unusable executable will not fix itself; an
            // unsuccessful exit is reported per item below.
            let download = steamcmd.workshop_download(args.app_id, batch, &args.steamcmd_dir)?;

            let batch_timeouts = download.timed_out().map(str::to_string).collect::<Vec<_>>();
            for mod_id in &batch_timeouts {
                report.warn(&format!("{mod_id}: {TIMED_OUT}"));
            }
            if !batch_timeouts.is_empty() && batch.len() > 1 {
                report.warn(&format!(
                    "the timeout discarded whatever else was in flight; the other {} item(s) in this batch are retried",
                    batch.len() - batch_timeouts.len()
                ));
            }
            timed_out.extend(batch_timeouts);

            if let Some(error) = download.failure() {
                report.warn(&format!("{error}"));
            }
        }

        failures.clear();
        let mut still_pending = Vec::new();
        for mod_id in &pending {
            match place_requested(&args, &remote, &metadata, mod_id) {
                Ok(entry) => {
                    report.log(&format!("Placed: {mod_id} => {}", entry.directory_name));
                    entries.retain(|existing| existing.mod_id != entry.mod_id);
                    entries.push(entry);
                }
                Err(error) => {
                    // A timeout explains the missing content better than the
                    // empty staging directory it leaves behind.
                    let reason = if timed_out.contains(mod_id) {
                        TIMED_OUT.to_string()
                    } else {
                        error.to_string()
                    };
                    failures.push((mod_id.clone(), reason));
                    still_pending.push(mod_id.clone());
                }
            }
        }
        pending = still_pending;

        if pending.is_empty() {
            break;
        }
        if attempt < attempts {
            report.warn(&format!(
                "{} mod(s) incomplete; retrying in {}s",
                pending.len(),
                args.retry_delay
            ));
            sleep(Duration::from_secs(args.retry_delay));
        }
    }

    let deleted = apply_deletions(&args, &entries, &doomed, &report);
    entries.retain(|entry| !doomed.contains(&entry.mod_id));

    // Save before reporting failures: mods already on disk must be recorded
    // even when the run as a whole did not succeed.
    save_manifest(&manifest_path, &entries)?;
    cleanup_steamcmd_dir(&args);

    if deleted > 0 {
        report.log(&format!("Deleted     : {deleted} mod(s)"));
    }

    if failures.is_empty() {
        report.log(&format!("All {} mod(s) in place.", mod_ids.len()));
        return Ok(());
    }

    for (mod_id, reason) in &failures {
        report.warn(&format!("  {mod_id}: {reason}"));
    }
    if let Some(hint) = anonymous_failure_hint(args.app_id, &failures, anonymous) {
        report.warn(&hint);
    }
    if let Some(hint) = timeout_hint(&failures, batch_size) {
        report.warn(&hint);
    }
    Err(format!(
        "{} mod(s) failed after {attempts} attempt(s); re-run to retry — completed mods are skipped",
        failures.len()
    )
    .into())
}

/// `450814997 (CBA_A3, 1.2 GiB)`, dropping whatever Steam did not tell us.
fn download_label(item: Option<&RemoteMod>, mod_id: &str) -> String {
    let mut details = Vec::new();
    if let Some(item) = item {
        if item.name != mod_id {
            details.push(item.name.clone());
        }
        if item.file_size > 0 {
            details.push(human_size(item.file_size));
        }
    }
    if details.is_empty() {
        mod_id.to_string()
    } else {
        format!("{mod_id} ({})", details.join(", "))
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn place_requested(
    args: &SyncArgs,
    remote: &[RemoteMod],
    metadata: &[crate::metadata::WorkshopMetadata],
    mod_id: &str,
) -> Result<ManifestEntry, Error> {
    let source = workshop_content_path(&args.steamcmd_dir, args.app_id, mod_id);
    if !source.is_dir() {
        let partial = workshop_download_path(&args.steamcmd_dir, mod_id);
        if partial.exists() {
            return Err(format!("still downloading in {}", partial.display()).into());
        }
        return Err(NO_DOWNLOAD.into());
    }

    let item = remote.iter().find(|item| item.mod_id == mod_id);
    let title = item.map_or(mod_id, |item| item.name.as_str());
    let destination = place_mod(
        &source,
        &args.output,
        mod_id,
        title,
        args.name_mode,
        &args.name_prefix,
    )?;
    let size = directory_size(&destination)?;
    let directory_name = destination.file_name().map_or_else(
        || mod_id.to_string(),
        |name| name.to_string_lossy().into_owned(),
    );

    Ok(ManifestEntry {
        downloaded_at: chrono::Utc::now().to_rfc3339(),
        name: title.to_string(),
        mod_id: mod_id.to_string(),
        file_size: size,
        last_updated: item.and_then(|item| item.last_updated).or_else(|| {
            metadata
                .iter()
                .find(|item| item.mod_id == mod_id)
                .and_then(|item| item.last_updated)
        }),
        directory_name,
    })
}

fn apply_deletions(
    args: &SyncArgs,
    entries: &[ManifestEntry],
    doomed: &[String],
    report: &Reporter,
) -> usize {
    let mut deleted = 0;
    for entry in entries.iter().filter(|e| doomed.contains(&e.mod_id)) {
        let path = args.output.join(resolved_directory_name(args, entry));
        if !path.exists() {
            report.warn(&format!(
                "{} was recorded at {} but that directory is gone",
                entry.mod_id,
                path.display()
            ));
            continue;
        }
        match fs::remove_dir_all(&path) {
            Ok(()) => {
                report.log(&format!("Removed: {} ({})", entry.mod_id, path.display()));
                deleted += 1;
            }
            Err(error) => report.warn(&format!("could not remove {}: {error}", path.display())),
        }
    }
    deleted
}

/// `directory_name` is `#[serde(default)]`, so manifests written before it
/// existed carry an empty string. Reconstruct the sanitized folder name rather
/// than falling back to the raw Workshop title, which never matches on disk.
fn resolved_directory_name(args: &SyncArgs, entry: &ManifestEntry) -> String {
    if entry.directory_name.is_empty() {
        directory_name(
            args.name_mode,
            &entry.mod_id,
            &entry.name,
            &args.name_prefix,
        )
    } else {
        entry.directory_name.clone()
    }
}

/// Remove SteamCMD's now-empty staging directories, as the shell script's
/// trailing `rmdir -p` does. Best effort: a non-empty directory is left alone.
fn cleanup_steamcmd_dir(args: &SyncArgs) {
    let content = args
        .steamcmd_dir
        .join("steamapps/workshop/content")
        .join(args.app_id.to_string());
    let _ = fs::remove_dir(&content);
    let _ = fs::remove_dir(args.steamcmd_dir.join("steamapps/workshop/content"));
    let _ = fs::remove_dir(args.steamcmd_dir.join("steamapps/workshop/downloads"));
}

fn build_remote_metadata(
    requested: &[RequestedMod],
    acf: &[crate::metadata::WorkshopMetadata],
    published_files: Option<&HashMap<String, crate::steam_api::PublishedFile>>,
) -> Vec<RemoteMod> {
    requested
        .iter()
        .map(|item| {
            let mod_id = &item.mod_id;
            let api_item = published_files.and_then(|files| files.get(mod_id));
            let acf_item = acf.iter().find(|entry| entry.mod_id == *mod_id);
            RemoteMod {
                // A name written in the mod list is a deliberate choice and
                // wins over the Workshop title.
                name: item
                    .name
                    .clone()
                    .or_else(|| {
                        api_item
                            .filter(|item| !item.title.is_empty())
                            .map(|item| item.title.clone())
                    })
                    .unwrap_or_else(|| mod_id.clone()),
                mod_id: mod_id.clone(),
                file_size: api_item
                    .and_then(|item| item.file_size)
                    .or_else(|| acf_item.map(|item| item.file_size))
                    .unwrap_or_default(),
                last_updated: api_item
                    .and_then(|item| item.time_updated)
                    .or_else(|| acf_item.and_then(|item| item.last_updated)),
                deleted: api_item.is_some_and(|item| item.deleted),
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

fn parse_mod_id(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|character| character.is_ascii_digit()) {
        Ok(trimmed.to_string())
    } else {
        Err(format!("`{value}` is not a numeric Workshop ID"))
    }
}

fn resolve_requested(args: &SyncArgs) -> Result<Vec<RequestedMod>, Error> {
    let mut items = args
        .mod_ids
        .iter()
        .map(|mod_id| RequestedMod {
            mod_id: mod_id.clone(),
            name: None,
        })
        .collect::<Vec<_>>();

    if let Some(path) = &args.mod_list {
        let contents = fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        for (line_number, line) in contents.lines().enumerate() {
            if let Some(item) = parse_mod_list_line(line).map_err(|error| {
                format!(
                    "invalid mod list entry at {}:{}: {error}",
                    path.display(),
                    line_number + 1
                )
            })? {
                items.push(item);
            }
        }
    }

    if let Some(collection) = &args.collection {
        items.extend(
            fetch_collection_items(collection)?
                .into_iter()
                .map(|id| RequestedMod {
                    mod_id: id,
                    name: None,
                }),
        );
    }

    // First mention wins, so a name from a mod list survives a later bare ID.
    let mut unique: Vec<RequestedMod> = Vec::new();
    for item in items {
        match unique.iter_mut().find(|seen| seen.mod_id == item.mod_id) {
            Some(seen) => {
                if seen.name.is_none() {
                    seen.name = item.name;
                }
            }
            None => unique.push(item),
        }
    }
    Ok(unique)
}

/// A mod list line is a Workshop ID, optionally followed by a name:
/// `450814997 CBA_A3   # comment`
fn parse_mod_list_line(line: &str) -> Result<Option<RequestedMod>, &'static str> {
    let value = line.split('#').next().unwrap_or("").trim();
    if value.is_empty() {
        return Ok(None);
    }

    let (id, name) = match value.split_once(char::is_whitespace) {
        Some((id, rest)) => (id, rest.trim()),
        None => (value, ""),
    };
    if !id.chars().all(|character| character.is_ascii_digit()) {
        return Err("expected a numeric Workshop ID");
    }

    Ok(Some(RequestedMod {
        mod_id: id.to_string(),
        name: (!name.is_empty()).then(|| name.to_string()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requested(id: &str, name: Option<&str>) -> Option<RequestedMod> {
        Some(RequestedMod {
            mod_id: id.into(),
            name: name.map(str::to_string),
        })
    }

    #[test]
    fn parses_mod_list_entries_and_comments() {
        assert_eq!(
            parse_mod_list_line("123 # server mod"),
            Ok(requested("123", None))
        );
        assert_eq!(parse_mod_list_line("  # comment"), Ok(None));
        assert_eq!(parse_mod_list_line("   "), Ok(None));
    }

    #[test]
    fn reads_the_name_written_beside_the_id() {
        assert_eq!(
            parse_mod_list_line("450814997 CBA_A3"),
            Ok(requested("450814997", Some("CBA_A3")))
        );
        assert_eq!(
            parse_mod_list_line("463939057   Advanced Combat Environment  # ace"),
            Ok(requested("463939057", Some("Advanced Combat Environment")))
        );
    }

    #[test]
    fn rejects_non_numeric_mod_list_entries() {
        assert_eq!(
            parse_mod_list_line("not-a-workshop-id"),
            Err("expected a numeric Workshop ID")
        );
    }

    #[test]
    fn rejects_non_numeric_positional_ids() {
        assert!(parse_mod_id("abc").is_err());
        assert_eq!(parse_mod_id("123").as_deref(), Ok("123"));
    }

    #[test]
    fn mod_list_names_take_precedence_over_workshop_titles() {
        let requested = vec![RequestedMod {
            mod_id: "1".into(),
            name: Some("CBA_A3".into()),
        }];
        let files = HashMap::from([(
            "1".to_string(),
            crate::steam_api::PublishedFile {
                title: "Community Base Addons".into(),
                file_size: Some(10),
                time_updated: Some(5),
                deleted: false,
                result: 1,
                app_id: Some(107410),
            },
        )]);

        let remote = build_remote_metadata(&requested, &[], Some(&files));
        assert_eq!(remote[0].name, "CBA_A3");
        assert_eq!(remote[0].file_size, 10);
    }

    #[test]
    fn falls_back_to_the_workshop_title_when_the_list_has_no_name() {
        let requested = vec![RequestedMod {
            mod_id: "1".into(),
            name: None,
        }];
        let files = HashMap::from([(
            "1".to_string(),
            crate::steam_api::PublishedFile {
                title: "Community Base Addons".into(),
                file_size: None,
                time_updated: None,
                deleted: false,
                result: 1,
                app_id: Some(107410),
            },
        )]);

        let remote = build_remote_metadata(&requested, &[], Some(&files));
        assert_eq!(remote[0].name, "Community Base Addons");
    }

    #[test]
    fn a_missing_api_record_is_not_treated_as_deleted() {
        let requested = vec![RequestedMod {
            mod_id: "private".into(),
            name: None,
        }];
        let remote = build_remote_metadata(&requested, &[], Some(&HashMap::new()));
        assert!(!remote[0].deleted);
    }

    #[test]
    fn debug_output_redacts_the_password() {
        let args = SyncArgs {
            app_id: 107410,
            mod_ids: vec![],
            mod_list: None,
            collection: None,
            steamcmd: "steamcmd".into(),
            steamcmd_dir: ".swmctl-steamcmd".into(),
            output: ".".into(),
            manifest: None,
            name_mode: NameMode::ModId,
            name_prefix: String::new(),
            max_retries: 3,
            retry_delay: 30,
            batch_size: 1,
            dry_run: false,
            quiet: false,
            delete_unrequested: false,
            delete_unavailable: false,
            username: Some("alice".into()),
            password: Some("hunter2".into()),
        };

        let rendered = format!("{args:?}");
        assert!(!rendered.contains("hunter2"));
        assert!(rendered.contains("<redacted>"));
    }

    fn published(title: &str, result: i32, app_id: Option<u32>) -> crate::steam_api::PublishedFile {
        crate::steam_api::PublishedFile {
            title: title.into(),
            file_size: Some(10),
            time_updated: Some(5),
            deleted: result == 9,
            result,
            app_id,
        }
    }

    #[test]
    fn warns_about_items_steam_will_not_serve() {
        let files = HashMap::from([
            ("1".to_string(), published("Fine", 1, Some(107410))),
            ("2".to_string(), published("Hidden", 8, Some(107410))),
            ("3".to_string(), published("Gone", 9, Some(107410))),
            ("4".to_string(), published("Wrong App", 1, Some(221100))),
        ]);
        let mod_ids = vec!["1".into(), "2".into(), "3".into(), "4".into()];

        let warnings = availability_warnings(107410, &mod_ids, &files);

        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings[0].contains("2 (Hidden)") && warnings[0].contains("hidden"));
        assert!(warnings[1].contains("3 (Gone)") && warnings[1].contains("deleted"));
        assert!(warnings[2].contains("4 (Wrong App)") && warnings[2].contains("221100"));
    }

    #[test]
    fn healthy_items_and_unknown_ids_produce_no_warnings() {
        let files = HashMap::from([("1".to_string(), published("Fine", 1, Some(107410)))]);
        // An id Steam did not answer for is left to the download to resolve,
        // and a missing app_id is not evidence of a mismatch.
        let mod_ids = vec!["1".into(), "999".into()];

        assert!(availability_warnings(107410, &mod_ids, &files).is_empty());
        assert!(
            availability_warnings(
                107410,
                &["2".to_string()],
                &HashMap::from([("2".to_string(), published("No App", 1, None))]),
            )
            .is_empty()
        );
    }

    #[test]
    fn hints_at_credentials_when_anonymous_downloads_produce_nothing() {
        let failures = vec![
            ("583496184".to_string(), NO_DOWNLOAD.to_string()),
            ("463939057".to_string(), NO_DOWNLOAD.to_string()),
        ];

        let hint = anonymous_failure_hint(107410, &failures, true).expect("hint should be offered");
        assert!(hint.contains("107410"));
        assert!(hint.contains("--username"));
    }

    #[test]
    fn labels_a_download_with_whatever_steam_told_us() {
        let item = RemoteMod {
            name: "CBA_A3".into(),
            mod_id: "450814997".into(),
            file_size: 1_288_490_188,
            last_updated: None,
            deleted: false,
        };
        assert_eq!(
            download_label(Some(&item), "450814997"),
            "450814997 (CBA_A3, 1.2 GiB)"
        );

        // A name that is just the ID again, and an unknown size, add nothing.
        let bare = RemoteMod {
            name: "450814997".into(),
            mod_id: "450814997".into(),
            file_size: 0,
            last_updated: None,
            deleted: false,
        };
        assert_eq!(download_label(Some(&bare), "450814997"), "450814997");
        assert_eq!(download_label(None, "450814997"), "450814997");
    }

    #[test]
    fn scales_sizes_to_the_nearest_unit() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KiB");
        assert_eq!(human_size(5_242_880), "5.0 MiB");
    }

    #[test]
    fn hints_at_resuming_when_items_time_out() {
        let failures = vec![
            ("541888371".to_string(), TIMED_OUT.to_string()),
            ("463939057".to_string(), NO_DOWNLOAD.to_string()),
        ];

        let hint = timeout_hint(&failures, 1).expect("hint should be offered");
        assert!(hint.starts_with("1 item(s) timed out"));
        assert!(!hint.contains("--batch-size"));

        let batched = timeout_hint(&failures, 8).expect("hint should be offered");
        assert!(batched.contains("--batch-size 1"));
    }

    #[test]
    fn does_not_hint_about_timeouts_when_none_timed_out() {
        assert!(timeout_hint(&[], 1).is_none());
        assert!(
            timeout_hint(&[("1".to_string(), NO_DOWNLOAD.to_string())], 4).is_none(),
            "an ordinary failure is not a timeout"
        );
    }

    /// A timeout is not a "(No Connection)" failure, so the credentials hint
    /// stays out of the way when the run was merely too slow.
    #[test]
    fn a_timeout_suppresses_the_anonymous_credentials_hint() {
        let failures = vec![
            ("541888371".to_string(), TIMED_OUT.to_string()),
            ("463939057".to_string(), NO_DOWNLOAD.to_string()),
        ];
        assert!(anonymous_failure_hint(107410, &failures, true).is_none());
    }

    #[test]
    fn does_not_hint_when_authenticated_or_partially_downloaded() {
        let failures = vec![("583496184".to_string(), NO_DOWNLOAD.to_string())];
        assert!(anonymous_failure_hint(107410, &failures, false).is_none());

        let mixed = vec![
            ("583496184".to_string(), NO_DOWNLOAD.to_string()),
            (
                "463939057".to_string(),
                "still downloading in /tmp".to_string(),
            ),
        ];
        assert!(anonymous_failure_hint(107410, &mixed, true).is_none());

        assert!(anonymous_failure_hint(107410, &[], true).is_none());
    }
}
