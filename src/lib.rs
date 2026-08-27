pub mod auth;
pub mod cli;
pub mod manifest;
pub mod metadata;
pub mod naming;
pub mod paths;
pub mod placement;
pub mod steam_api;
pub mod steamcmd;

pub use auth::{CredentialOverrides, Credentials, load_config, resolve_credentials};
pub use steamcmd::{DownloadReport, ItemOutcome, SteamCmd, SteamCmdError};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_exports_are_available() {
        let credentials = Credentials::default();
        assert!(credentials.username.is_none());
    }
}
