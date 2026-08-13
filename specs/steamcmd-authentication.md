# Spec: SteamCMD Authentication Support

## Goal

Allow authenticated Steam Workshop downloads without reimplementing Steam login. `swmctl` resolves credentials from flags, environment variables, or a platform-native config file, then passes them directly to SteamCMD.

## Constraints

- SteamCMD remains responsible for login, passwords, sessions, and Steam Guard prompts.
- Steam Guard and other interactive SteamCMD input must pass through directly to the user's terminal.
- `swmctl` must not cache sessions, validate credentials independently, or store credentials outside the platform-native config file.
- Anonymous SteamCMD usage remains the default when no credentials are configured.

## Approach

Add credential resolution with precedence:

1. CLI flags
2. Environment variables
3. Platform-native config file
4. No credentials, allowing anonymous SteamCMD access

Invoke SteamCMD with the resolved credentials and inherited terminal input/output. If SteamCMD exits unsuccessfully, `swmctl` exits unsuccessfully and reports the failure without exposing the password.

## Files touched

- `src/` - credential model, config loading, SteamCMD invocation, and CLI wiring
- `Cargo.toml` - CLI, config-directory, and serialization dependencies as needed
- `tests/` - credential precedence, config loading, argument construction, and failure propagation

## Acceptance criteria

- [ ] Credentials can be supplied through CLI flags, environment variables, or the platform-native config file.
- [ ] Higher-precedence sources override lower-precedence sources.
- [ ] No credentials permits anonymous SteamCMD execution.
- [ ] SteamCMD receives the resolved credentials only for the current invocation.
- [ ] Steam Guard prompts and other SteamCMD input remain interactive.
- [ ] A failed SteamCMD process causes a failed `swmctl` command with a useful error.
- [ ] Passwords are not printed in logs or error messages.
- [ ] Automated tests cover credential precedence and SteamCMD failure propagation.

## Verification

```text
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
