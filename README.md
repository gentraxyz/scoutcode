<p align="center">
  <img src="assets/icon.png" width="96" alt="Scout logo">
</p>

<h1 align="center">Scout Desktop</h1>

<p align="center">
  A coding agent for Windows that works in your project folder, asks before it changes anything, and lets you undo what it did.
</p>

<p align="center">
  <a href="https://scoutcode.dev/download/windows"><b>Download for Windows (x64)</b></a> ·
  <a href="https://scoutcode.dev/download/windows?arch=arm64">ARM64</a> ·
  <a href="https://scoutcode.dev">scoutcode.dev</a> ·
  <a href="LICENSE">MIT License</a>
</p>

---

Scout reads your code, edits files, runs commands and searches the web, with a diff and a permission card for every
change. It's a single executable built with [Tauri](https://tauri.app): the agent and everything that touches your
computer are Rust, and the window is React in the system's WebView2.

## Features

- **Asks first, undoes after.** Edits and commands ask in a card with the diff. Choose per action, for the session, or
  for a command prefix in this project. `/undo` and editing an earlier message put the files back.
- **Five permission modes.** Ask first, Accept edits, Plan (read-only), Auto (runs edits and commands but holds dangerous
  ones and edits outside the project for review) and Yolo (never asks). Shift+Tab cycles them.
- **Many models and providers.** ScoutCode's own models, plus your Claude Code and Codex plans and your accounts
  with GitHub Copilot, OpenRouter, NVIDIA NIM, Kilo, Nous Portal (Hermes), Devin, Kiro, OpenCode and Grok Build. See [docs/providers.md](docs/providers.md). When a
  model fails, Scout can carry the turn on with fallback models you pick, for every model or just one.
- **Built-in workspace.** A right-hand panel with a real terminal (xterm.js on a pseudo console), the files changed since
  the last commit with their diffs, and a file tree with an editor.
- **Memory and instructions.** Project memory in `.scout/memory.md` (which you can commit) and personal memory in
  `~/.scoutcode/memory.md`, plus saved instructions and a response language. `SCOUT.md`, `AGENTS.md` and `CLAUDE.md` are
  read too.
- **Skills and MCP.** Install skills from a folder, a `.zip` or [skills.sh](https://skills.sh), and mention them with `@`.
  Connect MCP servers over stdio or Streamable HTTP with the usual `mcpServers` config.
- **Attachments and voice.** Drop files or images on the window, or pick files with `@`. Dictate with the mic or
  Ctrl+Shift+Space.
- **Long sessions.** Queue messages while Scout works, or steer the running turn. Automatic pruning and compaction keep
  long chats going, and several chats can run at once.
- **Self-updating.** Signed updates download in the background and install on restart.

## Install

1. Download `ScoutSetup.exe` from [scoutcode.dev](https://scoutcode.dev/download/windows)
   ([ARM64](https://scoutcode.dev/download/windows?arch=arm64)).
2. Run it. It installs to `%LOCALAPPDATA%\Programs\Scout`, adds a Start menu entry, and appears under Settings > Apps,
   where you can uninstall it.
3. Sign in with your browser, paste a ScoutCode API key, or choose **Continue without an account** to use the free
   models at the guest limits.

Windows SmartScreen warns about the installer the first time because the exe isn't code-signed yet. Choose
**More info > Run anyway**.

**Requirements:** Windows 10 or 11 (x64 or ARM64) with the WebView2 runtime. Windows 11 and most copies of Windows 10
already have it; if yours doesn't, Scout offers the download.

## Usage

Launched from the Start menu, Scout opens the last folder you used (your home folder the first time). Use
**Open folder…** to switch. From a terminal:

```powershell
Scout.exe [folder]                 # open a project; defaults to the current directory
Scout.exe --resume <session-id>    # reopen a saved chat (ids are file names in ~/.scoutcode/sessions)
```

### Keyboard shortcuts

| Keys | Action |
| --- | --- |
| Enter / Shift+Enter | Send / new line (while Scout works, Enter queues the message) |
| Ctrl+Enter | Steer the running turn with the message |
| Esc | Stop |
| ↑ / ↓ | Message history |
| Shift+Tab | Cycle the permission mode |
| Ctrl+N | New chat |
| Ctrl+F | Find in the open chat |
| Ctrl+\` | Toggle the terminal |
| Ctrl+Shift+Space | Start or stop dictation |
| Ctrl+S | Save in the editor |

In the message box, type `/` to pick a command: `/plan <task>` (Scout writes a plan before it starts), `/compact [focus]`,
`/undo`, `/prune` and `/new`.

### Where your data lives

Everything stays on your computer in `~/.scoutcode` (`%USERPROFILE%\.scoutcode`):

| Path | What |
| --- | --- |
| `config.json` | Sign-in, settings and keys you save |
| `sessions/` | Saved chats |
| `instructions.json`, `memory.md` | Your saved instructions and personal memory |
| `skills/`, `mcp.json` | Installed skills and MCP servers |
| `runtime/` | CLIs downloaded for providers that need them |

Uninstalling keeps this folder, so your chats and sign-in survive a reinstall. Delete it to remove them.

What leaves your computer: messages and the files you attach go to the model provider you pick. Voice recordings go to
ScoutCode's server, which transcribes them with Deepgram. Update checks go to scoutcode.dev.

### Environment variables

| Variable | Effect |
| --- | --- |
| `SCOUTCODE_API_KEY`, `SCOUTCODE_BASE_URL` | Use this key or server instead of the saved sign-in |
| `OPENROUTER_API_KEY`, `NVIDIA_API_KEY`, `KILO_API_KEY` | Provider keys, used over the saved ones |
| `SCOUT_HOME` | Keep data somewhere other than `~/.scoutcode` |
| `SCOUT_TERMINAL` | The shell for the terminal panel (default: PowerShell 7 if installed, else Windows PowerShell) |
| `SCOUT_NO_UPDATE=1` | Turn off updates |
| `SCOUT_UPDATE_URL` | Check another update manifest |

## Security

- Every edit and command goes through the permission modes above. In Auto, commands flagged as dangerous and edits
  outside the project still wait for you.
- Updates install only if they're signed with ScoutCode's release key, which is built into the app, and their SHA-256
  matches. Control of the website or the download bucket isn't enough to push code to you.
- Provider CLIs are checked against hashes pinned in the source before they run, and are kept apart from any other copy
  on your PC. Scout refuses to run OpenCode or Grok Build in folders where they would run a project's own code without
  asking. See [docs/providers.md](docs/providers.md).
- The window can't fetch anything (a strict CSP), replies are rendered as Markdown without raw HTML, and memory edits
  refuse secrets.

To report a vulnerability, email the maintainers through [scoutcode.dev](https://scoutcode.dev) rather than opening a
public issue.

## Building from source

**Prerequisites:** Windows 10 or 11, [Rust](https://rustup.rs) stable (1.85 or newer, for edition 2024) and
[Node.js](https://nodejs.org) 20 or newer.

```powershell
git clone https://github.com/gentraxyz/scoutcode.git
cd scoutcode
powershell -ExecutionPolicy Bypass -File install.ps1
```

`install.ps1` builds the web UI and the release binary (with thin LTO, so it takes a few minutes), installs it to
`%LOCALAPPDATA%\Programs\Scout\Scout.exe` with a Start menu entry, and closes a running copy first. Run it again to
update. `-Desktop` adds a desktop shortcut, and `-Uninstall` removes everything except `~/.scoutcode`.

For development, run the UI from Vite so changes to `ui/` show at once and only Rust changes rebuild:

```powershell
npm --prefix ui ci
npx --prefix ui tauri dev -- -- path\to\project
```

For a build like the installed one without installing it, run `npm --prefix ui run build`, then `cargo build --release`.
The UI in `ui/dist` and WebView2's loader both go into `target/release/scout-desktop.exe`, so it needs nothing beside it.

### Tests

```powershell
cargo test
```

Tests marked `#[ignore]` talk to real services and need an account. For example,
`cargo test -- --ignored live_turn --nocapture` runs one real turn against your account in a temporary folder.
Set `SCOUT_HOME` to a temporary folder for the provider tests so they don't touch your real install.

### How it's put together

Rust owns the state and the web UI draws it. `src/core.rs` builds a view of what the window needs and sends only the
parts that changed as `view`, `models` and `items` events; the UI calls back through the commands in `src/bridge.rs`.
What's only about drawing, like open menus and drafts, stays in the UI.

| File | |
| --- | --- |
| `src/agent.rs` | The agent loop on a worker thread; talks to the core over a channel |
| `src/api.rs` | Streaming chat completions (SSE) with one pooled HTTP connection |
| `src/tools.rs`, `src/patch.rs` | File, search, shell and web tools; `apply_patch` |
| `src/core.rs` | The app's state: the transcript, sign-ins, the queue, and what's sent to the window |
| `src/bridge.rs` | The commands the UI can call |
| `src/dock.rs` | The right panel's backend: the shell in a pseudo console, git, files |
| `src/webview2.rs` | WebView2's loader, built into the exe so that nothing has to sit beside it |
| `src/attach.rs` | Attached files and the `@` picker's file list |
| `src/skills.rs` | Installed skills, uploads and the skills.sh directory |
| `src/mcp_client.rs` | The user's MCP servers (`src/mcp.rs` is the opposite: Scout's tools for chatbots) |
| `src/acp.rs` and the provider files | Outside providers; see [docs/providers.md](docs/providers.md) |
| `src/update.rs` | Checking for, verifying and installing updates, and installing from `ScoutSetup.exe` |
| `src/config.rs`, `src/session.rs`, `src/prompt.rs`, `src/oauth.rs`, `src/diff.rs` | Settings, saved chats, the system prompt, browser sign-in and diffs |
| `ui/src/store.ts`, `ui/src/api.ts` | What Rust last said, and the shapes it says it in |
| `ui/src/App.tsx`, `Sidebar.tsx`, `Transcript.tsx`, `Composer.tsx`, `ModelMenu.tsx` | The window, the chat and its menus |
| `ui/src/Settings.tsx`, `Accounts.tsx` | Settings, and the sign-ins several screens share |
| `ui/src/Dock.tsx` | The right panel: terminal (xterm.js), changes, files and the editor (CodeMirror) |
| `scripts/release.ts` | Building, signing and publishing a release; see [docs/releasing.md](docs/releasing.md) |

## Contributing

Issues and pull requests are welcome. Work happens on the `dev` branch, so open pull requests against `dev`; `main` is
what's released. CI builds and tests every pull request. Before opening one, run `cargo test` and build with
`install.ps1` to check that the app still starts. For changes to a provider, read its section in [docs/providers.md](docs/providers.md)
first, since several of them have safety checks that need re-testing with each new CLI version.

Merging a version bump into `main` publishes a release automatically, with notes from [CHANGELOG.md](CHANGELOG.md);
see [docs/releasing.md](docs/releasing.md).

## License

[MIT](LICENSE) © 2026 Gentra
