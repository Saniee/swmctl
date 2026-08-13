# Tool Workflow

## Overview

`swmctl` uses SteamCMD as its source of truth for Steam Workshop mod
metadata, reconciles that against a local manifest, and performs the
minimum set of download/update/delete actions needed to bring a mod
directory in sync — renaming and sanitizing filenames along the way so
output is safe for restrictive game servers.

## Core workflow

1. **Gather info via SteamCMD**
For a given mod ID, text file of mod IDs, or Steam Workshop collection, query SteamCMD for current
   Workshop metadata (title, size, last-updated timestamp, etc.).

2. **Diff against the manifest**
   Compare the fetched info against the existing local manifest (if one
   exists) and classify each mod as one of:
   - **Download** — not present locally
   - **Update** — present, but remote version is newer
   - **Delete** — present locally, and either no longer in the requested
     list, and/or (if enabled) no longer available on the Workshop.
     Each check is independently configurable (on/off).

3. **Execute via SteamCMD**
   For anything flagged download/update, fetch the mod through SteamCMD.

4. **Place and rename**
   Once a download finishes, move the mod into:
   - the directory the tool was invoked in (default), or
   - a directory passed via argument

   The mod folder is renamed according to a user-selectable scheme
   (e.g. raw mod ID vs. actual mod name), also set via argument.

5. **Sanitize the name**
   Renaming must support strict-filesystem/game targets (e.g. Arma):
   no spaces, no special characters, no diacritics (e.g. `ä` → `a`).
   This sanitization applies whenever renaming to the "actual name"
   scheme — the mod-ID scheme is unaffected since IDs are already safe.

6. **Update the manifest**
   After all actions complete, create or update the manifest in the
   target directory with the current state of every managed mod.

## Manifest format

JSON, minified (no pretty-printing). Of the plain-text options considered
(JSON/TOML/YAML), JSON is the most size-efficient for this data shape —
a flat array of same-shaped mod records — since it carries no repeated
indentation or block-scalar overhead the way YAML does, and no repeated
table-header syntax the way TOML does for arrays of objects. If manifest
size ever becomes a real concern at scale, optional gzip compression is
a straightforward follow-up without changing the schema.

## Manifest contents (per mod)

- Date downloaded / last updated
- Mod name
- Mod ID
- File size (used to prioritize ordering in subsequent download runs)

## Configurable via arguments

- Workshop mod IDs, a newline-delimited mod list, or a Workshop collection ID/URL
- Output/target directory
- Renaming scheme (mod ID vs. sanitized mod name)
- Manifest path (defaults to target directory, overridable)

## File size prioritization

When multiple mods are queued for download/update, largest first —
front-loads the longest transfers so they aren't left stalled at the end
of a run.
