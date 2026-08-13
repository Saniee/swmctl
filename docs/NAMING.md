# Naming Convention

## Primary name: `swmctl`

`swmctl` (**S**team **W**orkshop **M**anager **c**on**t**ro**l**) is the
canonical name for this project. It is used as:

- the crate name on crates.io
- the binary/CLI command
- the GitHub repository name
- the name referenced in all technical documentation, API docs, and code

**Convention:** follows the Unix `-ctl` pattern seen in tools like
`systemctl` and `pactl` — short, lowercase, no separators, and immediately
recognizable as a control/management tool to anyone used to Linux server
tooling. This matches the project's primary audience (Linux dedicated
server admins) and keeps the binary name terse for daily CLI use.

**Availability (checked before adoption):**
- crates.io: not registered
- GitHub: no existing repositories with this exact name

## Secondary name: "Steward"

"Steward" is an informal, friendly alias used in prose contexts — README
taglines, blog posts, talks, or anywhere a more human, pronounceable name
helps (e.g. *"swmctl — the Steward of your Steam Workshop mods"*).

It exists because `-ctl`-style names, while standard in sysadmin contexts,
can read as opaque or unwelcoming to people outside that world (e.g. mod
authors building a GUI on top of the library). "Steward" gives those
audiences a warmer entry point without changing the actual package,
binary, or import name.

**Not used for:**
- the crate name
- the binary name
- import paths / module names
- CLI examples in docs (always show `swmctl ...`, not `steward ...`)

**Availability note:** the bare name `steward` is already taken on
crates.io by an unrelated project, so it is not reserved and should not be
assumed to work as a fallback package name if ever needed literally.

## Quick rule of thumb

| Context                          | Use        |
|-----------------------------------|------------|
| Crate / package name              | `swmctl`   |
| Binary / CLI command               | `swmctl`   |
| Code, imports, API references      | `swmctl`   |
| README tagline / marketing copy    | Steward    |
| Casual conversation about the project | Either |
