# swmctl

[![CI](https://github.com/Saniee/swmctl/actions/workflows/ci.yml/badge.svg)](https://github.com/Saniee/swmctl/actions/workflows/ci.yml)
[![Build](https://github.com/Saniee/swmctl/actions/workflows/release.yml/badge.svg)](https://github.com/Saniee/swmctl/actions/workflows/release.yml)

`swmctl` is a command-line tool for keeping Steam Workshop mods synchronized with a local game-server directory.

It uses SteamCMD as the source of truth, compares Workshop metadata with a local manifest, and performs only the downloads, updates, and deletes required to reach the requested state.

## Features

- Download and update Workshop mods by ID.
- Compare remote metadata with a local minified JSON manifest.
- Remove mods no longer requested, with independent deletion checks.
- Rename folders by mod ID or sanitized Workshop name.
- Sanitize names for restrictive game servers such as Arma.
- Download larger mods first when several transfers are queued.
- Support anonymous and authenticated SteamCMD access.

## Requirements

- Rust and Cargo for development.
- SteamCMD installed and available to `swmctl`.
- A Steam account for Workshop items that require authentication.

## Authentication

Authentication is delegated entirely to SteamCMD. `swmctl` does not implement or cache Steam sessions.

Credentials may be supplied through:

1. CLI options.
2. Environment variables.
3. A configuration file in the platform-native configuration directory.

When no credentials are configured, `swmctl` uses anonymous SteamCMD access. Steam Guard codes and other SteamCMD prompts remain interactive in the terminal. Failed SteamCMD commands cause `swmctl` to exit with an error.

## Workflow

For each requested Workshop mod, `swmctl`:

1. Query SteamCMD for current metadata.
2. Compare the result with the local manifest.
3. Classify the mod as a download, update, or delete.
4. Download required items through SteamCMD.
5. Move and rename mod folders in the target directory.
6. Write the resulting state to the manifest.

The target directory defaults to the current working directory and can be overridden. The manifest defaults to the target directory and can also be overridden.

## Usage

Synchronize Workshop items for a Steam application:

```sh
swmctl sync --app-id 107410 123456789 987654321
```

Use a text file with one Workshop ID per line:

```text
# Server mods
123456789
987654321 # inline comments are allowed
```

```sh
swmctl sync --app-id 107410 --mod-list mods.txt
```

Or expand a Steam Workshop collection by ID or URL:

```sh
swmctl sync --app-id 107410 --collection 123456789
swmctl sync --app-id 107410 --collection "https://steamcommunity.com/sharedfiles/filedetails/?id=123456789"
```

Positional IDs, `--mod-list`, and `--collection` can be combined. Duplicate IDs are downloaded once.

Useful options include `--output`, `--manifest`, `--steamcmd`, `--steamcmd-dir`, `--name-mode`, `--delete-unrequested`, and `--delete-unavailable`.

## Installation

Release binaries are published for Linux and Windows. The installers download the latest release and install it to a user-local directory.

The Unix installer uses `curl`; the PowerShell installer uses PowerShell's native download facilities.

Unix-style shell:

```sh
curl -fsSL https://raw.githubusercontent.com/Saniee/swmctl/main/scripts/install.sh | sh
```

PowerShell:

```powershell
irm https://raw.githubusercontent.com/Saniee/swmctl/main/scripts/install.ps1 | iex
```

Set `SWMCTL_REPOSITORY` when installing from a fork.

Both installers support an optional version override. Uninstall removes only the binary; configuration, manifests, and downloaded mods are preserved.

## Development
