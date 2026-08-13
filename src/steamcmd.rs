use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use crate::auth::Credentials;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct SteamCmd {
    executable: PathBuf,
    credentials: Credentials,
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
        }
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

    pub fn workshop_download(
        &self,
        app_id: u32,
        mod_ids: &[String],
        install_dir: &Path,
    ) -> Result<(), SteamCmdError> {
        let mut command = Command::new(&self.executable);
        command.args(self.workshop_arguments(app_id, mod_ids, install_dir));

        let status = command.status().map_err(|source| SteamCmdError::Start {
            path: self.executable.clone(),
            source,
        })?;
        if status.success() {
            Ok(())
        } else {
            Err(SteamCmdError::Failed { status })
        }
    }
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
}
