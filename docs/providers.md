# Providers

How Scout connects to each outside provider, what it installs, and how to move each one to a new version. Every
download is checked against SHA-256 hashes pinned in the source before anything runs, and everything goes in
`~/.scoutcode/runtime`.

Providers that run their own agent (Claude Code, Codex, Devin, Kiro, OpenCode and Grok Build) are started one process per turn and spoken
to over the [Agent Client Protocol](https://agentclientprotocol.com) (`src/acp.rs`). They bring their own system
prompt; Scout sends the conversation as text, with the user's saved instructions.

## Codex and Claude Code

Settings > Providers, the model menu or the first-run screen signs in to the user's own ChatGPT plan (Codex) or Claude Pro or
Max plan (Claude Code). Both agents only ship as npm packages, so `src/node.rs` first downloads Scout's own Node.js (about
30 MB, unpacking to about 100 MB, checked against the hashes in `src/node.rs`) to `~/.scoutcode/runtime/node`. Each agent is
then installed with `npm ci` from a lockfile Scout ships, `src/npm/codex` and `src/npm/claude`, which pins every package and
its SHA-512. npm refuses anything that doesn't match, no install scripts run, and the user's and machine's `.npmrc` and
`NODE_*`/`npm_config_*` variables are ignored. Codex comes to about 420 MB and Claude Code to about 370 MB.

- Both run through their Agent Client Protocol adapters, `@agentclientprotocol/codex-acp` (which starts `codex app-server`)
  and `@agentclientprotocol/claude-agent-acp` (which drives the Claude Code CLI bundled with the Agent SDK). They run their own
  tools, so Scout shows them and answers what they ask permission for, but their edits are outside Scout's diffs and undo.
- Sign-in runs the agent's own login, `codex login` or `claude auth login --claudeai`, which opens the browser. Scout passes on
  the page it prints in case the browser didn't open, and never sees the token.
- Scout's copies are separate from any other Codex or Claude Code on the PC: their sign-ins, settings and sessions are in
  `~/.scoutcode/runtime/codex-home` (`CODEX_HOME`) and `~/.scoutcode/runtime/claude-home` (`CLAUDE_CONFIG_DIR`). Every
  `CODEX_*`, `OPENAI_*`, `CLAUDE_*` and `ANTHROPIC_*` variable from outside is dropped, so an API key or another endpoint can't
  take over from the sign-in.
- **Modes.** Codex: Ask first and Plan run read-only (Plan also turns on its plan collaboration mode), Accept edits runs
  `workspace-write`, Auto runs its auto review, and Yolo runs with full access. Claude Code: its `default`, `acceptEdits`,
  `plan`, `auto` and `bypassPermissions` modes. A mode the session doesn't offer falls back to asking (Claude Code) or stops the
  turn (Codex), never to more access.
- **A project's own Claude Code code stays off.** Each session is made with `settingSources: []` and `strictMcpConfig`, so a
  repo's `.claude/settings.json`, hooks, plugins and `.mcp.json` don't load, and with `persistSession: false`. Codex only loads a
  project's `.codex` settings and hooks for a trusted project, and nothing is trusted in Scout's home.

**Updating:** regenerate a lockfile with `npm install --ignore-scripts --save-exact <adapter>@<version>` in an empty folder,
copy `package.json` and `package-lock.json` into `src/npm/<agent>` (keeping the `scout-<agent>` name), and change `version` in
`src/codex.rs` or `src/claude.rs`. `cargo test the_lockfiles_match_their_packages` checks every package is pinned, from npm, and
has no install script. For Node, take the version and hashes from `https://nodejs.org/dist/latest-v24.x/SHASUMS256.txt` for
`VERSION` and `PACKAGES` in `src/node.rs`. Then run `SCOUT_HOME=<temp> cargo test -- --ignored live_node_agents --nocapture`.
Read the adapters' changelogs for changes to their config option ids (`model`, `reasoning_effort`, `effort`, `mode`,
`collaboration_mode`) and session `_meta`.

## GitHub Copilot

Settings > Providers signs in to the user's own GitHub account with Scout's OAuth App (device flow). That also downloads
GitHub's runtime (about 150 MB, checked against the hashes in `src/copilot.rs`) to `~/.scoutcode/runtime`, which needs no
Node. Scout's tools and permissions do the work; the runtime's own tools are off.

**Updating:** change the `github-copilot-sdk` version in `Cargo.toml` together with `VERSION` and `PACKAGES` in
`src/copilot.rs`. The first build also downloads the runtime to a per-user cache: the crate's build script does it, and
Scout's binary doesn't contain it.

## Devin

Settings > Providers downloads Cognition's Devin CLI (about 75 MB, unpacking to about 200 MB, checked against the hashes in
`src/devin.rs`) to `~/.scoutcode/runtime/devin` and signs in with the browser. `src/devin.rs` runs it as `devin acp`.
Devin works in the user's own account and plan.

- Scout offers ACP's file and terminal calls, so Devin's reads, writes and commands come back here and go through Scout's
  permissions, diffs and undo. Searches and any web or MCP tools run inside Devin; Scout only shows them, and asks first
  when Devin asks for permission.
- Devin reads `AGENTS.md` and its own settings.
- The CLI keeps the token and Scout never sees it. On Windows nothing moves where it's kept (`XDG_*` and `APPDATA` are
  ignored, and overriding `USERPROFILE` stops the CLI), so the sign-in is shared with any other Devin CLI on the PC.

**Updating:** take the version and the archive URLs from the
[ACP registry](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json), then change `VERSION` and hash each
archive for `ARCHIVES` in `src/devin.rs`. Read `devin acp --help`, and run `cargo test -- --ignored live_devin --nocapture`
while signed in: the CLI's ACP extensions aren't documented, so a new version can change them.

## Kiro

Settings > Providers downloads AWS's Kiro CLI (about 330 MB on Windows, unpacking to about 470 MB, checked against the hashes
in `src/kiro.rs`) to `~/.scoutcode/runtime/kiro` and signs in with the browser. On Windows the only package is an MSI, which
is unpacked with `msiexec /a` and never installed. `src/kiro.rs` runs it as `kiro-cli acp --agent-engine=v3`. Kiro works
in the user's own account and plan.

- Kiro runs its own tools and offers no file or terminal calls, so Scout shows them and answers what it asks permission
  for, but its edits are outside Scout's diffs and undo. Each session turns its `autopilot` off, so that it asks first,
  unless the mode is Yolo. Each also turns `contentCollection` off.
- The CLI keeps the token and Scout never sees it. The sign-in is shared with any other Kiro CLI on the PC.

**Updating:** take the version and the hashes from Kiro's
[release manifest](https://prod.download.cli.kiro.dev/stable/latest/manifest.json), and change `VERSION` and `PACKAGES` in
`src/kiro.rs`. Read `kiro-cli acp --help` and the [ACP migration guide](https://kiro.dev/docs/cli/v3/acp-migration/), and run
`cargo test -- --ignored live_kiro --nocapture` while signed in.

## OpenCode

Settings > Providers or the model menu downloads OpenCode's CLI (about 80 MB, unpacking to about 200 MB, checked against the
hashes in `src/opencode.rs`) to `~/.scoutcode/runtime/opencode`. `src/opencode.rs` runs it as `opencode acp --pure`. Its free
models need no account. `opencode console login`, OpenCode's own device sign-in, adds an account's paid models.

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
- A finished session is deleted from its history with `opencode session delete`, since ACP has no call for that.
- When a model refuses, such as past a free limit, the turn ends normally with no text and the reason only in OpenCode's log.

**Updating:** take the version and the SHA-256 digests from
[its GitHub release](https://github.com/anomalyco/opencode/releases), and change `VERSION` and `PACKAGES` in `src/opencode.rs`.
Free models need 1.18.0 or newer. Then run this, which checks all of the above against the real CLI in a throwaway home:
`SCOUT_HOME=<temp> cargo test -- --ignored live_opencode --nocapture`. It can't cover signing in, which needs an account.

## Grok Build

Settings > Providers or the model menu downloads xAI's Grok Build CLI (about 50 MB from npm, unpacking to about 150-175 MB,
checked against the hashes in `src/grok.rs`) to `~/.scoutcode/runtime/grok`. `src/grok.rs` runs it as
`grok agent --no-leader stdio`. It works on a Grok sign-in, through `grok login --device-auth`, or on an xAI key saved in
Scout, passed as `XAI_API_KEY`. With both, the CLI uses the sign-in.

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
- A finished session is deleted from its history with `grok sessions delete`, since ACP has no call for that. It also asks
  the model for a session title on each new session, which a key pays for.
- The ACP handshake only offers `xai.api_key` once a request to `/v1/api-key` accepts the key, so a bad key shows up as
  "Authentication required".

**Updating:** use the `latest` tag of `@xai-official/grok` on npm (`alpha` is ahead of it), hash each
`@xai-official/grok-<platform>` package for `PACKAGES`, and change `VERSION` in `src/grok.rs`. A package's own `latest` tag is
out of date, so ask for the version by name. Then run this, which checks all of the above against the real CLI in a throwaway
home: `SCOUT_HOME=<temp> SCOUT_GROK_TEST_KEY=<key> cargo test -- --ignored live_grok --nocapture`. It can't cover signing in.

## Hermes

Settings > Providers or the model menu connects the user's own Nous Portal account, by device sign-in or an API key saved in Scout.
With both, the sign-in is used. Nothing is installed: `src/hermes.rs` handles the sign-in, and requests go straight to
`https://inference-api.nousresearch.com/v1` like any other OpenAI-style provider, so Scout's tools, permissions, diffs and undo
all apply. The model menu lists Nous' public catalog, models with tools only, free ones first.

- **Refresh tokens are single-use.** Using one twice makes Nous revoke the sign-in, so every refresh happens under one lock, from
  what's in `~/.scoutcode/hermes-login.json`, and the new pair is saved before its token is used. Only an explicit refusal ends a
  sign-in. An outage or Nous' firewall (`x-vercel-mitigated`) keeps it. Two running copies of Scout wouldn't share that lock.
- The sign-in uses Hermes' own client ID, `hermes-cli`. The user agent still says it's Scout.
- [hermes-agent#47950](https://github.com/NousResearch/hermes-agent/issues/47950) reports that accounts with no plan or credit never
  get an approval button, so the code expires. Scout says so when that happens and offers the key.
- **Scout doesn't use Nous' no-account free tier** (`nous/welcome` on `welcome-api.nousresearch.com`). It's a trial for Hermes'
  own desktop app, once per OS user, behind a browser check, and Nous' terms forbid creating accounts by automation (§10.1).
- Nous may train on what's sent unless Privacy Mode is on for the account (§12.3), so the tab says so.

**Testing:** `cargo test hermes` covers refreshing against a stand-in server, and
`cargo test -- --ignored live_hermes --nocapture` reads the real catalog. Neither can cover signing in or a real chat,
which need an account.

## Cohere

**Cohere** is an additional provider like Google AI Studio: the user's own key from dashboard.cohere.com (a free trial key
works, rate-limited with a monthly cap), sent to Cohere's OpenAI-compatible API at `https://api.cohere.ai/compatibility/v1`.

- **The model list comes from Cohere's own API** (`GET https://api.cohere.com/v1/models?endpoint=chat`), which the
  compatibility API doesn't document. `cohere_listing` keeps the models whose `features` include tool use.
- **No `reasoning_effort`.** Cohere takes only `none` and `high` and refuses it on models that don't reason, so Scout
  leaves it out there.

**Testing:** `cargo test cohere` covers reading the list. A real chat needs a key.

## DeepSeek API and DeepSeek Web

**DeepSeek API** is an additional provider like Google AI Studio: the user's own key from platform.deepseek.com, sent
straight to `https://api.deepseek.com/v1`, OpenAI-style, so Scout's tools, permissions, diffs and undo all apply.

**DeepSeek Web** (`src/deepseek.rs`) uses the user's own chat.deepseek.com account, with no key, through the website. It's
desktop only, off until turned on in Settings > Providers, and its settings are in `~/.scoutcode/deepseek-web.json`.

- **A driver script runs in the page** (`src/deepseek/driver.js`). For each request it opens the right DeepSeek chat, types a
  short line into the composer and sends it like the user would, so the page does its own sign-in and proof-of-work. As
  the page's `/api/v0/chat/completion` request goes out, the driver swaps in Scout's full prompt and the mode
  (`thinking_enabled`, `search_enabled`), and reads a copy of the streamed answer. The model menu's four entries are
  those modes: DeepSeek, DeepThink, and each with Search.
- **Scout runs the tool loop.** The website has no tool calling, so the first message of each DeepSeek chat lists Scout's
  tools (and the user's MCP servers') and asks for calls in DSML, DeepSeek's own function-call format
  (`<｜DSML｜invoke name="…">` with `<｜DSML｜parameter name="…" string="true|false">`), which its models write by habit
  anyway. `dsml_calls` reads them, and earlier calls go back to it written the same way. `<tool_call>` blocks are still
  read, through `tools::recover_calls`. Results go back as `<tool_result>` blocks. A Scout conversation stays in one
  DeepSeek chat while its history matches (the system prompt isn't compared), so each step only sends what's new.
- **Two places it can run.**
  - *In Scout:* a hidden window, `deepseek`, on chat.deepseek.com with the driver as its initialization script. It has no
    capability, so the page can't call Scout's IPC. Scout starts jobs and reads events by `eval`, polling
    `window.__scoutDS.drain()`. Closing the window hides it, which keeps the sign-in. Background throttling is off.
  - *Browser extension* (`src/deepseek/extension`, MV3, Chrome or Edge): Settings writes it to
    `~/.scoutcode/deepseek-extension` with `scout.json` (port and token) for Load unpacked. `driver.js` and `main.js` run
    in the page's main world, `bridge.js` links them to the background worker, and the worker long-polls
    `POST http://127.0.0.1:47615/deepseek/sync` (or another free port, written to `scout.json`). The server answers only
    requests with the token in `X-Scout-Token` and no web-page `Origin`. It only starts while the extension mode is on.
    The extension's `claude.js` is also the fallback for Claude's "Add to Claude for me" setup (claude_setup.rs). On
    Windows that setup first runs without it (claude_native.rs): UI Automation finds claude.ai's buttons and fields by
    name in the browser window in front. Scout's own cursor (`setup-cursor`, a click-through window) moves to each one,
    and the real cursor hops there only for the click and straight back. Keys are typed with the real keyboard, and
    buttons below the fold are scrolled to (the form's Add on its second page). Low-level input hooks hold back the
    user's clicks, wheel and keys over the browser while it works (no admin needed); Esc, the `setup-bar` window's
    Pause, or leaving the browser pauses it and gives the browser back. The setup card picks the browser: the default
    one unless another is chosen, from those Windows lists under `StartMenuInternet` (Internet Explorer left out). A
    chosen browser is started with the address, and only windows whose program is in its folder are worked. When no
    browser with claude.ai comes to the front within 25 seconds, the job goes to the extension.
- **The warning.** DeepSeek's terms don't allow automated use of the website, so the account could be limited or banned, and
  Settings says so before it can be turned on. It also breaks when DeepSeek changes the page or its stream format. The
  stream reader follows the formats seen so far (OpenAI-style `choices`, and the JSON-patch `{p, o, v}` updates with and
  without `fragments`).

**Testing:** `cargo test deepseek` covers the prompt bookkeeping and reading tool calls back. Nothing can test the page
without a signed-in account: after a change, sign in, pick DeepSeek Web, and ask for something that needs a tool.
