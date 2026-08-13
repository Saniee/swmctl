use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Credentials {
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CredentialOverrides {
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    steamcmd: Option<ConfigSteamCmd>,
}

#[derive(Debug, Default, Deserialize)]
struct ConfigSteamCmd {
    username: Option<String>,
    password: Option<String>,
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("could not read config file {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not parse config file {path}: {source}")]
    ParseConfig {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    #[error("SteamCMD username and password must be supplied together")]
    IncompleteCredentials,
}

pub fn config_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "swmctl").map(|dirs| dirs.config_dir().join("config.toml"))
}

pub fn load_config(path: &Path) -> Result<Credentials, AuthError> {
    if !path.exists() {
        return Ok(Credentials::default());
    }

    let contents = fs::read_to_string(path).map_err(|source| AuthError::ReadConfig {
        path: path.to_path_buf(),
        source,
    })?;
    let config: ConfigFile =
        toml::from_str(&contents).map_err(|source| AuthError::ParseConfig {
            path: path.to_path_buf(),
            source: Box::new(source),
        })?;
    let steamcmd = config.steamcmd.unwrap_or_default();

    Ok(Credentials {
        username: steamcmd.username,
        password: steamcmd.password,
    })
}

pub fn resolve_credentials(
    overrides: &CredentialOverrides,
    config: &Credentials,
) -> Result<Credentials, AuthError> {
    let username = overrides
        .username
        .clone()
        .or_else(|| env::var("SWMCTL_STEAM_USERNAME").ok())
        .or_else(|| config.username.clone());
    let password = overrides
        .password
        .clone()
        .or_else(|| env::var("SWMCTL_STEAM_PASSWORD").ok())
        .or_else(|| config.password.clone());

    if username.is_some() != password.is_some() {
        return Err(AuthError::IncompleteCredentials);
    }

    Ok(Credentials { username, password })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_config(contents: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!("swmctl-{suffix}.toml"));
        fs::write(&path, contents).expect("config should be writable");
        path
    }

    #[test]
    fn loads_steamcmd_credentials_from_config() {
        let path = temp_config("[steamcmd]\nusername = 'alice'\npassword = 'secret'\n");
        let credentials = load_config(&path).expect("config should parse");
        fs::remove_file(path).expect("test config should be removable");

        assert_eq!(credentials.username.as_deref(), Some("alice"));
        assert_eq!(credentials.password.as_deref(), Some("secret"));
    }

    #[test]
    fn flags_override_config() {
        let config = Credentials {
            username: Some("config-user".into()),
            password: Some("config-pass".into()),
        };
        let overrides = CredentialOverrides {
            username: Some("flag-user".into()),
            password: Some("flag-pass".into()),
        };

        let credentials = resolve_credentials(&overrides, &config).expect("credentials are valid");
        assert_eq!(credentials.username.as_deref(), Some("flag-user"));
        assert_eq!(credentials.password.as_deref(), Some("flag-pass"));
    }

    #[test]
    fn partial_credentials_are_rejected() {
        let overrides = CredentialOverrides {
            username: Some("alice".into()),
            password: None,
        };

        assert!(matches!(
            resolve_credentials(&overrides, &Credentials::default()),
            Err(AuthError::IncompleteCredentials)
        ));
    }
}
