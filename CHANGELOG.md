# Changelog

All notable changes to cc one are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.2.0] - 2026-09-01

### Changed

- **Dashboard regrouped by semantics** — the usage page re-orders into two adjacent groups: dimension rankings (model distribution > session Top-N > projects > devices; the device card renders only with a multi-device registry, otherwise the project card takes the full row) and time distribution (daily cost > daily requests > turn distribution > duration distribution). The old session/request sections dissolve into four standalone cards (session-ranking, turn-distribution, daily-request-chart, duration-distribution), and every card adopts the equal-height system (h-full + flex-1 content fill, subtitles moved into CardAction) so two cards on a row always bottom-align. (#119)
- **Usage heatmap rebuilt with three window-fitted forms** — the calendar card now fills the full card width by filter window: a day × hour matrix at ≤7 days, a month-calendar wall chart at 8–70 days (rows = weeks, columns = weekdays, day numbers embedded in large cells), and a GitHub-style week grid beyond 70 days (column count adapts; "last year" spans 53 full-width columns). The color scale switches to NONE + four steps of accent progression (color-mix), retiring the --heat-* ink scale. (#119)

### Removed

- **Rhythm radars retired** — the weekly / hourly rhythm cards and their rhythm derivations are gone; the calendar derive module is renamed accordingly. (#119)

### Fixed

- **Calendar cells no longer drift** — zero-filling the daily trend now backfills missing zero days, so cells no longer slide onto the wrong weekday when the window's first day has no records. (#119)

## [2.1.0] - 2026-09-01

### Added

- **Usage trends restacked into an area group + a daily-cost card** — the four-bucket line chart becomes a stacked area group (bucket-colored fills, shared stack, per-bucket legend show/hide kept) with an absolute/share capsule toggle on the card. Share mode pins the Y axis to 0–100%, drops hidden buckets from the denominator, reads tooltip rows as "share · absolute", and shows the day's absolute total and cost in the footer. A new daily-cost card plots per-day total cost as top-labeled bars (label density capped at 14) under a permanent "list-price estimate" note; both charts read the same trend cache — zero extra queries. (#119)
- **Contribution calendar + "Last year" preset** — a calendar-heatmap card: Monday-first week grid on a five-level neutral ink scale (quartile cuts over non-zero days, so any magnitude stays five-level readable), month/weekday labels from the dayjs locale, per-cell native-title readings; the span stretches with the global filter window (wide windows scroll horizontally) and clicking a cell narrows the window to that day. The new "Last year" preset is dynamic (`today − 364d`, never frozen into the query) and lands on all three pages' date-range chips. (#119)
- **KPI sparklines + semicircle composition charts** — the requests / cache-hit / cost / tokens KPI cells each gain a trailing sparkline (hand-drawn SVG with an accent end dot, following the global filter's day-by-day / hour-by-hour granularity; the hit-rate formula is single-sourced into `lib/token-buckets`, shared with the sessions page). Session turns and request durations upgrade from progress bars to 180° semicircle composition charts (joined arc segments + ring-center total + precise readings on the right). (#119)
- **Dimension cards completed + most-expensive-sessions Top-N** — the sessions Top-N card gains a tokens/cost metric switch (the cost setting is "most expensive sessions", ranked like ccusage's session report) and Top rows carry cost / request count / four-bucket breakdowns; model rows gain request counts, project rows gain cache-hit rate (suppressed when there was no cache activity), device rows gain cost. (#119)

### Changed

- **Cost caliber labeled everywhere** — every cost figure now carries a permanent "list-price estimate" note (same estimation family as ccusage calculate / Claude Code's local estimate): the hero footer, project / device / session inline cost segments, the KPI cost cell, and the trend tooltip's cost row, with the KPI total-cost label updated to match. (#119)

## [2.0.3] - 2026-08-28

### Added

- **The packaged binary is named "CC One"** — tauri.conf.json now declares `mainBinaryName` explicitly, so the bundled executable ships as `CC One`, matching the app name, instead of the cargo output name `cc-one` (`CC One.exe` on Windows).

### Fixed

- **Gemini snippet preview no longer over-promises** — an empty-value env key (e.g. `KEY=`) used to be listed as an importable snippet candidate even though extraction could never deliver it (the validator rejects empty values). "Which keys are shareable" now lives in a single decision roster that both the preview candidates and the extraction derive from, so Gemini's empty-value keys no longer appear as candidates.

### Changed

- **`imported` now counts items, not rows** — the sync align report (startup log and settings-page toast) sums what each domain actually imported: new usage rows, imported peer session snapshots, provider entries written, device registrations loaded (wording updated in en/zh; ja was already neutral).
- **Internal refactors, behavior intact** — all fourteen architecture candidates landed, with 40 new tests pinning the invariants on production paths:
  - **Sync** — the per-device JSON sync document (fault-tolerant read, byte-stable write, latest-wins merge) converges on a single `synced_doc` module, adopted by providers / groups / devices and pinned by byte-level golden tests; the sync domain list becomes data — a static DOMAINS table drives pull/push (adding a domain = adding a row).
  - **Providers** — shareable-key decisions are single-sourced (preview candidates and extraction derive from the same roster); the three-state controlled merge (present → replace / absent → withdraw / unlisted → skip) becomes one shared primitive, and Gemini's "the whole settings.json top level is controlled" ruling is made explicit (merge behavior byte-identical); the provider-row UPSERT clock difference is typed as ProviderUpsertPolicy (local clock / author timestamp pass-through).
  - **Database** — the three cross-cutting aspects of dimension aggregation (driver projection, composite-key predicates, bucket decoding) converge in aggregate_sql; usage_records' column list, bind and decode now live in one place with a compile-time length check; "which session columns ride git pushes" is explicit as PushTrack (the favorites track pushes, device-local fields never enter snapshots).
  - **Source parsing** — per-line JSONL folding converges on one shared walker (1-based numbering / blank lines / skipped accounting single-sourced), with the two cursor semantics (claude/codex vs grok) explicit as two policies.
  - **Frontend** — the transcript derive cluster moves to its own module; the token-bucket display roster (color order / key order) replaces four hand copies with a single BUCKET_DISPLAY; the five-dimension filter bar (time · source · model · project · device) becomes a shared FilterBar bound by both toolbars, dropping six date keys from the browser hook; page-edge stepping decisions become pure functions and the rename hook moves into session detail.
  - **Misc** — post-change push sinks into the library domain entry (callers can no longer "change locally without pushing"); track/column magic strings become enums (bindings regenerated); "write then emit sessions_changed" is structurally fixed by two combinators; forget_device's is_self guard sinks into the domain entry.

## [2.0.2] - 2026-08-18

### Fixed

- **Local credentials are no longer lost on sync pull** — importing a peer's Codex / OpenCode providers used to silently overwrite your API keys with the redacted copy (push stripped four key locations, pull restored two); the key locations now live in a single module behind paired strip/restore, and codex auth plus opencode options restore too, pinned by a round-trip property test.
- **Dirty flags can no longer wedge sync** — flags left dirty by a failed or crashed push were never re-cleared behind the "pushed" gate, leaving the device permanently one push behind; clearing is now a single transaction without the gate, proven by a test that fails on the old code.
- **No more half-cleared mid-sync state** — two sequential clears could leave "days cleared, sessions still dirty" when the second failed; a single transaction makes that state structurally impossible, guarded by a SQLITE_BUSY-injection test.
- **All-secret template values no longer vanish from sync** — stripping removed the whole record when every templateValue was a key, dropping the provider on round-trip (the Bedrock AK/SK scenario); restore now recreates it.
- **Claude saves no longer write back unexpanded placeholders** — an unexpanded Bedrock placeholder endpoint was written into the snapshot and rejected by the backend validator; saving now reads from the materialized text.
- **OpenCode edits can no longer swallow half-written JSON** — field write-backs go through the same guarded write as the other four apps, refusing when the JSON text is broken.

### Changed

- **Internal refactors, behavior intact** — all twelve architecture candidates landed: key-location module, per-app live adapter seam (8 dispatch points), five-domain sync pairing, import convergence on a store-level conflict-planning seam, discovery traversal skeleton with explicit gate modes, corrections channel for the collect protocol, FilterSelect sentinel dropdowns, paged-browser controller, provider-form mirror-state removal (1033→640 lines), api.ts split across seven domains with a cache-key dimension registry, turn-nav search state-machine reducer, and the four-item misc cluster (shell state clusters, date-range filter, raw error text, delete confirmation). 50+ new tests, all on production paths.
- **Request-log empty-state copy follows sync state** — with multi-device sync in progress the CTA reads "Syncing…" instead of the generic "Collecting…", matching the sidebar.

## [2.0.1] - 2026-08-18

### Fixed

- **Switching to a login-state provider actually switches** — Codex's controlled identity keys (`model` / `model_providers` / `experimental_bearer_token`) are now withdrawn when the incoming provider doesn't carry them (ADR-0010's "new provider wins"), so third-party → official (ChatGPT login) no longer leaves Codex CLI silently routing to the old vendor while the UI reports switched. Gemini gets the same treatment for its top-level `model` key.
- **No half-written Gemini config** — merging and validation complete before any write; if the `settings.json` step fails, the already-written `.env` is rolled back, and both files get symmetric `.bak` backups.
- **Re-switching the same provider is a no-op everywhere** — Claude and Gemini gain the unchanged-content check Codex / Grok / OpenCode already had: no file rewrite, no `.bak` refresh, no mtime touch. The write transaction (no-change check, backup, atomic write, side-file rollback) now lives in one shared implementation behind all five apps.
- **Missing-key confirmation covers every app** — the pre-switch check used to read Claude's env keys only; it now derives the key set per app (Codex auth + TOML base_url, Gemini env, Grok TOML, OpenCode options), so switching to an incomplete provider asks first instead of writing a dead config.
- **Snippet hardening** — extraction runs through the same validation as saving; Claude / Gemini snippets reject credential keys at the write layer too (not only at the save command); Gemini flat keys (e.g. `{"GEMINI_MODEL":"m"}`) are rejected with a reason instead of silently doing nothing; extracting a snippet no longer force-enables it.
- **Session pagination clamps its limit** — the sessions query clamps `limit` to 1–1000 like the usage query (a zero or huge limit no longer materializes the whole table).
- **OpenCode display names agree between import and preview** — an empty `name` falls back to the provider key on both paths.
- **Saving returns the real app** — the internal shim that hard-coded Claude into `save_provider`'s return value is gone; a saved Codex provider reads back as Codex.
- **Localization** — Japanese regains 17 missing keys (no more English fallbacks), the "enabled" badge wording is consistent across languages, the Codex / Grok snippet hints speak plainly (internal key names only appear in validation errors), and the Grok hint notes custom model profiles ride along with the snippet.
- **UI fixes** — transcript load failures get a retry action, a failed library scan no longer masquerades as "empty folder", library / providers loading states are centered skeletons with a retry on error, provider-row and session-detail controls align their heights, the turn-nav full-text tooltip is no longer clipped, long column headers no longer overflow at the minimum window width, and the snippet editor fills the viewport's remaining height.
- **Codex official presets renamed to be self-explanatory** — "OpenAI (ChatGPT login)" vs "OpenAI (API Key)", distinguishable without reading the notes.

### Changed

- **Internal refactors, behavior intact** — the 2,084-line commands monolith is split into 13 domain modules; repeated frontend logic (source tags, device labels, copy buttons, state actions) and parser / write-layer helpers converge to single owners; the README's preset counts now cover all five pools (59 presets total).

## [2.0.0] - 2026-08-14

### Added

- **Apps & Providers leaves beta** — the nav entry drops its beta flag and every AI CLI gains a real app dimension: per-app presets, per-app form fields (Claude Code / Codex / Gemini CLI / Grok CLI / OpenCode), and live write into each CLI's genuine configuration files. Switching a provider now writes Codex's `config.toml` + `auth.json` via a controlled TOML merge, Gemini CLI's `settings.json` merge plus a wholesale `env` update with an OAuth marker, Grok's `config.toml` live merge, and OpenCode's `opencode.json` additive mode with secrets redacted. Each app's provider list syncs per device, byte-stable — API keys never leave your machine.
- **Import from CC-Switch** — the provider list imports directly from a CC-Switch export, and a unified import dialog treats your local config files, a CC-Switch file, and a CC One backup as equal sources (ADR-0011/0012). Live imports are generalized across every app, and an opencode.json import previews its changes before committing.
- **Native model fetching for Gemini** — the vendor's `/v1beta/models` endpoint joins the OpenAI-compatible fetch; one click fills all five model roles.
- **Common config snippets for Codex, Gemini, and Grok** — a provider-independent snippet (default: hide attribution) merges into controlled fields only, on every switch; toggling it applies immediately without waiting for a save.
- **Transcripts render as markdown** — code blocks and JSON are syntax-highlighted and themed inside every session.
- **Search within a session** — hit-highlighting and jump-to-message navigation across the full transcript, with a numbered turn panel on the side and a ring flash marking the landing spot.
- **Subagent sessions** — Claude Code subagent runs surface as their own rows with an agent-type badge.
- **Token-first model distribution** — the usage chart ranks models by tokens and adds a per-model cache-hit rate.
- **Expandable request-log rows** — click any row to unfold its full details (token/cost breakdown, service tier); all four tables share one pagination bar at 20 rows per page.
- **shadcn calendar date picker** — the native date input is replaced by a proper calendar.
- **Provider-row polish** — semantic category colors, copy-provider, default 1M on create, and an "apply to all" that never clobbers your per-role 1M checks.
- **Theme system** — dark mode gains a three-tier surface ladder (page / modal / input), dedicated tooltip surface tokens, and a brand gradient on the glance card.
- **Rebrand: VaultOne → cc one (归一)** — the app, repo, and packages are now `cc-one`; old config directories migrate automatically on first launch.

### Changed

- **Sidebar restructured** — navigation groups (Observe / Manage), smooth collapse animation, a lighter footer, and group counts that ignore the selected-group filter.
- **Time-range filter rebuilt** — the cross-midnight patch is deleted; the query derives live from the filter (ADR-0008).
- **Session-detail sheet** — header split into identity and stats, prev/next session navigation, bulk collapse, row selection, per-tab empty states, and a two-row toolbar.
- **Modal interactions** — delete confirms get a success-path polish, export options become decision cards, and the pricing editor is matrix-shaped.
- **Narrow-window layout** — filter rows fold via container queries and the pagination bar holds a single line.
- **Tooltips and labels** — 30 native `title` attributes become themed custom tooltips with accessible names; glance-mode copy is standardized across languages.

### Fixed

- **Modal close flicker** — a stray implicit transition polluted Tailwind v4's duration token and flashed one frame on close; the popup no longer carries its own duration.
- **Codex "unknown model" lag** — model context no longer lags token events.
- **Turn-jump highlight drift** — long Virtuoso jumps no longer snap the highlight back one row; a pure turn-nav reducer pins the target (regression-tested).
- **Dashboard refresh after collection** — aggregate store tags invalidate in one place (ADR-0009).
- **Rename no longer resizes columns** — `table-fixed` keeps the name column stable.
- **Delete-confirm busy state** — prop-driven close no longer leaves the button loading.
- **Narrow-window overlap** — the sessions table scrolls horizontally instead of squashing the title column.

### Internal

- **ADR-0008 / 0009 / 0011 / 0012** — time-range derivation, store-tag invalidation, standalone import entry, live-import generalization.
- **One shared TOML parse/cleanup** — the codex/grok live-write duplication collapses into shared helpers.
- **Pure functions with tests** — the turn-nav reducer, transcript highlight extraction (single source of truth), and import text extraction (`extractTitle` / `pairModelNameKeys`) all live in `derive` with tests.

## [1.8.0] - 2026-08-07

### Added

- **Provider management: the provider list + CRUD** — a new **Providers** top-level view (供应商) lists every saved provider (name / category / endpoint / model) with drag-to-reorder, plus an empty state that points to the upcoming preset picker. Create, edit and delete custom providers through a side-panel sheet (name / endpoint / API key); the API key and endpoint live inside the provider's `settingsConfig` snapshot, which the form preserves field-for-field. Data stays in a local SQLite `provider` table for now — switching providers (live write), presets, sync, model mapping and JSON editing land in later tickets.
- **Switching a provider writes to live settings** — the Providers view gains a **Currently using** card (active provider + endpoint + model) and a per-row **Switch** button. Switching merges the provider's snapshot into `~/.claude/settings.json` *controlled fields only* (`env` + `includeCoAuthoredBy` / `attribution` / `effortLevel` / `enabledPlugins` / `skipWebFetchPreflight`), leaving everything else — hooks, MCP servers, permissions, plugins, model — untouched on disk; the previous live file is backed up to `settings.json.bak` (single-file overwrite) before an atomic temp-file rename. The active provider is remembered in the local `config.json` and survives restarts.
- **18 built-in provider presets** — the Providers view now ships a preset picker (searchable, grouped by category) with 18 templates: Claude Official + AWS Bedrock ×2, eleven domestic vendors (Kimi, DeepSeek, GLM, Volcengine, DouBao, Baidu, Alibaba, StepFun, MiniMax, MiMo …) and four popular aggregators (SiliconFlow, OpenRouter, ModelScope, Novita). Picking one pre-fills the new-provider form with its endpoint / auth / model mapping; the preset itself is never mutated, so you can edit and save as a custom provider. Presets ship with the app — they never sync.
- **Raw settings.json editor in the provider form** — the provider sheet ends with a CodeMirror 6 JSON editor over the full snapshot: syntax highlighting, red underlines for parse errors or a non-object top level, a format button (2-space), and dark/light theme following. The JSON is the single source of truth — editing it re-derives the form's endpoint / API key, editing a field merges back, and an invalid snapshot blocks saving instead of silently discarding your edit.
- **Provider structure syncs across devices** — the provider list now rides the existing push/pull orchestration as a per-device `providers.json` under `repo/data/<deviceId>/`: every push materializes this device's file from the local database (byte-stable, so an unchanged store pushes nothing), every pull merges all peers' files back by id with the newest `updated_at` winning. API keys never enter the sync file — the four secret env keys (`ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_API_KEY` / the AWS key pair) are stripped before writing, and each device fills its own keys locally; importing a peer's key-stripped copy merges the local row's keys back in, so a keyless copy can never overwrite a filled one. The active provider stays local in `config.json` — activation never syncs.
- **Model role mapping with 1M marker** — the provider form gains a five-role model map (Sonnet / Opus / Fable / Haiku / Subagent), each role with a display name, a request model, and a 1M-context checkbox (Haiku strips the marker automatically — it does not support 1M). Roles missing from a provider's config are back-filled when the provider loads, and the "apply to all" action spreads any filled-in model across every role in one click.
- **Fetch model list** — the model-map section's "fetch models" button pulls the vendor's OpenAI-compatible `/v1/models` endpoint from the backend (the webview's fetch would hit CORS): an explicit `modelsUrl` override wins, a base URL ending in a version segment appends `/models` instead of a second `/v1`, known Anthropic-compatible subpaths are stripped, and the candidates deduplicate to at most three tried in order. The returned model ids fill all five roles in one click; failures surface as bucketed toasts (auth failed / endpoint closed / timeout / unsupported format).
- **Common config snippet** — a provider-independent settings snippet (default `{"includeCoAuthoredBy": false}` to hide attribution) lives in the provider form: hand-write it, tick it enabled, and every switch merges it into the controlled fields only — `env` merges deep by key with the provider's own values winning, other controlled switches are filled only when the provider hasn't set them, and non-controlled fields are never touched by it.
- **Auth field toggle + template variables** — the form's auth field switches between `ANTHROPIC_AUTH_TOKEN` and `ANTHROPIC_API_KEY`, moving the entered value across on toggle. Bedrock presets additionally surface template variables (region / access key / secret key) that are substituted recursively into `${VAR}` placeholders before saving; an unfilled variable blocks saving instead of writing a broken config.
- **Export / import providers** — the full provider list can be exported as one JSON document (optionally including API keys) and imported back on any device: merge mode dedupes by id without touching existing rows, overwrite mode lets the imported copy win. A purely manual migration path — import goes to the local database only and never through git sync.
- **Jump between user turns** — opening a session now also mounts a slim panel beside the transcript listing every user message (width of ~16 Chinese characters, overflowing rows ellipsize). Click a row to jump straight to that message, the row for the message you're reading stays highlighted, and hovering any row shows its full text. The two panels open and close together.
- **Jump flashes the target** — clicking a turn in the nav panel rings the message it lands on with three quick pulses of the accent ring, so the eye arrives exactly where the scroll did.

### Changed

- **Source parsers renamed to `SourceParser`** — the log-parser module (`providers/`) becomes `source_parser/`, freeing the word "Provider" for the upcoming vendor-management feature. Pure rename, no behavior change.
- **Request-log table drops its duplicate Source columns** — the two columns both derived from the same `source` field (a readable name with a stale "(Session)" suffix, and the raw tag) merge into one Source column: the readable name, with the raw tag on hover. The misleading suffix is gone — each row is a single API call, not a session.

- **Long sessions open instantly** — the transcript now renders through a virtualized list (react-virtuoso), keeping only the messages near the viewport in the DOM. A multi-thousand-message session no longer stalls while every row is laid out at once, and scrolling stays smooth regardless of length.
- **Message header rows mirror cleanly** — on both the assistant and user voices the toolbar (collapse chevron + copy) and the time/model block are pinned to opposite ends of the bubble, so a short user message no longer crams the two together against the edge.

### Fixed

- **Turn jumps no longer overshoot** — jumping scrolled smoothly past the target to an estimated position and glided back, visibly running to the bottom of the transcript before springing to the message. Jumps now land instantly, with the ring flash carrying the feedback.
- **The 840×600 minimum window actually sticks now** — the lightweight-mode restore commands were re-applying an old 720×520 floor via `set_min_size`, silently overriding the raised minimum. The dashboard no longer restores undersized after a glance-card round-trip.
- **Sessions table no longer overlaps at narrow widths** — the seven fixed columns squeezed the title column to zero and spilled the header into the Project column. The table now keeps a readable width and scrolls horizontally instead.

## [1.7.1] - 2026-08-06

### Added

- **Update checks on launch** — cc one probes GitHub Releases for a newer version on every launch and re-checks every 6 hours while it stays open; the footer shows when one is available.

- **Drag to reorder groups** — grab any custom group in the sessions sidebar and drag it to a new spot; the list keeps your order, and on the Favorites tab that order follows the group to every other device. Order lives per track — a `position` column in the local database for local groups, a `position` field in the synced-groups artifact for the Favorites tab — and new groups always land at the end. The All / Ungrouped rows stay pinned.

### Changed

- **Roomier minimum window** — the main window's floor rises to 840×600 so the session list never cramps against the group sidebar; the session card title now ellipsizes on narrow windows.

- **Cleaner group sidebar** — group rows drop their folder icons and the sidebar narrows, giving the session table more room.

- **Session detail sheet polish** — the title's rename trigger is just the title plus a pencil icon (blank space no longer starts editing), the source renders as a tag chip, the favorite button matches the group picker's height, and chat bubbles cap at 80% width on narrow windows while keeping the 72ch line-length cap wide.

## [1.7.0] - 2026-08-06

### Added

- **Copy buttons on every message** — hover (or keyboard-focus) a message row in a session's transcript and a copy button appears; clicking it puts the raw text on your clipboard with a momentary checkmark.
- **Window-tracking detail sheet** — the session detail panel now spans ~70% of the window (`100vw - 32rem`), leaving the sidebar and the title column of the list visible behind it, so you always know which session is open.

### Changed

- **Three-voice transcript layout** — assistant messages float left, user messages float right as a mirrored bubble (corner cut toward the edge), tool and system rows span the full width in the middle as the workbench. Position alone tells you who spoke.
- **Tighter message headers** — icon, time, and model badge share one line; on user messages the group mirrors to the bubble's right edge so the time sits flush against it, aligned with the edge of the sheet.
- **Collapsible messages, quiet tool rows** — every message collapses on click (expanded by default); tool rows collapse to their tool name by default, and tool output is styled as a monospace code panel.
- **Aligned group counts** — the group sidebar's counts always sit flush right, matching the plain All / Ungrouped rows; the edit menu slides the count aside on hover instead of occupying space at rest.

### Fixed

- **Tool rows cleaned up** — the duplicated icon pair is gone, and a tool without a name truncates its content's first line instead of stretching the row.

## [1.6.0] - 2026-08-06

### Added

- **Sessions browser** — a new side-navigation entry that turns the raw session logs your AI CLIs write into a browsable, searchable history. Every session sits under its project directory with its full transcript, per-session token breakdown, and cost (computed live from the usage records — nothing is double-stored). Filter by time range, source, model, and device; search titles and paths; rename a session in place; open any session in a detail panel with a color-coded transcript (assistant model badges, distinct user turns, collapsible tool calls).
- **Two tabs, two ways to organize** — a **Local** tab lists every session collected on this machine, sorted into private groups that never leave it; a **Favorites** tab lists the sessions you favorited across all devices, sorted into synced groups shared everywhere, each entry marked with its source device. The same session can sit in different groups in each tab.
- **Favorites sync across devices** — starring a session publishes its transcript and synced-group placement through your sync repo; unstarring removes it everywhere. Only favorited sessions ever leave your machine — everything else stays local.
- **Transcripts for every session, instantly** — all conversation text is stored in the local database at collect time, so any session — favorited or not — opens its full transcript without re-reading a log file that may still be mid-write.
- **Faster collection** — a 5-second collect interval joins the scheduler presets for near-real-time dashboards.

### Changed

- **Sync rebuilt on dirty-day tracking** — collect now writes the local store only and marks each affected day dirty in the same transaction; push regenerates that day's artifact deterministically from the store (byte-stable, not append) and clears the dirty marks only after the push lands. The old JSONL-first write order, the with-own-data snapshot/restore protection, and the artifact-gap reconciler are gone — one write path, so two devices can never disagree on a file's content.
- **Session snapshots are derived, not written** — a favorited session's synced snapshot is recomputed from the store on push; a session whose source log has vanished is reclaimed automatically on every device (local rows and synced snapshot both removed).
- **Sync scope narrowed** — the optional sync repo now carries usage, favorited sessions, and library files only. The Sync-config feature (syncing app settings through the repo) shipped in 1.5.x is removed; settings are per-device again.
- **Smarter project grouping** — a session's project directory is derived from the most frequent working directory across its events (the mode, not the first entry), so a session that starts inside a subdirectory is grouped under the real project root.

### Fixed

- **Session collection robustness** — a log file that ends mid-character (a session still being written, e.g. in Chinese) is read lossily instead of dropped, so an in-progress session no longer loses its whole file; the scan cursor advances correctly, titles follow renames, and missing sessions are recovered.
- **Unfavorited sessions can now be viewed** — previously opening one asked you to favorite it first, because its transcript lived only in the favorited snapshot; the transcript now comes from the local database, so every session opens, favorited or not.
- **Favorite state no longer flickers** — a refetch no longer falls back to a stale snapshot, so a session you just favorited stays favorited, and the star icon matches the table row.
- **Session layout** — the "New group" button stays visible and the group sidebar no longer overflows its area.

### Internal

- **The largest architecture batch yet** — the db god-module was split into domain modules (`store_*`), 22 commands migrated out of domain modules into `commands.rs`, and single sources of truth were consolidated across the board (model normalization, price matching, device registry, provider parsing, UI formatting, date-range chips); the sync god-module was split into git primitives + flow orchestration, and a review pass cleaned up remaining drift. No user-visible change — the dashboard's code is measurably simpler to extend.

## [1.5.1] - 2026-07-31

### Fixed

- **Sync self-heals when a local commit duplicates a remote patch** — the 1.5.0 rebase self-heal aborted if a local commit duplicated a patch already on the remote (e.g. the same device-cleanup run on two machines), surfacing `rebase onto remote tip would conflict ... this patch has already been applied` and leaving the device stuck diverged again. `pull` now drops already-applied commits during the rebase and continues, so the divergence self-heals instead of stalling.
- **Usage rows no longer silently drop out of sync** — ingest wrote SQLite before the JSONL Artifact and treated the Artifact as a mere backup, swallowing append errors. A row that hit the DB but missed the Artifact (a transient append failure, or residue from ≤1.3.x) was then locked out forever — the ledger dedup silenced every later collect, so peers pulling the Artifact never saw it (one device showed ~24M tokens while a peer showed ~30M under the same filter). Ingest now appends the JSONL Artifact first and idempotently, and propagates append errors, so a failed append leaves the scan cursor untouched and the next collect re-parses the same source lines from the AI CLI logs. A new pre-collect reconcile also clears the cursors when the store holds rows the Artifact is missing, so a single rescan backfills pre-existing gaps — devices converge without manual repair.

## [1.5.0] - 2026-07-31

### Added

- **Grok CLI** — reads token usage from Grok Build's session logs, making it the fifth supported AI CLI (overlooked in this release's notes; documented retroactively).

### Changed

- **Settings layout** — the standalone Cloud-config section merges into Sync: the "Sync config" button and conflict resolver now sit beside "Sync now" under a single Sync card. Section order is now General / This machine / Devices / Sync / Maintenance, and the "Sync cloud config" button reads "Sync config".

### Fixed

- **Sync self-heals after a diverged push** — when a device lost a push race (a peer pushed between its own last pull and push), every "Sync now" / "Sync config" failed with `pull would diverge on 'main'; refusing to auto-merge` and could never recover on its own, leaving the dashboard on stale pulled data. `pull` now rebases the device's local-only commits onto the remote tip and pushes, auto-healing the divergence. Device isolation (`data/<deviceId>/`) keeps the rebase conflict-free, so both devices' data survive on the remote — a soft/reset-only fix would have replayed the local tree verbatim and clobbered the peer's data.
- **Trend chart for a single past day** — selecting a single past day (e.g. 2026-07-30 → 2026-07-30) collapsed the usage trend to a flat zero line: the chart zero-filled *today's* hours instead of the selected day, so the real records never matched. It now fills the selected day's full 24h axis (00:00 → 23:00; the current day stops at the current hour).

## [1.4.0] - 2026-07-30

### Added

- **Library — per-device file relay** — drag files or directories onto the window to upload (= a push into the device's subtree of the sync repo), drill into nested directories (upload, export, and single-file download all work at every depth), preview a file in-app (images fit-to-width with ctrl+wheel zoom; everything else in a sandboxed iframe), and export to a path of your choice. Upload is the only automatic direction — export stays manual and never writes into an AI tool's own config dir. Same-name same-kind overwrites (git history is the safety net); same-name different-kind is rejected. Forgetting a peer offers to migrate its files into yours under `from-<peer>/`, or delete them.

### Changed

- **Picker labels** — the logs and dashboard device / source / model dropdowns drop the dim `.`-prefixed placeholder; the "all" option now reads its full label (All devices / All sources / All models), and the date-range chip collapses same-day ranges and no longer wraps in the narrow dashboard column.

### Fixed

- **Usage filter** — the dashboard filter now persists the time-range preset (today / 7d / 30d / all / custom) instead of concrete dates, so a "today" selection no longer reads back as "yesterday" after midnight. Legacy rows without a preset fall back to "custom" with their literal dates.
- **Sync dedup** — `usage_records` / `turn_durations` / `ledger` used `uuid` as a global primary key, so the same source event replayed under two device ids collapsed into one row and could attribute one device's data to another. Dedup is now keyed on `(uuid, device_id)` (existing single-column PK migrated to a composite key, row counts preserved), and binding a sync repo pulls immediately so peer devices appear without a restart.
- **Window minimum size** — the morphing main window could restore or snapshot at the glance card's small size. The full dashboard now enforces a 720×520 minimum (the lightweight dock clears it; full restore re-applies it), and a stale sub-minimum rect self-heals on the next restore.

## [1.3.1] - 2026-07-28

### Changed

- **Source filter visibility** — the Source dropdown now renders whenever any source data exists, so a single-source user still sees the filter (previously it required ≥2 collected sources).
- **Filter chip sizing** — the logs control bar now sizes its filter chips by typical content (model `w-48`, source `w-40`, device `w-36`) instead of a uniform width; the dashboard card column stays uniform at `w-36`.

### Fixed

- **Release completeness** — `v1.3.0` was tagged two commits early, so the Source-filter and chip-sizing changes above never shipped in the 1.3.0 installers. `v1.3.1` tags the current `main` to include them.

## [1.3.0] - 2026-07-28

### Added

- **Multi-CLI usage collection** — usage collection now spans four AI CLIs that write to local logs: Claude Code (`~/.claude`), Codex (`~/.codex`), Gemini CLI (`~/.gemini`), and OpenCode (SQLite). Each source's token semantics are normalized into the same four-bucket model: Codex's cache-inclusive input becomes fresh input, Gemini's `thoughts` fold into output, OpenCode's `cache.write` maps to cache creation. Claude Code dedup now picks the best message-id snapshot (one with `stop_reason` set, else the largest output), so message_start snapshots no longer freeze and undercount output. Seed pricing added for gpt-5.5 / 5.4 / 5.2 (prefix-fallback covers `-codex` variants) and DeepSeek v3.x.
- **Device-scoped usage tracking** — filter the dashboard, the expanded lightweight card, and the tucked mini-bar by device. A unified device picker drives all three windows; the tucked bar's hover drawer opens on hover and lists devices on click.
- **Device lifecycle** — forget a peer device locally (clears its rows and artifact dir); stale peers with no git presence auto-clear within ~30s on both sync pull and the collect path, while a still-active peer self-heals on the next sync. Recent requests and the logs' Device column now show device names.
- **Persistent usage filter** — the time-range / model / device-scope filter survives app restarts.
- **Per-shape window geometry** — full, expanded, and tucked each remember their own placement and state, so switching shapes restores the last position instead of resetting.
- **System-proxy sync** — libgit2 push/fetch/clone/connect now follows the OS system proxy (env vars, then the Windows registry), so Synced-mode clients behind Clash/Mihomo or a corporate gateway no longer time out silently. Proxy changes apply on the next sync — nothing is cached.

### Changed

- **UI polish** — title bar raised to 36px; mid-window device selector tightened. Token columns carry a language-neutral `tok` unit in the header. Select popovers auto-fit content (never narrower than the trigger) and open top-aligned, fixing the jumping model dropdown.

### Fixed

- **Linux tray** — added a "Show" entry so the window can be restored from the tray on Linux, where the libappindicator/SNI backend never emits tray click events and left-clicking was a no-op. Windows/macOS left-click restore is unchanged.
- **Pricing seed** — a malformed seed literal now panics at startup instead of silently returning 0 and skewing every cost calculation.
- **Today trend** — the "today" trend now spans the full day instead of stopping at the current hour.
- **Request log** — switching the filter resets the log page, so you don't land on a stale, out-of-range page.

## [1.2.0] - 2026-07-24

### Added

- **Lightweight glance mode** — the main window morphs into a small, always-on-top "today" snapshot docked to the right screen edge. Two shapes reachable from one another: a tucked mini-bar that always shows today's token total, and an expanded card mirroring the dashboard's anchor. Switch full ⇄ expanded ⇄ tucked from any shape.
- **Multi-skin theming** — recolor the accent and chart palette across five skins (Neutral, Sage, Azure, Crimson, Mauve); Neutral (greyscale chrome) is the new default. Per-device, never synced.

### Changed

- **Usage trend** — the trend chart is now multi-line with data points instead of a single line, so each metric reads on its own.

### Fixed

- **Lightweight mode** — the entire tucked bar is draggable now (not just a tiny corner grip), and a press still distinguishes click-to-expand from drag-to-move.

## [1.1.0] - 2026-07-23

### Added

- **Auto-update** — check for new versions on launch (throttled to once per 24h) or manually from Settings; download and install signed installers straight from GitHub Releases, with Ed25519 signature verification and one-click relaunch. Distributed entirely through GitHub — no self-hosted server. On updater failure, a manual fallback opens the Releases page.
- **Display language** — switch the UI between English, 简体中文, and 日本語.

### Fixed

- **Lightweight mode** — edge-flush the tucked peek icon and smooth out the diagonal reveal animation.

## [1.0.0] - 2026-07-23

First public, open-source release.

### Added

- **Dashboard** — four-bucket token consumption (input / output / cache creation / cache read), cache-hit rate (`cache_read / (input + cache_creation + cache_read)`), total requests and total cost (USD, frozen at collection), dual-axis token-vs-cost usage trends, per-call request log (model, token breakdown, cost, turn duration, `stop_reason` / `service_tier` chips), and per-turn cost and wall-clock views.
- **Collection** — read-only parsing of Claude Code session logs (source logs are never modified), cursor-based incremental scan, tray-resident background scheduler. Pluggable provider architecture (Claude Code today, more planned).
- **Sync (optional)** — Standalone mode (full dashboard, zero network) and Synced mode (align usage across devices through a GitHub repository you own); plain-text artifacts partitioned by device and date (`data/<device>/usage-YYYY-MM-DD.jsonl`).
- **Cost & pricing** — editable per-model pricing overrides; rebill for records that had no price when collected, without re-costing existing history.
- **Experience** — lightweight glance mode (edge-tuck + hover-to-peek today's usage), custom title bar, light / dark theme, local-first and private by default.
- **Packaging** — cross-platform installers for Windows, macOS (Apple Silicon), and Linux, built automatically on tag push via GitHub Actions.

### Known limitations

- **macOS**: Apple Silicon (arm64) only; builds are unsigned — right-click → **Open** on first launch (or `xattr -dr com.apple.quarantine /Applications/cc one.app`). Intel Mac users can build from source.
- **Providers**: Claude Code only; additional providers (Codex, Cursor, …) are planned.

[2.2.0]: https://github.com/Buktal/cc-one/releases/tag/v2.2.0
[2.1.0]: https://github.com/Buktal/cc-one/releases/tag/v2.1.0
[2.0.3]: https://github.com/Buktal/cc-one/releases/tag/v2.0.3
[2.0.2]: https://github.com/Buktal/cc-one/releases/tag/v2.0.2
[2.0.1]: https://github.com/Buktal/cc-one/releases/tag/v2.0.1
[2.0.0]: https://github.com/Buktal/cc-one/releases/tag/v2.0.0
[1.8.0]: https://github.com/Buktal/cc-one/releases/tag/v1.8.0
[1.7.1]: https://github.com/Buktal/cc-one/releases/tag/v1.7.1
[1.7.0]: https://github.com/Buktal/cc-one/releases/tag/v1.7.0
[1.6.0]: https://github.com/Buktal/cc-one/releases/tag/v1.6.0
[1.5.1]: https://github.com/Buktal/cc-one/releases/tag/v1.5.1
[1.5.0]: https://github.com/Buktal/cc-one/releases/tag/v1.5.0
[1.4.0]: https://github.com/Buktal/cc-one/releases/tag/v1.4.0
[1.3.1]: https://github.com/Buktal/cc-one/releases/tag/v1.3.1
[1.3.0]: https://github.com/Buktal/cc-one/releases/tag/v1.3.0
[1.2.0]: https://github.com/Buktal/cc-one/releases/tag/v1.2.0
[1.1.0]: https://github.com/Buktal/cc-one/releases/tag/v1.1.0
[1.0.0]: https://github.com/Buktal/cc-one/releases/tag/v1.0.0
