# Releasing and updates

## How updates reach people

The installed app checks `https://scoutcode.dev/download/latest.json` five seconds after it starts and every 6 hours.
Anything newer downloads in the background to `update-<version>.exe` next to the exe, and a **Restart to update** button
appears in the title bar. Restarting reopens the same folder and chat (it waits while Scout is working or the editor
has unsaved changes). If the user just closes the window, the update goes in then. Settings > Updates shows the status,
the update summary and changelog and a switch for automatic downloads, and "Check for updates" is in the account menu.
The summary is also in the button's tooltip.

Only an installed release build (`Scout.exe`) updates itself, never `cargo run`. `SCOUT_NO_UPDATE=1` turns it off and
`SCOUT_UPDATE_URL` points it at another manifest.

**Nothing is installed unless its Ed25519 signature checks out against the key built into the app
(`PUBLIC_KEY` in `src/update.rs`) and its SHA-256 matches**, so control of the site or the bucket isn't enough to push
code to people. The signed message is `scout-desktop:<platform>:<version>:<sha256>`.

Each release has a one-line summary and a changelog. Besides `latest.json`, the release script keeps `releases.json` with
every release's notes, served at `https://scoutcode.dev/download/releases.json` (`?since=1.2.3` leaves out that version
and older). After an update, the first start looks up the releases since the version that ran before
(`desktopLastVersion` in the config) and shows them once in a "What's new" dialog. A fresh install shows nothing.

## Publishing a release

You need:

- rustup's `x86_64-pc-windows-gnullvm` and `aarch64-pc-windows-gnullvm` targets, and
  [llvm-mingw](https://github.com/mstorsjo/llvm-mingw) on the `PATH`
- `SUPABASE_URL` and `SUPABASE_SERVICE_ROLE_KEY`, in the environment or in a `.env` in this folder (git ignores it)
- the private signing key in `~/.scoutcode/desktop-release-key.pem`, or at the path in `SCOUT_RELEASE_KEY`

Bump `version` in `Cargo.toml` (Tauri reads it from there), then:

```powershell
npm ci
npm run release -- --summary "One line" --changelog "- What changed"
```

`--changelog-file CHANGES.md` reads the changelog from a file instead. This builds the web UI once, then the release
binary for x64 and ARM64, checks that each exe is for the right architecture and needs no toolchain DLLs, signs both,
uploads them to the public `desktop-releases` bucket in Supabase, and publishes `releases.json` and then `latest.json`.
Each installed copy fetches its own architecture. The site's download button gives x64, and
`/download/windows?arch=arm64` gives ARM64.

Other forms:

```powershell
# Publish the builds already in target/ without building again
npm run release -- --skip-build --summary "…" --changelog-file CHANGES.md

# Try a release against a local server: writes the files to a folder and uploads nothing
npm run release -- --skip-build --out <dir> --url-base http://127.0.0.1:8765 --version 9.9.9

# Make the signing key, once; prints the public half for PUBLIC_KEY in src/update.rs
npm run release -- keygen
```

## Rules

- **Keep a backup of the signing key outside the repo.** Installed apps accept nothing else, so if it's lost they can't
  be updated and everyone has to reinstall. Never commit it.
- **Apps only move to a higher version.** To undo a bad release, publish the old code under a new number.
- Every installed copy picks up a release within 6 hours, so only publish what's meant for everyone.
- The website (`src/lib/desktop.ts` in the site's repo) reads the same bucket with the same file names and shapes as
  `scripts/release.ts`. Change them together.
