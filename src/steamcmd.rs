use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::auth::Credentials;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct SteamCmd {
    executable: PathBuf,
    credentials: Credentials,
    quiet: bool,
}

#[derive(Debug, Error)]
pub enum SteamCmdError {
    #[error("failed to start SteamCMD at {path}: {source}")]
    Start {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("SteamCMD exited unsuccessfully with status {status}")]
    Failed { status: ExitStatus },
}

/// What SteamCMD reported for a single Workshop item during one invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemOutcome {
    Downloaded,
    /// `ERROR! Timeout downloading item <id>`. SteamCMD gives up on the item
    /// and tears down its work queue, discarding whatever else was in flight.
    /// The bytes already fetched stay in `steamapps/workshop/downloads`, so a
    /// later attempt resumes rather than starting over.
    TimedOut,
    /// `ERROR! Download item <id> failed (<reason>)`.
    Failed(String),
}

/// Per-item results parsed out of one SteamCMD run, in the order reported.
#[derive(Debug, Clone)]
pub struct DownloadReport {
    pub status: ExitStatus,
    pub outcomes: Vec<(String, ItemOutcome)>,
}

impl DownloadReport {
    pub fn timed_out(&self) -> impl Iterator<Item = &str> {
        self.outcomes.iter().filter_map(|(mod_id, outcome)| {
            (*outcome == ItemOutcome::TimedOut).then_some(mod_id.as_str())
        })
    }

    /// The process-level failure, if SteamCMD exited unsuccessfully.
    pub fn failure(&self) -> Option<SteamCmdError> {
        (!self.status.success()).then_some(SteamCmdError::Failed {
            status: self.status,
        })
    }
}

pub fn workshop_content_path(install_dir: &Path, app_id: u32, mod_id: &str) -> PathBuf {
    install_dir
        .join("steamapps/workshop/content")
        .join(app_id.to_string())
        .join(mod_id)
}

pub fn workshop_download_path(install_dir: &Path, mod_id: &str) -> PathBuf {
    install_dir
        .join("steamapps/workshop/downloads")
        .join(mod_id)
}

impl SteamCmd {
    pub fn new(executable: impl Into<PathBuf>, credentials: Credentials) -> Self {
        Self {
            executable: executable.into(),
            credentials,
            quiet: false,
        }
    }

    /// Suppress the echo of SteamCMD's own output. Its lines are still read and
    /// parsed; only the passthrough to stdout is dropped.
    pub fn quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn workshop_arguments(
        &self,
        app_id: u32,
        mod_ids: &[String],
        install_dir: &Path,
    ) -> Vec<String> {
        let mut args = vec![
            "+force_install_dir".into(),
            install_dir.display().to_string(),
            "+login".into(),
        ];
        match (&self.credentials.username, &self.credentials.password) {
            (Some(username), Some(password)) => {
                args.push(username.clone());
                args.push(password.clone());
            }
            _ => args.push("anonymous".into()),
        }
        for mod_id in mod_ids {
            args.extend([
                "+workshop_download_item".into(),
                app_id.to_string(),
                mod_id.clone(),
            ]);
        }
        args.push("+quit".into());
        args
    }

    /// Run one SteamCMD invocation and report what it said about each item.
    ///
    /// An unsuccessful exit is not an error here: a timeout ends the process
    /// with status 10 after some items have already been downloaded, and the
    /// caller decides what that means. Only a SteamCMD that cannot be started
    /// fails outright.
    pub fn workshop_download(
        &self,
        app_id: u32,
        mod_ids: &[String],
        install_dir: &Path,
    ) -> Result<DownloadReport, SteamCmdError> {
        let mut command = Command::new(&self.executable);
        command
            .args(self.workshop_arguments(app_id, mod_ids, install_dir))
            .stdout(Stdio::piped());

        let mut child = command.spawn().map_err(|source| SteamCmdError::Start {
            path: self.executable.clone(),
            source,
        })?;

        let mut outcomes = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            let mut reader = BufReader::new(stdout);
            let mut chunk = Vec::new();
            // A read error only costs progress output and parsed outcomes; the
            // exit status below still describes the run.
            while let Ok(read) = read_chunk(&mut reader, &mut chunk)
                && read > 0
            {
                let text = String::from_utf8_lossy(&chunk);
                if !self.quiet {
                    let mut stdout = std::io::stdout();
                    let _ = stdout.write_all(text.as_bytes());
                    let _ = stdout.flush();
                }
                if let Some(outcome) = parse_item_outcome(&text) {
                    outcomes.push(outcome);
                }
            }
        }

        let status = child.wait().map_err(|source| SteamCmdError::Start {
            path: self.executable.clone(),
            source,
        })?;
        Ok(DownloadReport { status, outcomes })
    }
}

/// Read up to and including the next `\n` or `\r`. SteamCMD redraws its
/// progress line with carriage returns, so splitting on newlines alone would
/// hold the whole download back in the buffer.
fn read_chunk<R: BufRead>(reader: &mut R, buffer: &mut Vec<u8>) -> std::io::Result<usize> {
    buffer.clear();
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            return Ok(buffer.len());
        }
        match available
            .iter()
            .position(|byte| *byte == b'\n' || *byte == b'\r')
        {
            Some(index) => {
                buffer.extend_from_slice(&available[..=index]);
                reader.consume(index + 1);
                return Ok(buffer.len());
            }
            None => {
                let length = available.len();
                buffer.extend_from_slice(available);
                reader.consume(length);
            }
        }
    }
}

/// Pull an item result out of a line of SteamCMD output. The messages are not
/// always newline-terminated — a timeout runs straight into the shutdown
/// chatter (`...item 541888371Unloading Steam API...`) — so the ID is read as
/// the digits following the marker rather than as the rest of the line.
fn parse_item_outcome(line: &str) -> Option<(String, ItemOutcome)> {
    if let Some(rest) = after(line, "Success. Downloaded item ") {
        return leading_id(rest).map(|id| (id, ItemOutcome::Downloaded));
    }
    if let Some(rest) = after(line, "Timeout downloading item ") {
        return leading_id(rest).map(|id| (id, ItemOutcome::TimedOut));
    }
    if let Some(rest) = after(line, "Download item ") {
        let id = leading_id(rest)?;
        let reason = rest
            .split_once("failed (")
            .and_then(|(_, tail)| tail.split_once(')'))
            .map_or_else(|| "failed".to_string(), |(reason, _)| reason.to_string());
        return Some((id, ItemOutcome::Failed(reason)));
    }
    None
}

fn after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    line.find(marker).map(|index| &line[index + marker.len()..])
}

fn leading_id(text: &str) -> Option<String> {
    let id: String = text.chars().take_while(char::is_ascii_digit).collect();
    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_configured_executable() {
        let steamcmd = SteamCmd::new("steamcmd", Credentials::default());
        assert_eq!(steamcmd.executable(), Path::new("steamcmd"));
    }

    #[test]
    fn uses_steam_workshop_content_layout() {
        let root = Path::new("steamcmd");
        assert_eq!(
            workshop_content_path(root, 123, "456"),
            Path::new("steamcmd/steamapps/workshop/content/123/456")
        );
        assert_eq!(
            workshop_download_path(root, "456"),
            Path::new("steamcmd/steamapps/workshop/downloads/456")
        );
    }

    #[test]
    fn builds_authenticated_arguments() {
        let steamcmd = SteamCmd::new(
            "steamcmd",
            Credentials {
                username: Some("alice".into()),
                password: Some("secret".into()),
            },
        );
        let args = steamcmd.workshop_arguments(123, &["456".into()], Path::new("cache"));
        assert_eq!(
            args,
            [
                "+force_install_dir",
                "cache",
                "+login",
                "alice",
                "secret",
                "+workshop_download_item",
                "123",
                "456",
                "+quit",
            ]
        );
    }

    #[test]
    fn reads_a_timeout_that_runs_into_the_shutdown_chatter() {
        assert_eq!(
            parse_item_outcome("ERROR! Timeout downloading item 541888371Unloading Steam API..."),
            Some(("541888371".to_string(), ItemOutcome::TimedOut))
        );
    }

    #[test]
    fn reads_successes_and_failures() {
        assert_eq!(
            parse_item_outcome("Success. Downloaded item 450814997 to \"/srv/mods\" (1234 bytes)"),
            Some(("450814997".to_string(), ItemOutcome::Downloaded))
        );
        assert_eq!(
            parse_item_outcome("ERROR! Download item 463939057 failed (No Connection)."),
            Some((
                "463939057".to_string(),
                ItemOutcome::Failed("No Connection".to_string())
            ))
        );
    }

    #[test]
    fn ignores_unrelated_output() {
        assert!(parse_item_outcome("Downloading item 541888371 ...").is_none());
        assert!(parse_item_outcome("Logging in user 'alice' to Steam Public...").is_none());
        assert!(parse_item_outcome("Success. Downloaded item to nowhere").is_none());
    }

    #[test]
    fn splits_output_on_carriage_returns() {
        let mut reader = BufReader::new(&b"first\rsecond\nthird"[..]);
        let mut buffer = Vec::new();

        let mut chunks = Vec::new();
        while read_chunk(&mut reader, &mut buffer).unwrap() > 0 {
            chunks.push(String::from_utf8(buffer.clone()).unwrap());
        }
        assert_eq!(chunks, ["first\r", "second\n", "third"]);
    }
}
