# sorts

A simple font manager CLI for macOS, similar to RightFont or FontBase. It is written in Rust.

The special feature: a team shares one `Fonts` folder (on Google Drive), and each user keeps their own personal settings for which fonts are enabled. Collections are stored inside the shared folder so the whole team sees the same collections.

**This repo is public.** Never write facts about the user's machine or the team's Fonts folder (paths, names, file lists, counts) into tracked files. If private notes are needed, put them in `CLAUDE.local.md` or `private/`; both are in `.gitignore`.

## Status

- Roadmap step 2 is done: `init`, `sync`, `status`, settings, scanner, active set, `formatOrder`, collection loading, CoreText activator.
- Roadmap step 3 is done: the interactive list (`sorts` with no command), with search and toggles.
- Roadmap step 4 is done: create, rename, delete, enable/disable, and edit collections in the TUI.
- Collection picker (`+` in the Fonts tab) is done.
- Settings tab (after Collections) is done: change `formatOrder` in the TUI.
- `sorts fix` + Missing tab is done: the user replaces or removes entries that point to moved fonts.
- Scan progress is done: online-only files are downloaded by the scan, and stderr shows the progress (see "Online-only files").
- Roadmap step 5 is prepared: public repo + public tap `designsystemsinternational/homebrew-tap`, formula builds from source. See `docs/RELEASING.md`. The user does all Git work (repos, tags, pushes) themselves.
- The user tries the CLI from source (`cargo run --release --` / `cargo install --path .`, see README "Run from source") and gives feedback.
- The build needs only Rust and the Xcode Command Line Tools, not Xcode.

## Core principles

- **macOS only.**
- **The Fonts folder is the source of truth.** There is no cache and no import step. To add a font, a user puts the file in the Fonts folder.
- **Personal settings live outside the Fonts folder.** They are never shared.
- **Collections live inside the Fonts folder.** They sync to the team with the folder.
- **The CLI also works without Google Drive.** The root can be any local folder.
- **Never write to the Fonts folder except for collection files** in `.sorts/collections/`. Never move, rename, or delete font files from the CLI.
- **Test-driven development.** See below.

## Code layout

```
src/
  main.rs        CLI parsing (clap) and output only. Picks the activator.
  app.rs         Commands: init, sync, status. `Context::load` = settings + scan + collections.
  scan.rs        Walks the Fonts root, reads names with ttf-parser (parallel with rayon). Detects online-only files.
  progress.rs    The scan progress line on stderr (scanning count, downloads, offline tip).
  entry.rs       Folder/font entry matching ("A/" matches everything under A/).
  active.rs      Active set calculation + formatOrder selection.
  collections.rs Collection files: load, slug, create, rename, save, delete.
  settings.rs    Settings (config.json) and State (state.json) in the config folder.
  sync.rs        Reconciliation plan + apply.
  activator.rs   FontActivator trait, FakeActivator, FileFakeActivator, CoreTextActivator.
  testing.rs     Generates complete TrueType test fonts (also used by tests/).
  typefaces.rs   Groups the fonts of a list of entries by family (typeface name + style count).
  tree.rs        Folder tree for the TUI: rows (expand + search filter) and set_enabled (toggle → entries).
  tui/mod.rs     App: all TUI state and key handling. Tested with key events.
  tui/view.rs    Drawing. Tested with ratatui TestBackend + insta snapshots (tui/snapshots/).
  tui/terminal.rs The event loop (thin, not tested).
tests/
  cli.rs         End-to-end tests of the binary (temp folders + FileFakeActivator).
  coretext.rs    Real CoreText tests, process scope, #[ignore].
```

- Env vars: `SORTS_CONFIG_DIR` overrides the settings folder. `SORTS_FAKE_ACTIVATOR=<file>` makes the binary use the file-based fake activator (end-to-end tests only). `SORTS_FAKE_DATALESS=<text>` makes font IDs that contain the text count as online only (end-to-end tests only).
- Formatting: `rustfmt.toml` sets `max_width = 120`.
- Snapshots: after an intended UI change, run `INSTA_UPDATE=always cargo test tui::view`, then read the changed `.snap` files and check them before you keep them.
- The test fonts from `src/testing.rs` are accepted by both ttf-parser and CoreText, so no licensed fixture fonts are needed.
- To try the CLI against a real Fonts folder without touching the real settings, set `SORTS_CONFIG_DIR` to a temp folder (and `SORTS_FAKE_ACTIVATOR` to a temp file). Do not register fonts on the real system unless the user asks.

## Test-driven development

The user does not want to test things manually. Every feature is built test-first.

Run all tests: `cargo test && cargo test -- --ignored`.

### Workflow

1. Write a failing test that describes the behavior.
2. Run it and see it fail for the right reason.
3. Write the smallest code that makes it pass.
4. Refactor with the tests green.
5. `cargo test`, `cargo clippy -- -D warnings`, and `cargo fmt --check` must pass before a task is done.

Bug fixes start with a test that reproduces the bug.

### Architecture for testability

- Keep the logic in a library crate (`src/lib.rs` + modules) and keep `src/main.rs` thin.
- Pure logic with no I/O where possible: slug, folder/font ID expansion, active set, `formatOrder` selection, reconciliation diff (desired vs. registered → register list + unregister list), collection create/rename/delete rules, TUI state transitions.
- Side effects behind traits, with an in-memory fake for tests:
  - `FontActivator` (register, unregister, is_registered). Real: CoreText. Fake: a `HashSet` of paths that can also simulate errors.
- File system code (scanner, settings, collection files) is tested against temporary folders (`tempfile`) built by the test.
- Paths are injected: the Fonts root, the settings folder, and the activator are parameters. Nothing reads `~/Library/...` or the real Fonts folder directly in library code. The binary can take the settings folder from an env var (`SORTS_CONFIG_DIR`) so end-to-end tests can isolate it.

### Test layers

- **Unit tests** next to the code (`#[cfg(test)] mod tests`).
- **TUI tests:** drive the app state with key events and assert the state. Render with ratatui's `TestBackend` and compare snapshots with `insta`.
- **End-to-end CLI tests** in `tests/` with `assert_cmd`: a temp Fonts root with fixture fonts, a temp settings folder, and the fake activator.
- **CoreText integration tests** (`#[ignore]` by default, run with `cargo test -- --ignored`): use `CTFontManagerScope::Process` so nothing persists on the machine, and use fixture fonts only.

### Rules for tests

- Use generated fonts from `sorts::testing` (`write_font`, `write_collection`, `write_file` for unreadable files). Do not add licensed font files to the repo.
- Tests never touch the real Fonts folder, the real settings file, or the persistent CoreText scope.

## Data model

### Shared Fonts folder

```
Fonts/
  <Foundry>/.../<font files>
  .sorts/
    collections/
      <collection-slug>.json   # e.g. studio-work.json
```

- The team folder is organized by foundry. Some foundries have deep subfolders, some have all files in one folder.
- A **font ID** is the path of the file relative to the Fonts root, e.g. `Foundry A/Sans/Sans-Regular.otf`.
- A **folder ID** is a relative path that ends with `/`, e.g. `Foundry A/Sans/`. A folder includes all fonts in it and in its subfolders, including fonts that are added later.
- Scanned file types: `.otf`, `.ttf`, `.ttc`, `.otc`. Skip `.woff`, `.woff2`, `.zip`, and anything else.
- Skip `*.rightfontlibrary` folders during the scan.

### Collection file

One JSON file per collection, to reduce Google Drive conflicts when two people edit at the same time.

The file name is a slug of the collection name: lowercase, ASCII only (strip accents), and runs of anything that is not a letter or digit become a single `-`, with no leading or trailing `-`. Example: `Studio Work` → `.sorts/collections/studio-work.json`. The file name is only for people to read.

Each collection file has a fixed `id` field (a random UUID created when the collection is created). The `id` never changes. Personal settings refer to collections by `id`, so renaming a collection does not break anyone's settings.

- Create: generate a new `id`. Refuse if a file with the same slug already exists.
- Rename: write the new name and rename the file to the new slug. Keep the `id`. Refuse if the new slug already exists.
- Delete: remove the file. Other users' settings keep the old `id`; `status` reports collection IDs in the settings that do not exist.
- If two files have the same `id` (e.g. someone duplicated a file by hand), `status` reports it and the CLI uses the first file in alphabetical order.

```json
{
  "id": "5f0c2a8e-3b1d-4c47-9a6e-2d8f1b7c9e40",
  "name": "Studio Work",
  "fonts": [
    "Foundry A/Sans/",
    "Foundry B/Display/Display-Bold.otf"
  ]
}
```

Entries can be folder IDs or font IDs.

### Personal settings

Location: `~/Library/Application Support/sorts/config.json`

```json
{
  "root": "/Users/<user>/Library/CloudStorage/GoogleDrive-<account>/Shared drives/<Team>/Fonts",
  "enabled": ["Foundry A/Sans/", "Foundry B/Display/Display-Bold.otf"],
  "enabledCollections": ["5f0c2a8e-3b1d-4c47-9a6e-2d8f1b7c9e40"],
  "formatOrder": ["otf", "ttf", "ttc", "otc"]
}
```

- `enabled` holds folder IDs and font IDs, the same as a collection.
- `enabledCollections` holds collection `id` values, not slugs.
- `formatOrder` is the user's preferred file format order (see below).

The CLI also keeps a state file in the same folder with the list of font files that the CLI registered. The CLI only unregisters fonts that it registered itself, never fonts installed by other apps.

### Active set

```
active fonts = expand(enabled) + expand(each enabled collection)
```

`expand` turns folder IDs into the font files that are currently in the folder.

### Format order

Many fonts exist in more than one format with the same PostScript name, e.g. `Serif-Italic.otf` and `Serif-Italic.ttf` in the same folder. macOS has problems when two active fonts share a PostScript name.

Rule: when several files in the active set share a PostScript name, the CLI activates only one of them. It picks the file whose extension comes first in `formatOrder`. If two files still tie, pick the first path in alphabetical order.

The UI does **not** label these files as "duplicates". The format order is simply a user setting that the CLI applies.

## Font activation

- Use CoreText: `CTFontManagerRegisterFontsForURL` / `CTFontManagerUnregisterFontsForURL` with `CTFontManagerScope::Persistent` (crates `objc2-core-text` 0.3 + `objc2-core-foundation` 0.3). These calls are synchronous. Fonts stay in place in the Fonts folder. No copies, nothing in `~/Library/Fonts`.
- Read PostScript names with `CTFontManagerCreateFontDescriptorsFromURL` + `kCTFontNameAttribute`, or with `ttf-parser` (faster, used by the scanner).
- Check availability with `CTFontManagerCopyAvailablePostScriptNames`.
- CoreText errors to treat as success (the operation is idempotent for the CLI):
  - register: "The file has already been registered in the specified scope." (`kCTFontManagerErrorAlreadyRegistered`, 105)
  - unregister: "The file is not registered in the specified scope." (`kCTFontManagerErrorNotRegistered`, 201)
- **Disabled fonts.** Other font managers (e.g. RightFont) and Font Book disable fonts instead of unregistering them (`CTFontManagerEnableFontDescriptors(..., false)`, stored in `~/Library/Preferences/com.apple.FontRegistry.user.plist` under `DisabledFonts`). A disabled font stays unavailable after a new register (error 105), and unregister fails with 201. So `CoreTextActivator::register` enables the font after it registers it, and `unregister` enables it first. Never edit the plist directly; `fontd` owns it.
- Tests for disabled fonts use `CTFontManagerScope::Session` (`CoreTextActivator::session()`), because the process scope does not keep a disabled registration. Session registrations end at logout and do not write the plist.
- Fallback (not needed so far): symlinks in `~/Library/Fonts`.

### Activation test results (October 2026)

Test font: one `.otf` file in a Google Drive Fonts folder.

- Register with persistent scope from the Google Drive path: works.
- A new process sees the font (`CTFontManagerCopyAvailablePostScriptNames`): yes.
- AppKit in another process (`NSFont fontWithName:` via `osascript`): yes.
- Unregister: works. The font disappears from new processes and from AppKit.
- Register twice / unregister twice: fails with the errors above.
- **Still to verify by the user:** the font stays active after a restart, and it appears in Figma and Adobe apps.
- Fonts registered with persistent scope show in the macOS font list (`system_profiler SPFontsDataType`, the list Font Book reads) and disappear from it when they are unregistered. Not checked in the Font Book window itself.

## Sync behavior

- Toggles in the interactive list apply immediately: the CLI registers or unregisters the affected fonts right away.
- **There is no automatic sync.** When the contents of an enabled folder or collection change (a teammate adds or deletes fonts), the user must run `sorts sync` manually.
- `sync` does the reconciliation:
  1. Scan the Fonts folder and the collection files.
  2. Calculate the active set from the personal settings and apply `formatOrder`.
  3. Register all fonts in the active set, also the fonts in the state file (the system can lose or disable a registration). Only new fonts are reported.
  4. Unregister fonts that the CLI registered but are no longer in the active set.
- No interval sync, no LaunchAgent, no file watcher in version 1. These can come later.

## Version 1 features

### Commands

```
sorts init [path]     # set the Fonts root folder; without a path, asks "Use <cwd> as the Fonts folder? [y/N]" (only y/yes accepts; anything else exits with an error and changes nothing). Then scans and prints "Font files: N".
sorts                 # open the interactive list (default command)
sorts sync            # reconcile the active fonts with the settings
sorts status          # show the root, counts, and settings entries that point to files that do not exist
sorts fix             # open the interactive list on the Missing tab; without missing entries, says so and exits
```

There are **no** `enable` / `disable` commands. Enabling and disabling happens only in the interactive list.

### Interactive list (TUI, `ratatui`)

Built (roadmap step 3). A tree that mirrors the folder structure (see the snapshots in `src/tui/snapshots/`):

```
 Fonts   Collections   Settings    / regular_
 ▾ [~] Foundry A  2/3
   ▾ [x] Sans  2/2
       [x] Sans-Bold.otf  Sans Bold
       [x] Sans-Regular.otf  Sans Regular
   ▸ [ ] Serif  0/1
 ▸ [c] Foundry B  0/2
 Enabled Foundry A/Sans/. Registered 2, unregistered 0.
 Enter : Toggle | ←→ : Close/open | A-Z : Jump | / : Search | Tab : Collections | Esc : Quit
```

- `[x]` all fonts enabled in the personal settings, `[~]` some, `[c]` none, but some are active through an enabled collection, `[ ]` none. Folders show `enabled/total`. Unreadable files show `unreadable`.
- Folders first, then files, each sorted case-insensitively.
- Keys: `↑↓` move (there are no `jk` keys), `→` open, `←` close or go to the parent folder, `enter` toggle (`space` does nothing), `+` collection picker (see Collections), `/` search, `tab`/`shift-tab` switch tabs, `esc` quits, `ctrl-c` always quits. `q` does not quit. While typing a search: `enter` keeps the filter, `esc` clears it. `esc` with a kept filter clears it first.
- **Mouse** (`App::handle_mouse`, mouse capture in `tui/terminal.rs`): the wheel scrolls the list under the pointer and does not change the selection. In the Collections tab, the left list and the typeface column scroll separately (`App::typeface_offset`, area in `App::typeface_area`). The typeface column starts at the top for each newly selected collection, and stops when its last row is at the bottom. A left click selects the row under the pointer (and puts down a picked-up format). A left click on a tab title (`App::tab_areas`, also stored by the view) switches to that tab: it ends edit mode, search typing (the filter stays), and a picked-up format. A click on the current tab does nothing, so a click on "Fonts" in edit mode keeps edit mode. The view stores the list area of the last draw in `App::list_area`; events outside it are ignored. While a prompt is open, the mouse does nothing. After a scroll (`App::scrolled`, cleared by the next key), the list is drawn without the selection if the selected row is out of view, because ratatui scrolls a list back to its selected row. The next key brings the selection back into view.
- **Jump** (Fonts tab, also in edit mode), as in macOS lists: a letter or digit starts a jump text and selects the next visible row (after the selected one, wrapping) whose name starts with it, case-insensitive. Each key within 500 ms of the previous one (`JUMP_TIMEOUT`) adds to the text, also a space or punctuation (not `/`), and the search then starts at the selected row, so "se" can keep or move the selection. No match: the selection stays. Any other key or a mouse event ends the text. Tests pass the time with `App::handle_key_at`. This is why the Fonts tab has no letter commands (no `q`, no `hjkl`).
- **Search**: whitespace-separated terms, all must match (case-insensitive substring). A folder matches on its path; a file matches on its ID, family names, and PostScript names. Matches are shown with their parent folders, and those parents are opened automatically. A folder that matches itself stays closed and shows all its children when opened.
- **Toggle**: a fully enabled node (`[x]`) is disabled; anything else is enabled. The new `enabled` entries are rebuilt from the set of enabled fonts (`Tree::set_enabled`): a folder whose fonts are all enabled becomes one folder entry (so new fonts in it are included later), and disabling one file in an enabled folder splits that folder entry into entries for the rest. Entries that match no font are kept.
- **A toggle applies only its own change** (`sync::apply_changes` on the active set before vs. after). Fonts that are enabled but not registered yet (e.g. a teammate added them) still wait for `sorts sync`.
- The settings and the state are saved after every toggle. The result or error shows in the message line.

### Collections (in the TUI)

Built (roadmap step 4). A second tab lists the collections, sorted by file name.

Each row shows "N typefaces". A read-only right column shows the typefaces of the selected collection: "Name (N styles)", sorted case-insensitively. A style is a unique PostScript name, so an `.otf` + `.ttf` pair counts once. Unreadable files show as one "Unreadable (N files)" line at the end. If rows are below the view, the last line is "↓ N more". The mouse wheel scrolls the column. An empty collection shows "No fonts. Press e to add some."

- `enter`: enable / disable the collection in the personal settings. `space` does nothing.
- `n`: create. A prompt in the message line asks for the name. The file is `<slug>.json` with a new UUID `id` and no fonts.
- `r`: rename. The prompt starts with the current name. The file moves to the new slug; the `id`, the fonts, and everyone's enabled state stay.
- `d`: delete. Asks "Delete collection "X" for the whole team? y/n"; only `y` deletes. The ID is also removed from the user's own `enabledCollections`, and its fonts are unregistered.
- `e`: **edit mode**. The Fonts tab opens with "Editing: <name>" in the header. The checkboxes show membership in that collection (no `[c]` marks), and `enter` adds or removes the folder or font with the same rules as personal toggles (`Tree::set_enabled` on the collection's `fonts`). Each change is saved to the collection file at once. If the collection is enabled, the change is registered at once. `esc` (after clearing a search) or `tab` ends edit mode and returns to the Collections tab.
- While a prompt is open, it gets all keys (letters are text). `esc` cancels; `ctrl-c` still quits.
- Errors (e.g. "A collection file named brand.json already exists.") show in the message line.
- **Collection picker** (`App::picker`): in the Fonts tab (not in edit or replace mode), `+` opens a box over the tree with all collections and a mark for the selected folder or font (`App::collection_mark`: `[x]` all its fonts are in the collection, `[~]` some, `[ ]` none). `↑↓` select, `enter` adds or removes the node with the same rule as a toggle (`App::toggle_node`, shared with edit mode), `esc` closes it. The picker stays open after a change. While it is open, it gets all keys (`ctrl-c` still quits) and the mouse does nothing. `+` within a jump is jump text. Without collections, the message line says so and the picker does not open.

Collection changes write to `.sorts/collections/` in the Fonts folder, so they affect the whole team. After each change the CLI reads all collection files again.

### Settings (in the TUI)

A third tab, after Collections (`tab`: Fonts → Collections → Settings → Fonts; `shift-tab` goes back). It shows the format order: all four formats (`Settings::full_format_order`: the formats from `formatOrder` first, then the missing ones in the default order).

- `↑↓` select a format. `enter` picks it up (the row shows `⇅`).
- While a format is picked up, `↑↓` move it. Each move saves the full list to `formatOrder` and applies only its own change (`update_settings`, the same as a toggle), so the newly preferred files are registered and the others unregistered.
- `enter` or `esc` puts the format down. Any other key puts it down and does its usual action.
- The Fonts folder is not shown here (a temp path would break the snapshots). Change it with `sorts init`.

### Missing entries (in the TUI)

When someone moves or renames fonts, entries in `enabled` and in collections can match no font. There is no automatic matching (the user rejected heuristics as too convoluted). The user repairs each entry by hand.

- `Context::missing_entries` lists them: the personal settings first, then each collection (`Owner::Settings` / `Owner::Collection`).
- The Missing tab comes after Settings. It is shown only if entries were missing when the app opened, or later; once shown, it stays for the session ("All entries point to fonts." when empty). At start, the message line says how many entries are missing.
- `↑↓` select. `enter` starts **replace mode**: the Fonts tab opens with "Replace: <entry>" in the header, and `enter` on a folder or file asks `Replace "X" with "Y"? y/n` (for a collection: `... in "Name" for the whole team? y/n`). Only `y` replaces; another key closes the question and keeps replace mode. Any target type is allowed (a folder can replace a file).
- `entry::replace` puts the new entry at the place of the old one and does not add a duplicate. The change goes through `App::change_entries` (the same path as toggles), so the new fonts are registered at once. The old registrations of the moved files leave the state at the next `sync`.
- `esc` (after clearing a search), `tab`, or a click on a tab title ends replace mode.
- `d` asks to remove the entry (y/n), with the same team warning for collections.

## Facts about a real Fonts folder

Measured on a shared Google Drive folder with several thousand desktop fonts (October 2026):

- The folder is organized by foundry. Most fonts are 4 folder levels deep.
- Full walk without reading files: 20–60 ms. Walk plus reading all font names with `ttf-parser` + `rayon`: 120–235 ms. A cache is not needed.
- Many PostScript names appear in more than one file, mostly `.otf` + `.ttf` pairs in the same folder (hence `formatOrder`).
- Some files cannot be parsed. The CLI must handle unreadable files without failing.
- Files set to "Available offline" in Google Drive are read without downloads.
- Files that are not available offline are dataless placeholders. A read downloads them first, so a scan of an online-only folder can take minutes and showed nothing before the progress line was added.

## Online-only files

- A file sync app (Google Drive, Dropbox, ...) can keep files as dataless placeholders (`SF_DATALESS` in `st_flags`). Reading the metadata does not download the file. Reading the contents downloads it.
- The user wants the automatic download. sorts does not block it. It shows it instead.
- `scan::scan_with` checks each font file with `ScanOptions::is_dataless` (injected, so tests can fake it) and reports `ScanEvent`s: `Found(n)` during the walk, `Downloading { files, bytes }` once after the walk, `Downloaded { bytes }` per online-only file. `Library::downloaded` counts them.
- `progress::Progress` draws the events on stderr. In a terminal: one line updated in place (at most every 100 ms), cleared at the end. Without a terminal: only "Downloading N files (X MB)..." and the end line. After downloads it shows "Downloaded N files (X MB)." and a tip to make the folder available offline.
- Never name a specific sync app (e.g. "Google Drive") in the output. The user does not know which app a teammate uses.
- All commands that scan show the progress: `init` (it scans once and prints "Font files: N"), `sync`, `status`, `fix`, and `sorts`.
- With no font files, `sorts` and `sorts fix` fail with "No font files found in <root>." `init` prints the same text but keeps the root.

## Roadmap

1. **Activation test**: done (see "Font activation"). Only the restart and Figma/Adobe checks by the user remain.
2. **Rust project** (done): `init`, personal settings, scanner, active set calculation, `formatOrder`, `sync`, `status`.
3. **Interactive list** (done): tree, search, toggles.
4. **Collections** (done): list, enable/disable, create, rename, delete, edit.
5. **Distribution** (prepared): Homebrew tap, build from source. Files: `packaging/homebrew/sorts.rb.template`, `scripts/release.sh` (interactive release: bump, check, tag, push, tap), `scripts/update-formula.sh`, `.github/workflows/ci.yml`, `README.md`, `docs/RELEASING.md`.
6. **Team migration** (done).

## Distribution facts (October 2026, Homebrew 7.0.11)

- The user chose public repos and a formula that builds from source (`depends_on "rust" => :build`, `cargo install *std_cargo_args`, which uses `--locked`).
- **Homebrew refuses to build from source if the Command Line Tools are outdated** ("Your Command Line Tools are too outdated"), although `cargo` itself works with them. `brew install` of the formula is not verified end-to-end yet. Verified: `brew style` passes, `cargo install --locked` from a GitHub-style source archive works, and the formula's test commands pass.
- Homebrew 7 requires trust for third-party taps (`brew trust --formula <user>/<tap>/<formula>`). The README includes this step.
- `brew tap-new` turns on Homebrew developer mode. Turn it off again (`brew developer off`) after local tap tests.
- If outdated CLTs are a problem for the team, the alternative is prebuilt binaries (GitHub Actions release + bottles or a formula that downloads a binary). Not built.

## Later (not version 1)

- Automatic sync (interval LaunchAgent or file watcher).
- Fuzzy search.
- Font previews.
