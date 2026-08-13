# Spec: Build, Release, and Installation Automation

## Goal

Build distributable `swmctl` CLI binaries automatically, publish semver-tagged releases, and provide user-local install/uninstall scripts for Linux/macOS-style shells and Windows PowerShell.

## Constraints

- GitHub Actions must build Linux and Windows CLI binaries.
- Tags matching `v*` and semver format, such as `v0.1.0`, create GitHub Releases.
- Installers download the latest release by default.
- Installation must use user-local locations and avoid elevation where possible.
- Uninstall removes only the `swmctl` binary; configuration, manifests, and downloaded mods remain.
- Releases must include SHA-256 checksums.
- Scripts must use the binaries produced by the release workflow rather than compiling locally.

## Approach

Add a GitHub Actions workflow that:

1. Builds release binaries for Linux GNU and Windows MSVC.
2. Runs formatting, tests, and lint checks.
3. Packages binaries with stable asset names.
4. Generates and publishes checksums.
5. Creates a GitHub Release when a valid `v*` semver tag is pushed.

Add shell and PowerShell scripts that can themselves be fetched remotely using `curl` or PowerShell's native web-download facilities. The scripts resolve the latest GitHub Release, download the matching binary using the platform-appropriate tool, install it into the user-local executable directory, and provide a binary-only uninstall path. They should also work from a local checkout, support an optional version override for reproducible installation, and must not require Rust, Cargo, or a package manager.

## Files touched

- `.github/workflows/` - build, test, package, checksum, and tag-release workflow
- `scripts/install.sh` - Unix-style installer
- `scripts/uninstall.sh` - Unix-style uninstaller
- `scripts/install.ps1` - PowerShell installer
- `scripts/uninstall.ps1` - PowerShell uninstaller
- `README.md` - installation, upgrade, uninstall, and release usage
- `Cargo.toml` and `src/` - ensure the CLI binary is defined and release-buildable

## Acceptance criteria

- [ ] A normal workflow run builds Linux and Windows release binaries.
- [ ] The workflow runs `cargo fmt --check`, `cargo test`, and Clippy with warnings denied.
- [ ] Pushing a valid `v*` semver tag creates a GitHub Release.
- [ ] The release contains both platform binaries and a SHA-256 checksum file.
- [ ] `install.sh` downloads the latest Linux binary by default.
- [ ] `install.ps1` downloads the latest Windows binary by default.
- [ ] `install.sh` uses `curl` for release downloads.
- [ ] `install.ps1` uses PowerShell's native web-download facilities.
- [ ] The installer scripts can be fetched and executed from the repository without cloning it.
- [ ] Both installers accept an optional explicit version.
- [ ] Installers use user-local destinations and do not require administrator/root privileges by default.
- [ ] Uninstall scripts remove only the installed binary.
- [ ] Scripts fail clearly on unsupported platforms, unavailable releases, failed downloads, or checksum mismatches.
- [ ] Installation and release usage is documented.

## Verification

```text
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Manual verification should run both installers against a test release, verify the binary executes, verify checksum validation, and confirm uninstall preserves configuration and downloaded mod data.
