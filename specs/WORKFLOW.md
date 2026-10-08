# Tool Workflow

## Overview

`swmctl` asks SteamCMD — the one operation that speaks to the live depots —
about every requested Workshop mod, reconciles the results against a local
manifest, performs deletion checks, and renames/sanitizes folders so output is
safe for restrictive game servers.

## Core workflow

1. **Gather metadata**
   For a given mod ID, text file of mod IDs, or Steam Workshop collection,
   gather Workshop metadata (title, size, availability).

   The public Steam Web API (`GetPublishedFileDetails`) supplies titles,
   sizes and availability, queried anonymously and in batches. The API is
   read-only and is never used for authentication — downloads remain
   entirely SteamCMD's responsibility. Because the request is anonymous,
   private and login-gated items are invisible to it; such items are treated
   as *unknown*, never as unavailable, so they are never deleted.

   **The API's version fields decide which mods are fetched.** A mod is
   updated when `hcontent_file` differs from the handle recorded in the
   manifest, or, without a handle, when `time_updated` is newer than the
   recorded one. A mod the API says nothing about is checked through SteamCMD.
   SteamCMD's `appworkshop_<appid>.acf` contributes only a size fallback. Its
   timestamps come from SteamCMD's own clock, so they are never recorded as
   versions.

2. **Classify downloads, updates and deletions**
   Compare the manifest against the requested list and the fetched metadata:
   - Requested but not in the manifest: Download.
   - Steam reports a different version than the one recorded, or the
     recorded directory is missing: Update.
   - Managed but not in the requested list — removed with
     `--delete-unrequested`.
   - Reported deleted by the Workshop — removed with `--delete-unavailable`.
   Each check is independently configurable (on/off), and a mod absent from
   the current request is never treated as unavailable.

3. **Fetch via SteamCMD**
   Downloads and updates are handed to SteamCMD, largest first. With
   `--check-all`, the remaining requested mods follow, so a lagging API cannot
   hide an update. SteamCMD's own version check compares its cached manifest
   against the live depot: changed or missing items are downloaded, and
   current ones get an empty success that swmctl reads as
   `unchanged — left in place` (recording Steam's reported version so the mod
   is not flagged again). SteamCMD may exit successfully while
   silently omitting individual items, so what actually arrived on disk is
   checked per item and only the missing ones are retried, up to
   `--max-retries`. `--force-refresh` clears the app's ACF and each item's
   content directory first, forcing a fresh fetch of the current version
   regardless of the cache. Partial downloads in `steamapps/workshop/downloads`
   survive and resume.

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
   Create or update the manifest in the target directory with the current
   state of every managed mod. The manifest is written even when some mods
   failed, so work already done survives a failed run, and it is written
   atomically so an interrupted run cannot truncate it.

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
- File size (used to order fetches)
- Steam content handle and update timestamp when the API supplied them
  (compared against Steam's on the next run to decide updates)

## Configurable via arguments

- Workshop mod IDs, a newline-delimited mod list, or a Workshop collection ID/URL
- Output/target directory
- Renaming scheme (mod ID vs. sanitized mod name)
- Manifest path (defaults to target directory, overridable)

## File size prioritization

Fetches are ordered largest first — front-loads the longest transfers so
they aren't left stalled at the end of a run.