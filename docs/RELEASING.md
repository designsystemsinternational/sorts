# Releasing

sorts is distributed with a Homebrew tap that builds from source.

- Code: `github.com/designsystemsinternational/sorts` (public)
- Tap: `github.com/designsystemsinternational/homebrew-tap` (public), with `Formula/sorts.rb`
- Formula template: `packaging/homebrew/sorts.rb.template`

## One-time setup

1. Create the public repo `designsystemsinternational/sorts` and push `main`.
2. Create the public repo `designsystemsinternational/homebrew-tap` (the `homebrew-` prefix is required). Clone it next to this repo.

## Release a version

1. Set `version` in `Cargo.toml` (e.g. `0.2.0`) and run `cargo build` to update `Cargo.lock`.
2. Run all checks:
   ```sh
   cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo test -- --ignored
   ```
3. Commit, then tag and push:
   ```sh
   git tag v0.2.0
   git push origin main v0.2.0
   ```
4. Write the formula into the tap checkout. The script downloads the GitHub archive of the tag and fills in the URL and the SHA-256:
   ```sh
   scripts/update-formula.sh 0.2.0 ../homebrew-tap
   ```
5. Check the formula, then commit and push the tap:
   ```sh
   brew style ../homebrew-tap/Formula/sorts.rb
   cd ../homebrew-tap && git add Formula/sorts.rb && git commit -m "sorts 0.2.0" && git push
   ```
6. Test the install: `brew update && brew upgrade sorts` (or `brew install sorts`), then `brew test sorts`.

## Notes

- Homebrew refuses to build from source when the Xcode Command Line Tools are outdated ("Your Command Line Tools are too outdated"). Team members must keep them current.
- Homebrew 7 requires trust for third-party taps: `brew trust --formula designsystemsinternational/tap/sorts`.
- The formula's `head` points at `main`, so `brew install --HEAD sorts` builds the latest commit.
