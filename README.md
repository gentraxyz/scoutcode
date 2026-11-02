# Scout Desktop

The desktop app for Scout, built with [Tauri](https://tauri.app): the agent, its tools and everything else that touches
the computer are Rust (`src/`), and the window is React and TypeScript (`ui/`) in the system's WebView2. It still ships as
one executable with nothing beside it, since the UI and WebView2's loader are built in (see `src/webview2.rs`).
Without signing in, "Continue without an account" uses ScoutCode's free models that admins approved for the desktop app, at
the server's `desktop` limits, counted per address. The app marks those requests with `x-scoutcode-desktop-guest: 1` and its
user agent. That keeps web pages out, but a script can copy it, so the limits and free models are what protect the server.

Windows 11 has the WebView2 runtime, as do most copies of Windows 10; without it Scout offers the download and stops.

It keeps its login, settings, allow list and sessions in `~/.scoutcode`. Chats saved by the old Scout CLI, which has
been removed, are in the same format and still open.

## Install

```powershell
powershell -ExecutionPolicy Bypass -File install.ps1
```

This builds the web UI and the release binary (with thin LTO, so it takes a few minutes), copies it to `%LOCALAPPDATA%\Programs\Scout\Scout.exe`, and adds a Start menu entry,
so Windows search finds "Scout". It also lists Scout under Settings > Apps, where you can uninstall it. Run the script
again to update. Add `-Desktop` for a desktop shortcut, or `-Uninstall` to remove everything except your chats and
login in `~/.scoutcode`. Launched from the Start menu, Scout opens the last folder you opened (your home folder the
first time); use "Open folder…" to switch.

People who aren't building it themselves download `ScoutSetup.exe` from the site. It's the same binary: run under that
name, it installs itself to the same place with the same Start menu entry and starts the installed copy. Windows warns
about it the first time because the exe has no Authenticode signature.

## Updates

The installed app checks `https://scoutcode.dev/download/latest.json` five seconds after it starts and every 6 hours.
Anything newer downloads in the background to `update-<version>.exe` next to the exe, and a **Restart to update** button
appears in the title bar. Restarting reopens the same folder and chat (it waits while Scout is working or the editor
has unsaved changes). If the user just closes the window, the update goes in then. Settings > Updates shows the status,
the update summary and changelog and a switch for automatic downloads, and "Check for updates" is in the account menu.
The summary is also in the button's tooltip.

Each release has a one-line summary and a changelog (`--summary`, and `--changelog` or `--changelog-file`, on
`npm run release`). Besides latest.json, the script keeps `releases.json` in the bucket with every release's
notes, served at `https://scoutcode.dev/download/releases.json` (`?since=1.2.3` leaves out that version and older).
After an update, the first start looks up the releases since the version that ran before (`desktopLastVersion` in
the config) and shows them once in a "What's new" dialog. A fresh install shows nothing.

Only an installed release build (`Scout.exe`) updates itself, never `cargo run`. `SCOUT_NO_UPDATE=1` turns it off and
`SCOUT_UPDATE_URL` points it at another manifest. Nothing is installed unless its signature checks out against the key
built into the app and its SHA-256 matches, so control of the site or the bucket isn't enough to push code to people.

### GitHub Copilot

Settings > Account signs in to the user's own GitHub account with Scout's OAuth App (device flow). That also downloads
GitHub's runtime (about 150 MB, checked against the hashes in `src/copilot.rs`) to `~/.scoutcode/runtime`, which needs no
Node. Scout's tools and permissions do the work; the runtime's own tools are off. To move to a new version, change the
`github-copilot-sdk` version in `Cargo.toml` together with `VERSION` and `PACKAGES` in `src/copilot.rs`. The first build
also downloads the runtime to a per-user cache: the crate's build script does it, and Scout's binary doesn't contain it.

### Devin

Settings > Account downloads Cognition's Devin CLI (about 75 MB, unpacking to about 200 MB, checked against the hashes in
`src/devin.rs`) to `~/.scoutcode/runtime/devin` and signs in with the browser. `src/devin.rs` runs it as `devin acp`, one
process per turn, and speaks the Agent Client Protocol to it. Devin works in the user's own account and plan.

- Scout offers ACP's file and terminal calls, so Devin's reads, writes and commands come back here and go through Scout's
  permissions, diffs and undo. Searches and any web or MCP tools run inside Devin; Scout only shows them, and asks first
  when Devin asks for permission.
- Devin brings its own system prompt, and reads `AGENTS.md` and its own settings. Scout sends the conversation as text,
  with the user's saved instructions.
- The CLI keeps the token and Scout never sees it. On Windows nothing moves where it's kept (`XDG_*` and `APPDATA` are
  ignored, and overriding `USERPROFILE` stops the CLI), so the sign-in is shared with any other Devin CLI on the PC.

To move to a new version, take it and the archive URLs from the
[ACP registry](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json), then change `VERSION` and hash each
archive for `ARCHIVES` in `src/devin.rs`. Read `devin acp --help`, and run `cargo test -- --ignored live_devin --nocapture`
while signed in: the CLI's ACP extensions aren't documented, so a new version can change them.

### Kiro

Settings > Account downloads AWS's Kiro CLI (about 330 MB on Windows, unpacking to about 470 MB, checked against the hashes
in `src/kiro.rs`) to `~/.scoutcode/runtime/kiro` and signs in with the browser. On Windows the only package is an MSI, which
is unpacked with `msiexec /a` and never installed. `src/kiro.rs` runs it as `kiro-cli acp --agent-engine=v3`, one process
per turn, and speaks the Agent Client Protocol to it. Kiro works in the user's own account and plan.

- Kiro runs its own tools and offers no file or terminal calls, so Scout shows them and answers what it asks permission
  for, but its edits are outside Scout's diffs and undo. Each session turns its `autopilot` off, so that it asks first,
  unless the mode is Yolo. Each also turns `contentCollection` off.
- Kiro brings its own system prompt and settings. Scout sends the conversation as text, with the user's saved instructions.
- The CLI keeps the token and Scout never sees it. The sign-in is shared with any other Kiro CLI on the PC.

To move to a new version, take it and the hashes from Kiro's
[release manifest](https://prod.download.cli.kiro.dev/stable/latest/manifest.json), and change `VERSION` and `PACKAGES` in
`src/kiro.rs`. Read `kiro-cli acp --help` and the [ACP migration guide](https://kiro.dev/docs/cli/v3/acp-migration/), and run
`cargo test -- --ignored live_kiro --nocapture` while signed in.

### OpenCode

Settings > Account or the model menu downloads OpenCode's CLI (about 80 MB, unpacking to about 200 MB, checked against the
hashes in `src/opencode.rs`) to `~/.scoutcode/runtime/opencode`. `src/opencode.rs` runs it as `opencode acp --pure`, one
process per turn, and speaks the Agent Client Protocol to it, through `src/acp.rs` as Kiro does. Its free models need no
account. `opencode console login`, OpenCode's own device sign-in, adds an account's paid models.

- OpenCode runs its own tools, so Scout shows them and answers what it asks permission for, but its edits are outside
  Scout's diffs and undo. `OPENCODE_PERMISSION` makes it ask about everything that isn't only looking.
- `OPENCODE_DISABLE_PROJECT_CONFIG` has to stay on: a repo's `opencode.json` otherwise wins over `OPENCODE_PERMISSION` and
  can turn off every question.
- **A project's own OpenCode code is refused.** OpenCode runs `.opencode/plugin*/*` as each prompt starts, before anything is
  asked, whatever `--pure` and `OPENCODE_DISABLE_PROJECT_CONFIG` say (`packages/core/src/config.ts` has no check as of
  1.19.5), and agents can change what's asked. So `project_code` looks first, every turn, and Scout doesn't start OpenCode
  in a project that has any. Check this again with each new version.
- Scout's copy is separate from any other OpenCode on the PC: its sign-in, settings and history are in
  `~/.scoutcode/runtime/opencode-home`, through the `XDG_*` variables, and every `OPENCODE_*` variable from outside is dropped.
- It brings its own system prompt. Scout sends the conversation as text, with the user's saved instructions. A finished
  session is deleted from its history with `opencode session delete`, since ACP has no call for that.
- When a model refuses, such as past a free limit, the turn ends normally with no text and the reason only in OpenCode's log.

To move to a new version, take it and the SHA-256 digests from
[its GitHub release](https://github.com/anomalyco/opencode/releases), and change `VERSION` and `PACKAGES` in `src/opencode.rs`.
Free models need 1.18.0 or newer. Then run this, which checks all of the above against the real CLI in a throwaway home:
`SCOUT_HOME=<temp> cargo test -- --ignored live_opencode --nocapture`. It can't cover signing in, which needs an account.

### Grok Build

Settings > Account or the model menu downloads xAI's Grok Build CLI (about 50 MB from npm, unpacking to about 150-175 MB, checked
against the hashes in `src/grok.rs`) to `~/.scoutcode/runtime/grok`. `src/grok.rs` runs it as `grok agent --no-leader stdio`, one
process per turn, and speaks the Agent Client Protocol to it, through `src/acp.rs` as Kiro and OpenCode do. It works on a Grok
sign-in, through `grok login --device-auth`, or on an xAI key saved in Scout, passed as `XAI_API_KEY`. With both, the CLI uses
the sign-in.

- Grok Build runs its own tools, so Scout shows them and answers what it asks permission for, but its edits are outside Scout's
  diffs and undo. It doesn't ask about what it counts as read-only, including `mkdir` and `touch`.
- **A project's own Grok code stays off because the folder is never trusted.** Hooks, plugins, MCP and LSP servers, agents,
  skills, permission rules and `AGENTS.md` all wait on folder trust, which only `--trust` or a prompt can grant, and Scout does
  neither. `GROK_FOLDER_TRUST=0` would turn the check off, so every `GROK_*` variable from outside is dropped.
- **The home folder and drive roots are refused.** The CLI treats them as trusted without asking, and a repo's hooks and MCP
  servers really do run there as of 1.0.70, so `start` doesn't run it in them. Check this again with each new version.
- Scout's copy is separate from any other Grok on the PC: its sign-in, settings and history are in
  `~/.scoutcode/runtime/grok-home`, through `GROK_HOME`. `src/grok.rs` rewrites its `config.toml` before every run, which turns
  off Claude Code and Cursor compatibility, updates, telemetry and remembered approvals.
- It still loads the permission rules in `~/.claude/settings.json`, and no setting stops that. As of 1.0.70 they didn't skip
  Scout's question for a shell command, even with `Bash(*)` allowed and `defaultMode` on `bypassPermissions`.
- Refusing a tool ends Grok's turn with `cancelled`, so `acp_run` tells that apart from the user stopping it.
- It brings its own system prompt. Scout sends the conversation as text, with the user's saved instructions. A finished session
  is deleted from its history with `grok sessions delete`, since ACP has no call for that. It also asks the model for a session
  title on each new session, which a key pays for.
- The ACP handshake only offers `xai.api_key` once a request to `/v1/api-key` accepts the key, so a bad key shows up as
  "Authentication required".

To move to a new version, use the `latest` tag of `@xai-official/grok` on npm (`alpha` is ahead of it), hash each
`@xai-official/grok-<platform>` package for `PACKAGES`, and change `VERSION` in `src/grok.rs`. A package's own `latest` tag is
out of date, so ask for the version by name. Then run this, which checks all of the above against the real CLI in a throwaway
home: `SCOUT_HOME=<temp> SCOUT_GROK_TEST_KEY=<key> cargo test -- --ignored live_grok --nocapture`. It can't cover signing in.

### Hermes

Settings > Account or the model menu connects the user's own Nous Portal account, by device sign-in or an API key saved in Scout.
With both, the sign-in is used. Nothing is installed: `src/hermes.rs` handles the sign-in, and requests go straight to
`https://inference-api.nousresearch.com/v1` like any other OpenAI-style provider, so Scout's tools, permissions, diffs and undo
all apply. The model menu lists Nous' public catalog, models with tools only, free ones first.

- **Refresh tokens are single-use.** Using one twice makes Nous revoke the sign-in, so every refresh happens under one lock, from
  what's in `~/.scoutcode/hermes-login.json`, and the new pair is saved before its token is used. Only an explicit refusal ends a
  sign-in. An outage or Nous' firewall (`x-vercel-mitigated`) keeps it. Two running copies of Scout wouldn't share that lock.
- The sign-in uses Hermes' own client ID, `hermes-cli`, since Nous offers no way to register another. The user agent still says
  it's Scout.
- [hermes-agent#47950](https://github.com/NousResearch/hermes-agent/issues/47950) reports that accounts with no plan or credit never
  get an approval button, so the code expires. Scout says so when that happens and offers the key.
- **Scout doesn't use Nous' no-account free tier** (`nous/welcome` on `welcome-api.nousresearch.com`). It's a trial for Hermes'
  own desktop app, once per OS user, behind a browser check. Calling `/api/anonymous/create` from Scout would pose as that app,
  and Nous' terms forbid creating accounts by automation (§10.1).
- Nous may train on what's sent unless Privacy Mode is on for the account (§12.3), so the tab says so.

`cargo test hermes` covers refreshing against a stand-in server, and `cargo test -- --ignored live_hermes --nocapture` reads the
real catalog. Neither can cover signing in or a real chat, which need an account.

### Publishing a release

```powershell
# After bumping `version` in Cargo.toml (Tauri reads it from there):
npm ci
npm run release -- --summary "One line" --changelog "- What changed"
```

This builds the web UI once, then for x64 and ARM64 (the two `*-pc-windows-gnullvm` targets, with llvm-mingw on the PATH), signs both and
uploads them to the public `desktop-releases` bucket in Supabase, then publishes `latest.json`. Each installed copy
fetches its own architecture. The site's download button gives x64, and `/download/windows?arch=arm64` gives ARM64. It
needs `SUPABASE_URL` and `SUPABASE_SERVICE_ROLE_KEY` (set in the environment, or in a `.env` here, which git ignores), and the private key in `~/.scoutcode/desktop-release-key.pem` (or
`SCOUT_RELEASE_KEY`). Keep a backup of that key outside the repo: installed apps accept nothing else, so if it's lost
they can't be updated and everyone has to reinstall. Apps only move to a higher version, so to undo a bad release,
publish the old code under a new number.

## Build and run

```bash
npm --prefix ui ci
npx --prefix ui tauri dev -- -- path/to/project
```

That runs the UI from Vite on port 5183, so changes to `ui/` show at once and only Rust changes rebuild. For a build like
the installed one, run `npm --prefix ui run build` and then `cargo build --release`: the UI in `ui/dist` goes into the
binary, `target/release/scout-desktop.exe`. Arguments:

- `[folder]`: the project to open. Without one it uses the current directory, or the last folder you opened when
  started from a shortcut.
- `--resume <session id>`: reopen a saved conversation (ids are the file names in `~/.scoutcode/sessions`).

## What it does

The agent's tools are `view`, `write_file`, `edit_file` and `multi_edit` or
`apply_patch` depending on the model's `edit_tool` from `/v1/models`,
`list_dir`, `glob`, `grep`, `bash` with background shells, `web_fetch`, `todo_write` and `task` sub-agents. It has permission
modes, plan mode, prompt-cache-friendly system prompt, automatic pruning and compaction, and `/undo`.

- Edits and commands ask in a card above the input, with the diff. "Always" allows all edits for the session, or
  the command prefix for this project (saved to the allow list in `~/.scoutcode`).
- Model, reasoning effort and permission mode are in the top bar; shift+tab cycles the mode. **Auto** runs edits and
  commands without asking, but holds dangerous commands (flagged by the server's rules or by Jev) and edits outside
  the project for review, with the reason on the card.
- Enter sends, shift+enter adds a line, ↑/↓ walk history, esc stops, ctrl+N starts a new chat.
- **Settings > Instructions** saves general preferences for all desktop chats, instructions for the current project,
  and a response language (Auto follows the conversation, or choose Russian and other supported languages).
  These preferences are private to this computer in `~/.scoutcode/instructions.json`; they are kept outside chat
  history and included in the system prompt after compaction too. Project rules override general preferences.
  Saving applies from the next message in new or existing chats. Existing `SCOUT.md`, `AGENTS.md` and `CLAUDE.md`
  files still apply and are refreshed at the start of each turn. The website's settings are separate.
- **Memory** carries facts between chats: `.scout/memory.md` at the repo's root is about the project and can be
  committed, and `~/.scoutcode/memory.md` is about the user. Both go in every prompt (1,200 and 600 tokens at most; a
  file edited past that loses its last sections, with a pointer to them) and are refreshed at the start of each turn.
  The agent is told to keep them current with `update_memory`, which runs under the same permissions as a file edit,
  shows a diff and can be undone. It refuses secrets and text that tries to steer the agent. Kiro and Devin edit the
  files with their own tools instead. You can read and edit both in **Settings > Memory**.
- While Scout works, Enter queues a message: it's sent when the turn ends (unless you stopped Scout). Each queued
  message has a **Steer** button that sends it into the running turn instead, which Scout reads after the step it's
  on; ctrl+enter does that straight away.
- Attach files by dropping them on the window, with the paperclip button, or by typing `@` and picking from the
  project's files. Their contents go in with the message as `<file path="…">` blocks (300 KB at
  most each; a dropped folder sends its listing).
- **Skills** are folders with a `SKILL.md` of instructions, kept in `~/.scoutcode/skills`. Type `@` and a skill's name in
  a message (the picker lists skills before files) and its instructions go along as a `<skill>` block, with the paths of
  its supporting files. Nothing is sent otherwise: Scout doesn't pick skills by itself. **Settings > Skills** uploads a
  folder, a `.zip` or a single `.md`, and searches and installs from the [skills.sh](https://skills.sh) directory.
  Installing only copies files; what a skill then asks for goes through Scout's permissions.
- **MCP servers**: **Settings > MCP servers** connects Scout to servers on this computer (a command, over stdio) or remote
  (a URL, over Streamable HTTP, with any token in a header). They're kept in `~/.scoutcode/mcp.json` in the usual
  `mcpServers` format, so a setup from a server's docs can be pasted in. Their tools reach the model as
  `mcp__server__tool` and are asked about like commands: plan mode refuses them, and "Always" allows that one tool in this
  project. Only Yolo skips asking, and Auto doesn't review them. Sub-agents and Devin don't get them, and `Scout.exe mcp`
  doesn't pass them on. OAuth sign-in and the older SSE transport aren't supported.
- Click the mic (or press Ctrl+Shift+Space) to dictate, and again to stop: what you said goes into the box. The
  recording (16 kHz mono WAV) is sent to ScoutCode's `/v1/audio/transcriptions`, which transcribes it with Deepgram.
- Hover a message of yours and click the pencil to edit it and send it again. The chat is rewound to before it and
  the files Scout changed since are put back (only changes made since the chat was opened in this window are known).
- The sidebar searches every chat's title and text, and each chat's ⋯ menu can delete it. Ctrl+F finds text in the
  open chat.
- When a turn ends or Scout needs an answer while the window is in the background, Scout flashes its taskbar button
  and/or shows a Windows notification. The first run asks which; change it later under Settings in the account menu.
- `/compact [focus]`, `/undo`, `/prune` and `/new` work in the input.
- The icons next to the window buttons open a panel on the right with three tabs (click a lit icon again to
  close it; ctrl+` toggles the terminal):
  - **Terminal**: a real shell (PowerShell 7 if installed, else Windows PowerShell; `SCOUT_TERMINAL` overrides it)
    in a pseudo console and xterm.js, so colors, prompts and full-screen programs work. Drag to select, ctrl+C copies a selection
    (or interrupts without one), ctrl+V pastes, the wheel scrolls back.
  - **Changes**: every file that differs from the last commit, with line counts; click one for its diff,
    double-click to open it.
  - **Files**: the project tree and an editor with syntax colors and line numbers (ctrl+S saves, keeping the file's
    line endings). With room for both, the editor opens beside the tree; in a narrow panel it takes the tree's place.
  It all refreshes when Scout edits files, and an open file with no unsaved edits reloads.
- Sign in with the browser or paste an API key on first run.

## Layout

Rust owns the state and the web UI draws it. `src/core.rs` builds a view of what the window needs and sends only the parts
that changed as `view`, `models` and `items` events; the UI calls back through the commands in `src/bridge.rs`. What's only
about drawing, like open menus and drafts, stays in the UI. Keys don't reach the webview: it gets them masked. Replies are
Markdown without raw HTML, and the CSP stops the page fetching anything.

| File | |
| --- | --- |
| `src/agent.rs` | The agent loop on a worker thread; talks to the core over a channel |
| `src/api.rs` | Streaming chat completions (SSE) with one pooled HTTP connection |
| `src/tools.rs`, `src/patch.rs` | File, search, shell and web tools; `apply_patch` |
| `src/core.rs` | The app's state: the transcript, sign-ins, the queue, and what's sent to the window |
| `src/bridge.rs` | The commands the UI can call |
| `src/dock.rs` | The right panel's backend: the shell in a pseudo console, git, files |
| `src/webview2.rs` | WebView2's loader, built into the exe so that nothing has to sit beside it |
| `src/attach.rs` | Attached files: building and reading back the `<file>` blocks, and the @ picker's file list |
| `src/skills.rs` | Installed skills, uploads and the skills.sh directory |
| `src/mcp_client.rs` | The user's MCP servers, whose tools the model can call (`src/mcp.rs` is the opposite: Scout's tools for chatbots) |
| `src/notify.rs` | Windows notifications |
| `src/update.rs` | Checking for, verifying and installing updates, and installing from `ScoutSetup.exe` |
| `src/config.rs`, `src/session.rs`, `src/prompt.rs`, `src/oauth.rs`, `src/diff.rs` | Settings, saved chats, the system prompt, browser sign-in and diffs |
| `ui/src/store.ts`, `ui/src/api.ts` | What Rust last said, and the shapes it says it in |
| `ui/src/App.tsx`, `Sidebar.tsx`, `Transcript.tsx`, `Composer.tsx`, `ModelMenu.tsx` | The window, the chat and its menus |
| `ui/src/Settings.tsx`, `Accounts.tsx` | Settings, and the sign-ins several screens share |
| `ui/src/Dock.tsx` | The right panel: terminal (xterm.js), changes, files and the editor (CodeMirror) |

`cargo test -- --ignored live_turn --nocapture` runs one real turn against your account in a temporary folder.

## License

[MIT](LICENSE)
