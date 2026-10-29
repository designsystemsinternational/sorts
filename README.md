# sorts

A font manager for macOS that runs in the terminal. It is written for teams who
already share a font folder over Dropbox or Google Drive.

- **Store your fonts in a folder**. This can be a synced folder for your team or
  any folder on your computer.
- **Manage fonts via the filesystem.** To add a font, put the file in the
  folder. There is no import step and no cache.
- **Collections** live inside the font folder (`.sorts/collections/`), so they
  can be synced with your team.
- **Enable fonts or collections.** You can enable individual fonts or
  collections via the `sorts` command-line tool.

## Install

You need Homebrew and current Xcode Command Line Tools (Software Update in
System Settings). Homebrew builds sorts from source.

```sh
brew tap designsystemsinternational/tap
brew trust --formula designsystemsinternational/tap/sorts
brew install sorts
```

Update with `brew upgrade sorts`.

## Run from source

Use this to try changes without a release. You need Rust (`brew install rust`,
or [rustup](https://rustup.rs)) and the Xcode Command Line Tools
(`xcode-select --install`).

```sh
cargo install --locked --path .
```

This installs the `sorts` command from your local repo. This puts the binary in
`~/.cargo/bin`. `--locked` uses the dependency versions in `Cargo.lock`, the
same versions that the tests use:

If your shell cannot find `sorts` afterwards, add `~/.cargo/bin` to your `PATH`
(for zsh, the default shell on macOS):

```sh
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

Run the install command again after each change (or after `git pull`). It
replaces the installed binary. If you also installed sorts with Homebrew, the
first `sorts` in your `PATH` wins; check with `which sorts`. Remove the source
install with `cargo uninstall sorts`.

**Keep your test settings separate.** Settings live in
`~/Library/Application Support/sorts/`. To use another folder, for example while
you develop, set `SORTS_CONFIG_DIR`:

```sh
export SORTS_CONFIG_DIR="$HOME/sorts-dev"
cargo run --release -- init "/path/to/Fonts"
```

This keeps the settings apart, but enabled fonts are still really activated on
your Mac.

**Turn all fonts off again.** Disable them in the interactive list, or set
`"enabled": []` and `"enabledCollections": []` in `config.json` and run
`sorts sync`. Do not delete `state.json` while fonts are active: sorts uses it
to know which fonts it activated, and only deactivates those.

## Use

Point sorts at your Fonts folder once:

```sh
sorts init "$HOME/Library/CloudStorage/GoogleDrive-<you>/Shared drives/<Team>/Fonts"
```

Or go to the Fonts folder and run `sorts init` without a path. It asks before it
uses the current folder:

```sh
cd "$HOME/Library/CloudStorage/GoogleDrive-<you>/Shared drives/<Team>/Fonts"
sorts init
# Use /Users/<you>/Library/CloudStorage/.../Fonts as the Fonts folder? [y/N] y
```

Then open the interactive list:

```sh
sorts
```

| Key              | Fonts tab                         | Collections tab                    |
| ---------------- | --------------------------------- | ---------------------------------- |
| `↑` `↓`          | Move                              | Move                               |
| `j` `k`          |                                   | Move                               |
| `→` `←`          | Open / close a folder             |                                    |
| `a`–`z` `0`–`9`  | Jump to a row by its first letter |                                    |
| `space`          | Enable / disable a folder or font | Enable / disable a collection      |
| `/`              | Search                            | Search the fonts                   |
| `n` `r` `d`      |                                   | New / rename / delete a collection |
| `e` / `enter`    |                                   | Edit the fonts in a collection     |
| `tab`            | Switch tabs                       | Switch tabs                        |
| `ctrl-c` / `esc` | Quit                              | Quit                               |

Marks: `[x]` enabled, `[~]` partly enabled, `[c]` active through an enabled
collection, `[ ]` disabled.

When you enable a folder, all fonts in it are enabled, also fonts that are added
later. Changes apply immediately. When teammates add or remove fonts in the
Fonts folder, run:

```sh
sorts sync
```

`sorts status` shows the Fonts folder, counts, and problems (unreadable files,
settings that point to fonts that no longer exist).

When several active files have the same PostScript name (for example `Font.otf`
and `Font.ttf`), sorts activates only one. The order is set by `formatOrder` in
`~/Library/Application Support/sorts/config.json` (default: `otf`, `ttf`, `ttc`,
`otc`).

## Develop

```sh
cargo test                 # unit and end-to-end tests
cargo test -- --ignored    # CoreText tests (process scope, nothing persists)
```

See `docs/RELEASING.md` for releases.
