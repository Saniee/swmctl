# Tool Workflow

## Overview

`swmctl` fetches every requested Workshop mod fresh through SteamCMD — the
one operation that speaks to the live depots — reconciles the results
against a local manifest, performs deletion checks, and renames/sanitizes
folders so output is safe for restrictive game servers.

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

   **The API's version fields are never trusted as an update signal.** The
   details service serves a cache that can lag real depot changes by days
   (`time_updated` and `hcontent_file` both), so anything decided from them
   silently skips real updates. SteamCMD's `appworkshop_<appid>.acf`
   contributes only a size fallback.

2. **Classify deletions**
   Compare the manifest against the requested list and the fetched metadata,
   and plan only deletions:
   - Managed but not in the requested list — removed with
     `--delete-unrequested`.
   - Reported deleted by the Workshop — removed with `--delete-unavailable`.
   Each check is independently configurable (on/off), and a mod absent from
   the current request is never treated as unavailable.

   Nothing is classified as Download or Update: every requested mod is
   fetched on every run (see below).

3. **Fetch everything via SteamCMD**
   Every requested mod's SteamCMD cache is cleared first — the app's ACF and
   the item's content directory — which is the standard Workshop workaround
   that forces `+workshop_download_item` to download the current depot
   version rather than answer with an empty success. SteamCMD may exit
   successfully while silently omitting individual items, so what actually
   arrived on disk is checked per item and only the missing ones are
   retried, up to `--max-retries`. Partial downloads in
   `steamapps/workshop/downloads` survive the cache clear and resume.

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
  (informational; never gates a fetch)

## Configurable via arguments

- Workshop mod IDs, a newline-delimited mod list, or a Workshop collection ID/URL
- Output/target directory
- Renaming scheme (mod ID vs. sanitized mod name)
- Manifest path (defaults to target directory, overridable)

## File size prioritization

Fetches are ordered largest first — front-loads the longest transfers so
they aren't left stalled at the end of a run.