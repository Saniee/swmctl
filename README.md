# swmctl - Steam Workshop Manager Control (Steward)

[![CI](https://github.com/Saniee/swmctl/actions/workflows/ci.yml/badge.svg)](https://github.com/Saniee/swmctl/actions/workflows/ci.yml) [![Build](https://github.com/Saniee/swmctl/actions/workflows/release.yml/badge.svg)](https://github.com/Saniee/swmctl/actions/workflows/release.yml)

`swmctl` is a command-line tool for keeping Steam Workshop mods synchronized with a local game-server directory.

It uses SteamCMD as the source of truth, compares Workshop metadata with a local manifest, and performs only the downloads, updates, and deletes required to reach the requested state.

## Features

- Download and update Workshop mods by ID.
- Compare remote metadata with a local minified JSON manifest.
- Remove mods no longer requested, with independent deletion checks.
- Rename folders by mod ID or sanitized Workshop name, with an optional prefix (`@` for Arma).
- Sanitize names for restrictive game servers such as Arma.
- Download larger mods first when several transfers are queued.
- Retry only the items SteamCMD failed to deliver, and record what did succeed.
- Preview a run with `--dry-run` before anything is downloaded or deleted.
- Support anonymous and authenticated SteamCMD access.

## Requirements

- SteamCMD installed and available to `swmctl`.
- A Steam account for Workshop items that require authentication.
- Rust and Cargo, if building from source.

## Installation

Release binaries are published for Linux and Windows. The installers download the latest release and install it to a user-local directory.

**Unix-style shell** (uses `curl`):

```
curl -fsSL https://raw.githubusercontent.com/Saniee/swmctl/refs/heads/master/scripts/install.sh | sh
```

If `swmctl` is not found after installation, add its user-local bin directory to `PATH`:

```
export PATH="$HOME/.local/bin:$PATH"
```

**PowerShell** (uses native download facilities):

```
irm https://raw.githubusercontent.com/Saniee/swmctl/refs/heads/master/scripts/install.ps1 | iex
```

Set `SWMCTL_REPOSITORY` when installing from a fork.

Both installers accept an optional version override:

```
curl -fsSL https://raw.githubusercontent.com/Saniee/swmctl/refs/heads/master/scripts/install.sh | sh -s -- v0.3.0
```

```
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/Saniee/swmctl/refs/heads/master/scripts/install.ps1))) -Version v0.3.0
```

Uninstall removes only the binary; configuration, manifests, and downloaded mods are preserved.

## Usage

Synchronize Workshop items for a Steam application:

```
swmctl sync --app-id 107410 123456789 987654321
```

Use a text file with one Workshop ID per line:

```
# Server mods
123456789
987654321 # inline comments are allowed
450814997 CBA_A3            # an optional name after the ID
463939057 Advanced Combat Environment
```

A name written after the ID is used for the folder name in `--name-mode name`, taking precedence over the Workshop title.

```
swmctl sync --app-id 107410 --mod-list mods.txt
```

Or expand a Steam Workshop collection by ID or URL:

```
swmctl sync --app-id 107410 --collection 123456789
swmctl sync --app-id 107410 --collection "https://steamcommunity.com/sharedfiles/filedetails/?id=123456789"
```

Positional IDs, `--mod-list`, and `--collection` can be combined. Duplicate IDs are downloaded once.

### Arma servers

Arma expects mod folders to begin with `@`:

```
swmctl sync --app-id 107410 --mod-list mods.txt --name-mode name --name-prefix @
```

That produces `@cba_a3`, `@advanced_combat_environment`, and so on.

### Retries

```
swmctl sync --app-id 107410 --mod-list mods.txt --max-retries 5 --retry-delay 30
```

### Deletion

Deletion is off by default and split into two independent checks:

- `--delete-unrequested` removes managed mods absent from the requested list.
- `--delete-unavailable` removes managed mods that Steam reports as **deleted**.

Use `--dry-run` to preview planned actions before running for real.

### Other options

Useful options include `--output`, `--manifest`, `--steamcmd`, `--steamcmd-dir`, `--name-mode`, `--name-prefix`, `--max-retries`, `--retry-delay`, `--dry-run`, and `--quiet`.

## Authentication

Authentication is delegated entirely to SteamCMD. `swmctl` does not implement or cache Steam sessions.

Credentials may be supplied through:

1. CLI options.
2. Environment variables (`SWMCTL_STEAM_USERNAME` / `SWMCTL_STEAM_PASSWORD`).
3. A configuration file in the platform-native configuration directory.

When no credentials are configured, `swmctl` uses anonymous SteamCMD access. Paid titles require an account that owns the app:

```
swmctl sync --app-id 107410 --mod-list preset.txt --username <steam-user> --password <steam-password>
```

Accounts with Steam Guard prompt for a code on the first login in a terminal.

## Development

Requires a Rust toolchain supporting edition 2024 (1.85 or newer).

```
cargo build
cargo test
```

Before opening a pull request, run the same checks CI does:

```
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```
