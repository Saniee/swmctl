# AGENTS.md

Notes for agents and contributors working on `swmctl`: behaviour of the tools
it drives that is not obvious from the code, and the reasoning behind the
choices made because of it. User-facing documentation belongs in `README.md`;
this file is for findings.

## SteamCMD

### Download timeouts (issue #7)

SteamCMD enforces its own download timeout, with no option to raise it, and
gives up on items that take too long — large Workshop mods regularly do:

```
Downloading item 541888371 ...
ERROR! Timeout downloading item 541888371Unloading Steam API...
CWorkThreadPool::~CWorkThreadPool: work complete queue not empty, 179 items discarded.
warning: SteamCMD exited unsuccessfully with status exit status: 10
```

What this means in practice:

- **The timeout ends the whole process, not just the stalled item.** Every item
  still in flight is discarded along with it ("work complete queue not empty").
  Handing a long list of items to one invocation therefore risks losing every
  item's progress to the slowest one. `--batch-size` exists for this: it
  defaults to `1`, so each item gets its own invocation and a stalled item
  cannot take its neighbours down.
- **Exit status 10 means timeout, and it is not fatal.** Items downloaded
  before the timeout are already on disk, so an unsuccessful exit is reported
  per item rather than failing the run. `workshop_download` only returns an
  error when SteamCMD cannot be started at all.
- **A timeout is resumable.** The bytes already fetched stay in
  `steamapps/workshop/downloads/<id>`; a later invocation continues from there
  instead of starting over. `cleanup_steamcmd_dir` must keep leaving non-empty
  staging directories alone, or every retry restarts from zero. A very large
  mod may need several passes, which is what `--max-retries` buys.

### Parsing SteamCMD output

Per-item results are read from SteamCMD's stdout (`parse_item_outcome`), which
is piped and echoed back through so progress still reaches the terminal.

- **Its messages are not reliably newline-terminated.** A timeout runs straight
  into the shutdown chatter — `...item 541888371Unloading Steam API...` — so
  the item ID is parsed as the digits following the marker, never as the rest
  of the line.
- **Progress is redrawn with carriage returns.** Output is split on `\r` as
  well as `\n` for parsing; splitting on newlines alone holds a whole
  download in the buffer and the progress line appears to freeze.
- **Echo bytes before splitting them into lines.** SteamCMD announces an item
  as `Downloading item <id> ...` with no trailing newline, then prints nothing
  at all until that item finishes, and it prompts for a Steam Guard code the
  same way. Holding those bytes back until a line ends makes the run look
  hung: output stops at `Waiting for user info...OK` and everything arrives in
  one burst minutes later. `stream_output` writes and flushes each chunk as it
  arrives and buffers a separate copy for parsing. Under `--quiet` the
  passthrough goes to a sink, so an interactive prompt is invisible by
  construction — `--quiet` belongs in non-interactive runs only.
- **SteamCMD reports no progress for Workshop items.** There is no percentage
  or byte count between the start of an item and its `Success.` line, so a
  large mod is genuinely silent for minutes. swmctl logs the item's name and
  size before handing over so the wait is at least attributable.
- Messages currently recognised: `Success. Downloaded item <id> to ...`,
  `ERROR! Timeout downloading item <id>`, and
  `ERROR! Download item <id> failed (<reason>)`.

### Failure modes that look alike

Several distinct problems surface as an indistinguishable SteamCMD failure, so
each is named explicitly before or after the download rather than left to the
user to guess:

- **Anonymous login against a paid app** fails with `(No Connection)` for every
  item — see `anonymous_failure_hint`.
- **Items Steam will not serve** (hidden, deleted, or published for a different
  app) are warned about up front from the Steam API — see
  `availability_warnings`.
- **Timeouts** are reported as timeouts, not as a missing download — see
  `timeout_hint`.

### Workshop updates: SteamCMD answers with an empty success

SteamCMD remembers every item it downloaded in
`appworkshop_<appid>.acf`. A later `+workshop_download_item` for an item it
considers current can print `Success. Downloaded item <id> to ...` while
delivering nothing — the well-known "SteamCMD does not update Workshop mods"
behaviour that server mod scripts work around by deleting the item's cache
before asking again.

- **swmctl clears SteamCMD's memory before every run** (`forget_steamcmd_items`):
  the app's ACF and the pending items' content directories are removed before
  the attempt loop, so every item swmctl asks for is fetched fresh, as if new.
  This is the community workaround, applied only to the items actually being
  requested — items that are already in place are never re-downloaded.
- **Resumable partials are not touched.** `steamapps/workshop/downloads/<id>`
  keeps its bytes across the forget step, so a timed-out item still resumes
  (see the timeout section above).
- **The manifest records what SteamCMD actually delivered.** Timestamps are
  re-read from the ACF after each attempt and stored only then. A manifest
  built from the pre-run snapshot can lag one version behind — it re-plans
  the same update every run, and SteamCMD keeps answering that request with an
  empty success, which surfaces as a permanently failing run. An API-down
  fallback run therefore converges in one pass instead of looping.

### Relative install directories

SteamCMD resolves a relative `+force_install_dir` against its own installation
directory, while `swmctl` reads the staged files back relative to the working
directory. `--steamcmd-dir` is made absolute before use; left relative, the two
disagree and every item is reported as "SteamCMD did not produce a download".
