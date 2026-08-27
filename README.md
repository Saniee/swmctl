# swmctl - Steam Workshop Manager Control (Steward)
[![CI](https://github.com/Saniee/swmctl/actions/workflows/ci.yml/badge.svg)](https://github.com/Saniee/swmctl/actions/workflows/ci.yml)
[![Build](https://github.com/Saniee/swmctl/actions/workflows/release.yml/badge.svg)](https://github.com/Saniee/swmctl/actions/workflows/release.yml)

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

Anonymous access only works for apps whose Workshop content Valve serves anonymously. Paid titles — Arma 3 (app `107410`) among them — require an account that **owns the app**. Without one, SteamCMD reports:

```text
ERROR! Download item 583496184 failed (No Connection).
```

despite the network being fine, and `swmctl` then reports `SteamCMD did not produce a download` for every item. Log in with an owning account to fix it:

```sh
swmctl sync --app-id 107410 --mod-list preset.txt --username <steam-user> --password <steam-password>
```

The same credentials can come from `SWMCTL_STEAM_USERNAME` / `SWMCTL_STEAM_PASSWORD` or the config file. Accounts with Steam Guard prompt for a code on the first login in a terminal; SteamCMD caches the session afterwards, so unattended runs work once that first login has been completed interactively on the same machine and user.

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
450814997 CBA_A3            # an optional name after the ID
463939057 Advanced Combat Environment
```

A name written after the ID is used for the folder name in `--name-mode name`,
taking precedence over the Workshop title. This lets a preset control its own
naming and keeps `--mod-list` usable when the Steam Web API is unreachable.

```sh
swmctl sync --app-id 107410 --mod-list mods.txt
```

Or expand a Steam Workshop collection by ID or URL:

```sh
swmctl sync --app-id 107410 --collection 123456789
swmctl sync --app-id 107410 --collection "https://steamcommunity.com/sharedfiles/filedetails/?id=123456789"
```

Positional IDs, `--mod-list`, and `--collection` can be combined. Duplicate IDs are downloaded once.

### Arma servers

Arma expects mod folders to begin with `@`:

```sh
swmctl sync --app-id 107410 --mod-list mods.txt --name-mode name --name-prefix @
```

That produces `@cba_a3`, `@advanced_combat_environment`, and so on.

### Retries

SteamCMD regularly drops individual Workshop items while still exiting
successfully. `swmctl` checks what actually arrived on disk and retries only the
items still missing:

```sh
swmctl sync --app-id 107410 --mod-list mods.txt --max-retries 5 --retry-delay 30
```

Mods placed during a failed run are written to the manifest before the error is
reported, so re-running skips them.

### Deletion

Deletion is off by default and split into two independent checks:

- `--delete-unrequested` removes managed mods absent from the requested list.
- `--delete-unavailable` removes managed mods that Steam reports as **deleted**.
  Items that are merely invisible to an unauthenticated metadata request, such
  as private or login-gated mods, are never removed by this flag.

Without `--delete-unrequested`, managed mods missing from the request are
reported as warnings and left alone. Use `--dry-run` to see the planned actions
first.

### Other options

Useful options include `--output`, `--manifest`, `--steamcmd`, `--steamcmd-dir`,
`--name-mode`, `--name-prefix`, `--max-retries`, `--retry-delay`, `--dry-run`,
and `--quiet`.

A relative `--steamcmd-dir` is resolved against the working directory before it
is handed to SteamCMD, which would otherwise place the staged files under its
own installation directory. The resolved path is printed at the start of each
run.

## Installation

Release binaries are published for Linux and Windows. The installers download the latest release and install it to a user-local directory.

The Unix installer uses `curl`; the PowerShell installer uses PowerShell's native download facilities.

Unix-style shell:

```sh
curl -fsSL https://raw.githubusercontent.com/Saniee/swmctl/refs/heads/master/scripts/install.sh | sh
```

If `swmctl` is not found after installation, add its user-local bin directory to `PATH`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

PowerShell:

```powershell
irm https://raw.githubusercontent.com/Saniee/swmctl/refs/heads/master/scripts/install.ps1 | iex
```

Set `SWMCTL_REPOSITORY` when installing from a fork.

Both installers accept an optional version override. Because the one-line forms
above pipe the script into a shell, the version is passed through that shell:

```sh
curl -fsSL https://raw.githubusercontent.com/Saniee/swmctl/refs/heads/master/scripts/install.sh | sh -s -- v0.2.1
```

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/Saniee/swmctl/refs/heads/master/scripts/install.ps1))) -Version v0.2.1
```

Uninstall removes only the binary; configuration, manifests, and downloaded mods are preserved.

## Development

Requires a Rust toolchain supporting edition 2024 (1.85 or newer).

```sh
cargo build
cargo test
```

Before opening a pull request, run the same checks CI does:

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```
