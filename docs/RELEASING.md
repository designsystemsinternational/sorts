# Releasing

sorts is distributed with a Homebrew tap that builds from source.

- Code: `github.com/designsystemsinternational/sorts` (public)
- Tap: `github.com/designsystemsinternational/homebrew-tap` (public), with `Formula/sorts.rb`
- Formula template: `packaging/homebrew/sorts.rb.template`

## One-time setup

1. Create the public repo `designsystemsinternational/sorts` and push `main`.
2. Create the public repo `designsystemsinternational/homebrew-tap` (the `homebrew-` prefix is required). Clone it next to this repo.

## Release a version

A push to `main` does not release anything. Homebrew installs the version in the formula, and the formula points at a tag (`v0.2.0`). So a release needs a new version number, a tag, and an updated formula in the tap.

Run the release script from a clean `main`:

```sh
scripts/release.sh
```

The script:

1. Stops if you are not on `main`, if there are uncommitted changes, if `origin/main` has commits that you do not have, or if the tap checkout (`../homebrew-tap`, or `TAP_DIR`) has changes.
2. Shows the commits since the last tag and asks for the kind of release:
   - **patch** (`0.2.0` → `0.2.1`): bug fixes only.
   - **minor** (`0.2.0` → `0.3.0`): new features.
   - **major** (`0.2.0` → `1.0.0`): changes that break how people use it.
3. Sets the version in `Cargo.toml` and runs all checks. If a check fails, it restores `Cargo.toml` and `Cargo.lock` and stops.
4. Commits "Release vX.Y.Z" and creates the tag locally.
5. Asks before it pushes `main` and the tag to GitHub. If you say no, it prints how to undo the local commit and tag.
6. Pulls the tap, writes the formula with `scripts/update-formula.sh`, runs `brew style`, and shows the diff.
7. Asks before it commits and pushes the tap.

Then test the install: `brew update && brew upgrade sorts` (or `brew install sorts`), then `brew test sorts`.

### By hand

The same steps without the script:

1. Set `version` in `Cargo.toml` and run `cargo build` to update `Cargo.lock`.
2. Run all checks: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo test -- --ignored`
3. Commit, then `git tag v0.2.0 && git push origin main v0.2.0`.
4. `scripts/update-formula.sh 0.2.0 ../homebrew-tap`
5. `brew style ../homebrew-tap/Formula/sorts.rb`, then commit and push the tap.

## Notes

- Homebrew refuses to build from source when the Xcode Command Line Tools are outdated ("Your Command Line Tools are too outdated"). Team members must keep them current.
- Homebrew 7 requires trust for third-party taps: `brew trust --formula designsystemsinternational/tap/sorts`.
- `brew style` turns on Homebrew developer mode. The release script turns it off again if it was off before. By hand, run `brew developer off`.
- The formula's `head` points at `main`, so `brew install --HEAD sorts` builds the latest commit.
