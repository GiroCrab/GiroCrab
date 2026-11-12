# Homebrew distribution

GiroCrab for macOS is distributed through a Homebrew Cask that installs the **signed and
notarized stable release** from GitHub (`GiroCrab-macOS-arm64.zip`, Apple Silicon only,
macOS 14 Sonoma or later).

```bash
brew install --cask girocrab/tap/girocrab
```

The cask source lives in this repo at
[`packaging/homebrew/Casks/girocrab.rb`](../packaging/homebrew/Casks/girocrab.rb) and is
copied into the tap repository `GiroCrab/homebrew-tap` (path `Casks/girocrab.rb`).

## Own tap vs. official `homebrew/cask`

| | Own tap `GiroCrab/homebrew-tap` | Official `homebrew/cask` |
|---|---|---|
| Install command | `brew install --cask girocrab/tap/girocrab` | `brew install --cask girocrab` |
| Who updates | us (script below) | Homebrew's autobump bot, from `livecheck` |
| Requirements | none | notability + review (see below) |
| Available | immediately | after acceptance |

Requirements for the official repository (checked by `brew audit --cask --new`):

- **Notability:** roughly ≥ 75 stars, or ≥ 30 forks or watchers, and the GitHub repo must be
  **at least 30 days old**. GiroCrab has the stars (154 at the time of writing), but the repo
  was created on 2026-11-09, so `audit --new` currently fails with *"GitHub repository too
  new"*. The earliest submission date is around 2026-12-09.
- **Signed and notarized app.** Homebrew/cask no longer accepts apps that fail Gatekeeper.
  GiroCrab qualifies: the release app is signed with a Developer ID, notarized and stapled
  (`spctl` reports `source=Notarized Developer ID`).
- Stable, versioned download URLs (we have these: `releases/download/vX.Y.Z/...`).

Recommendation: start with the own tap now. Submit to `homebrew/cask` once the repo is
30 days old. After that the official cask is updated automatically, and the tap can then
be retired or kept as a mirror.

## Only stable releases

- The cask URL is pinned to a tag: `releases/download/v#{version}/GiroCrab-macOS-arm64.zip`.
- `livecheck` uses `strategy :github_latest`. GitHub's "latest release" never returns a
  pre-release, so the rolling `nightly` pre-release is never offered.
- `scripts/update-homebrew-cask.sh` refuses drafts and pre-releases, and only accepts
  `vX.Y.Z` tags.
- Possible later alternative: once every stable release ships a Sparkle `appcast.xml`,
  livecheck could use `strategy :sparkle` with
  `url "https://github.com/GiroCrab/GiroCrab/releases/latest/download/appcast.xml"`.
  `:github_latest` is simpler and does not depend on the appcast, so we keep it.

## In-app updates (Sparkle) and `auto_updates true`

The app updates itself through Sparkle, so the cask declares `auto_updates true`. As a
result, `brew upgrade` skips GiroCrab unless the user passes `--greedy`, and Homebrew does
not fight Sparkle over the app bundle. The version Homebrew shows can lag behind the
installed app; this is expected for self-updating casks.

> Note: the current release v0.5.5 does not yet contain `Sparkle.framework`. The
> `auto_updates true` stanza becomes accurate with the first stable release that ships
> Sparkle. Until then, users upgrade with `brew upgrade --cask --greedy girocrab`.

## Releasing: updating the cask

After `scripts/notarize.sh --upload vX.Y.Z` has published the stable release:

```bash
scripts/update-homebrew-cask.sh vX.Y.Z
```

The script reads the release via `gh api`, takes the asset's SHA-256 digest (or downloads
and hashes the zip if GitHub has no digest), and rewrites `version` and `sha256` in
`packaging/homebrew/Casks/girocrab.rb`. It does **not** commit or push.

Then validate and publish manually:

```bash
brew tap-new --no-git girocrab/tap   # once, creates a local tap for testing
cp packaging/homebrew/Casks/girocrab.rb "$(brew --repository)/Library/Taps/girocrab/homebrew-tap/Casks/"
brew trust girocrab/tap
brew style --cask girocrab/tap/girocrab
brew audit --cask --online --strict girocrab/tap/girocrab
```

Finally, copy the file into the `GiroCrab/homebrew-tap` checkout, commit and push it.

## Uninstall and `--zap`: the vault

| Command | Effect |
|---|---|
| `brew uninstall --cask girocrab` | removes `GiroCrab.app` only. The vault and settings stay. |
| `brew uninstall --cask --zap girocrab` | also moves app data to the **Trash** |

The `zap trash:` list covers `~/Library/Application Support/GiroCrab`, which holds the
encrypted vault `tresor.girocrab`, plus the app's caches, preferences, HTTP storage,
WebKit data and saved window state (`app.girocrab.mac`).

Why the vault is included: `--zap` is an explicit opt-in that means "remove everything
this app left behind", and Homebrew reviewers expect app data in it. To make this safe
and documented rather than silent:

- `trash:` moves files to the Trash instead of deleting them, so they can be restored until
  the Trash is emptied.
- The cask's `caveats` (shown on install) say where the vault is, and that `--zap` moves it
  to the Trash.
- The vault key in the Keychain (service `de.girocrab.vault`) is **not** removed, so a
  restored vault can still be opened.
- The optional iCloud Drive sync/mirror folder (`iCloud Drive/GiroCrab`, or
  `iCloud Drive/moneymoney`) is **never** touched, because it may be the user's only
  backup.

If maintainers would rather never touch the vault, remove that one line from `zap` and
change the caveat text to "is never deleted by Homebrew".

## README snippet (draft, not yet added)

```markdown
### Homebrew
brew install --cask girocrab/tap/girocrab
```
