# sorts

A font manager for macOS that runs in the terminal. I wrote it because existing
font managers:

- Charge a monthly subscription for simple features, or charge for new major
  versions that add little.
- Keep their own copy of every font, even if you already have an organized fonts
  folder.
- Cannot sync simple collections as plain files next to the fonts, for example
  in Google Drive or Dropbox.

`sorts` is for people and teams who already have an organized fonts folder. It
lets you enable and disable fonts from that folder, one by one, through the
parent folder, or through collections. The collections are stored in the same
folder, so they sync with it.

- **Your fonts stay in one folder.** This can be a folder that your team syncs,
  or any folder on your computer.
- **The folder is the source of truth.** To use a font, put its files in the
  folder and enable it in sorts. There is no import step and no cache: sorts
  reads the folder every time it runs.
- **Collections are JSON files** inside the fonts folder
  (`.sorts/collections/`).
- **Enable fonts or collections** with the `sorts` command.

The main downside: you cannot freely move your fonts after using them. Your
settings and the collections store the path of each font inside the fonts
folder. If a font moves, `sorts status` shows the broken paths, and `sorts fix`
lets you point them to the new location.

## Install

You need Homebrew and up-to-date Xcode Command Line Tools. Homebrew builds sorts
from source.

```sh
brew tap designsystemsinternational/tap
brew trust --formula designsystemsinternational/tap/sorts
brew install sorts
```

To update, run `brew upgrade sorts`.

## Usage

Tell sorts where your fonts folder is. Go to the folder in the terminal and run:

```sh
sorts init
```

Or run it from anywhere and give the path:

```sh
sorts init "$HOME/Library/CloudStorage/GoogleDrive-MyUser/Shared drives/MyTeam/Fonts"
```

sorts reads the names of all font files in the folder. If your sync app keeps
some files online only (for example Google Drive or Dropbox), sorts downloads
them first and shows the progress. This can take a long time the first time. To
make sorts start fast, make the fonts folder available offline in your sync app.

Then open the interactive list:

```sh
sorts
```

| Key               | Fonts tab                         | Collections tab                    |
| ----------------- | --------------------------------- | ---------------------------------- |
| `↑` `↓`           | Move                              | Move                               |
| `→` `←`           | Open / close a folder             |                                    |
| `a`–`z` `0`–`9`   | Jump to a row by its name         |                                    |
| `enter`           | Enable / disable a folder or font | Enable / disable a collection      |
| `/`               | Search                            | Search the fonts                   |
| `+`               | Add to / remove from collections  |                                    |
| `n` `r` `d`       |                                   | New / rename / delete a collection |
| `e`               |                                   | Edit the fonts in a collection     |
| `tab` `shift-tab` | Switch tabs                       | Switch tabs                        |
| `ctrl-c` / `esc`  | Quit                              | Quit                               |

Marks:

- `[x]` enabled
- `[~]` partly enabled
- `[c]` active through an enabled collection
- `[ ]` disabled

### Sync

When you enable a folder, sorts activates the fonts that are in it at that time.
If you or a teammate later add fonts to an enabled folder, or remove fonts from
it, run:

```sh
sorts sync
```

This activates the new fonts and deactivates the removed ones. It also repairs
fonts that other font managers left in a bad state, for example after you
uninstall them.

### Status

`sorts status` shows the fonts folder, the number of fonts, and problems:
unreadable files, and settings that point to fonts that no longer exist.

### Fix moved fonts

When someone moves or renames fonts in the fonts folder, your settings and the
collections can point to folders and files that no longer exist. To repair them,
run:

```sh
sorts fix
```

This opens the Missing tab, which lists each broken entry.

- To replace an entry: select it and press `enter`. The Fonts tab opens. Select
  the new folder or file, press `enter`, and press `y` to confirm.
- To remove an entry: select it and press `d`.

A change to a collection affects the whole team, so sorts asks before it saves
it. `sorts fix` never changes font files. When entries are missing, `sorts` also
shows the Missing tab.

### Format order

Some fonts exist in more than one format with the same PostScript name, for
example `Font.otf` and `Font.ttf`. sorts activates only one of them. To choose
which format sorts prefers:

1. Open the Settings tab.
2. Select a format and press `enter`.
3. Move it with `↑` `↓`.
4. Press `enter` again.

The change applies immediately. sorts saves the order as `formatOrder` in
`~/Library/Application Support/sorts/config.json`. The default order is `otf`,
`ttf`, `ttc`, `otc`.

## Develop

```sh
cargo test                 # unit and end-to-end tests
cargo test -- --ignored    # CoreText tests (process scope, nothing persists)
```

See `docs/RELEASING.md` for releases.

## Run from source

Use this to try changes without a release. You need Rust (`brew install rust`,
or [rustup](https://rustup.rs)) and the Xcode Command Line Tools
(`xcode-select --install`).

```sh
cargo install --locked --path .
```

This builds `sorts` from your local copy of the repo and puts it in
`~/.cargo/bin`. `--locked` uses the dependency versions in `Cargo.lock`, which
are the same versions that the tests use.

If your shell cannot find `sorts` after the install, add `~/.cargo/bin` to your
`PATH`. For zsh (the default shell on macOS):

```sh
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

Run the install command again after each change or `git pull`. It replaces the
installed version. If you also installed sorts with Homebrew, your shell uses
the one that comes first in your `PATH`. Run `which sorts` to check. To remove
the source install, run `cargo uninstall sorts`.

**Keep your test settings separate.** sorts stores its settings in
`~/Library/Application Support/sorts/`. To use another folder, for example while
you develop, set `SORTS_CONFIG_DIR`:

```sh
export SORTS_CONFIG_DIR="$HOME/sorts-dev"
cargo run --release -- init "/path/to/Fonts"
```

Your settings stay separate, but the fonts you enable are still really activated
on your Mac.

**Turn all fonts off again.** Disable them in the interactive list. Or set
`"enabled": []` and `"enabledCollections": []` in `config.json` and run
`sorts sync`. Do not delete `state.json` while fonts are active. sorts uses it
to know which fonts it activated, and it only deactivates those.
