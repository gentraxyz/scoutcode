# Scout Desktop

A native desktop app for Scout, separate from the CLI. It's written in Rust with [egui](https://github.com/emilk/egui):
no webview, no Node and no bundled browser. It ships as one ~11 MB executable, opens its window in under half a
second and uses about 90 MB of memory. It redraws only when something changes (or to blink the cursor), so it is
close to idle while you read.

It shares the CLI's login, settings, allow list and sessions (`~/.scoutcode`), so a chat started in one can be
continued in the other.

## Install

```powershell
powershell -ExecutionPolicy Bypass -File gui\install.ps1
```

This builds the release binary, copies it to `%LOCALAPPDATA%\Programs\Scout\Scout.exe`, and adds a Start menu entry,
so Windows search finds "Scout". It also lists Scout under Settings > Apps, where you can uninstall it. Run the script
again to update. Add `-Desktop` for a desktop shortcut, or `-Uninstall` to remove everything except your chats and
login in `~/.scoutcode`. Launched from the Start menu, Scout opens the last folder you opened (your home folder the
first time); use "Open folder…" to switch.

## Build and run

```bash
cd gui
cargo run --release -- path/to/project
```

The binary is `target/release/scout-desktop.exe`. Arguments:

- `[folder]`: the project to open. Without one it uses the current directory, or the last folder you opened when
  started from a shortcut.
- `--resume <session id>`: reopen a saved conversation (ids are the file names in `~/.scoutcode/sessions`).

It draws with OpenGL. Set `SCOUT_RENDERER=wgpu` to use DirectX 12/Vulkan instead, which takes about 3× the
memory but works on machines without OpenGL drivers.

## What it does

The same agent as the CLI, ported to Rust: the same tools (`view`, `write_file`, `edit_file` and `multi_edit` or
`apply_patch` depending on the model's `edit_tool` from `/v1/models`,
`list_dir`, `glob`, `grep`, `bash` with background shells, `web_fetch`, `todo_write`, `task` sub-agents), the same
permission modes, plan mode, prompt-cache-friendly system prompt, automatic pruning and compaction, and `/undo`.

- Edits and commands ask in a card above the input, with the diff. "Always" allows all edits for the session, or
  the command prefix for this project (saved to the same allow list the CLI uses).
- Model, reasoning effort and permission mode are in the top bar; shift+tab cycles the mode. **Auto** runs edits and
  commands without asking, but holds dangerous commands (flagged by the server's rules or by Jev) and edits outside
  the project for review, with the reason on the card.
- Enter sends, shift+enter adds a line, ↑/↓ walk history, esc stops, ctrl+N starts a new chat.
- While Scout works, Enter queues a message: it's sent when the turn ends (unless you stopped Scout). Each queued
  message has a **Steer** button that sends it into the running turn instead, which Scout reads after the step it's
  on; ctrl+enter does that straight away.
- Attach files by dropping them on the window, with the paperclip button, or by typing `@` and picking from the
  project's files. Their contents go in with the message as `<file path="…">` blocks, like the CLI's (300 KB at
  most each; a dropped folder sends its listing).
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
    in a pseudo console, so colors, prompts and full-screen programs work. Drag to select, ctrl+C copies a selection
    (or interrupts without one), ctrl+V pastes, the wheel scrolls back.
  - **Changes**: every file that differs from the last commit, with line counts; click one for its diff,
    double-click to open it.
  - **Files**: the project tree and an editor with syntax colors and line numbers (ctrl+S saves, keeping the file's
    line endings). With room for both, the editor opens beside the tree; in a narrow panel it takes the tree's place.
  It all refreshes when Scout edits files, and an open file with no unsaved edits reloads.
- Sign in with the browser or paste an API key on first run.

## Layout

| File | |
| --- | --- |
| `src/agent.rs` | The agent loop on a worker thread; talks to the window over a channel |
| `src/api.rs` | Streaming chat completions (SSE) with one pooled HTTP connection |
| `src/tools.rs`, `src/patch.rs` | File, search, shell and web tools; `apply_patch` |
| `src/app.rs` | The window: transcript (only on-screen items are laid out), composer, permission and plan cards |
| `src/markdown.rs` | Markdown for replies |
| `src/attach.rs` | Attached files: building and reading back the `<file>` blocks, and the @ picker's file list |
| `src/notify.rs` | Windows notifications |
| `src/dock/` | The right panel: `terminal.rs` (ConPTY + vt100), `changes.rs` (git), `files.rs` (tree and editor), `highlight.rs` |
| `src/config.rs`, `src/session.rs`, `src/prompt.rs`, `src/oauth.rs`, `src/diff.rs` | Ports of the CLI's modules |

`cargo test -- --ignored live_turn --nocapture` runs one real turn against your account in a temporary folder.
