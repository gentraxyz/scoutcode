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

## Experiential Labs

**Experiential Labs** is an additional provider like Google AI Studio: the user's own `xpl_…` key from
platform.experientiallabs.ai, sent to its OpenAI-compatible gateway at `https://api.experientiallabs.ai/v1`.

- **The model list is two reads.** `GET /v1/models` with the key says which slugs it may call, with nothing but ids, so
  `experiential_listing` fills in names, context, vision and prices from the public catalog (`GET /api/models`, paged).
  It keeps active chat models that take tools. Retired, `-batch`, image/video and decision models are left out.
- **Prices** are the cheapest route Experiential pays for (`host_managed`), from its nano-USD fields. A route on the user's
  own provider key (BYOK) bills them at that provider's price instead.
- **Free promotions** (`promotions[].free` in the catalog) are listed again as `<slug>:free`, which Experiential stops at
  the allowance instead of billing. The plain slug of the same model is billed.

**Testing:** `cargo test experiential` covers reading the list, and
`EXPLABS_API_KEY=xpl_… cargo test -- --ignored live_experiential --nocapture` reads the real one.

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

## Local models (llama.cpp)

`src/local.rs` runs GGUF models on the user's own PC with llama.cpp's `llama-server`, which speaks OpenAI's API, so
turns go through `api::stream_chat` like any other provider. The provider id is `local` and model ids are
`local/<id>`. Settings are in `config.json` under `local`.

- **Runtime.** Scout pins one llama.cpp build (`BUILD`, now `b12110`) with a table of each platform's archives and
  GitHub's SHA-256 for each (`ALL_BACKENDS`). The user picks the GPU API: Vulkan, CUDA (two archives: the build and
  `cudart`) or CPU on Windows x64 and Linux; Adreno OpenCL, Vulkan or CPU on Windows on ARM; Metal on macOS. The one
  offered first is the table's first for the platform, or CUDA on a PC with NVIDIA's driver (`has_nvidia`:
  `nvcuda.dll` on Windows, `/proc/driver/nvidia` on Linux). Builds go in `~/.scoutcode/runtime/llama/<build>-<backend>/`.
- **Models.** A short curated list (`CURATED`: Q4_K_M GGUFs from Unsloth, and Tesslate's own for OmniCoder, all
  Apache-2.0) downloads from Hugging Face
  into `~/.scoutcode/models/`, checked against Hugging Face's SHA-256. Downloads come in 32 MB chunks, four at a time
  (one long connection to Hugging Face's CDN gets about a quarter of the line's speed); the finished chunks are listed
  in `<file>.part.done`, so a paused download carries on, and the hash is checked once the file is whole. The user can also add any `.gguf` file, which stays where it is.
- **Quick setup.** What most users see, in onboarding and in the model menu until there's a model: one Set up
  button (`localSetup` → `local::start_setup`) that installs the best build if none is installed, downloads the model
  Scout picked and moves the chat to it (`Ev::LocalModel(m, true)`), as one bar over both. It refuses up front when
  the drive is short (`free_space`: `GetDiskFreeSpaceExW`, or `df -Pk`). Any download without llama.cpp installed goes
  through it too. The pick (`local::recommend`) is by `local::hardware()`: the GPU with the most memory of its own (on
  Windows the display adapters' `HardwareInformation.qwMemorySize` in the registry; on Linux `nvidia-smi`, or amdgpu's
  `mem_info_vram_total`; less than 2 GB counts as no GPU), or on Apple silicon 60% of its memory. `PICKS`: Qwen3.8 27B
  from 20 GB, Gemma 4 12B from 10 GB, Qwen3.5 9B from 7 GB, else Qwen3.5 4B with at least 6 GB of memory, else none.
  Each curated model's `needs_mib` gives its fit (`local::fit`: good on the GPU or, under 4 GB, with 12 GB of memory;
  slow when it only fits with two thirds of the computer's memory added; else too big), shown on every model.
- **Hugging Face.** Under Get more models, a link or `owner/name` (`local::parse_hf`) lists a repo's GGUF files
  (`local_hf_files`: `/api/models/<repo>` for `gated`, then `/tree/main?recursive=1`; `local::hf_tree` leaves out
  `mmproj` files and the later parts of split models, and the first part is shown but can't be downloaded yet). A
  file downloads like a curated model, checked against its `lfs.oid`, into `models/hf/<owner>/<name>/`, with the id
  `hf-<repo>-<file>`. Gated and private repos are refused with a pointer to Add a GGUF file.
- **Where it's shown.** `ui/src/Local.tsx`'s `LocalSetup` has three modes: `onboarding` (quick setup, Scout's other
  models folded under "Choose a different model"), `menu` (quick setup, then Get more models behind the Local tab's
  button) and `settings` (your models, Get more models, and the build, offload, context and small-model settings
  under Advanced).
- **Server.** `local::ensure` starts `llama-server` on a free port on 127.0.0.1 with `-ngl auto` (llama.cpp's `--fit`
  puts as many layers on the GPU as fit) or the user's layer count, `-c <context>`, `-np 1`, `--jinja` (tool calls
  need the model's own template), `--api-key` with a random key made up for each start (llama-server allows every
  origin, so without one any web page in the browser could use it) and `-lv 4` (the level that logs offload). It
  waits for `/health`, reads "offloaded X/Y layers to GPU" from its log for
  Settings, and starts again when the model, layers, context or build changes. With automatic offload, a model that
  runs out of GPU memory while loading starts again leaving 2 GB, then 4 GB, free (`-fitt`): Adreno's OpenCL needs
  room beyond what llama.cpp's fit counts. It stops on Unload, when its model is
  removed, and when Scout quits; one a crashed Scout left running is ended at the next start (`server.pid`).
- **Speed.** More flags: `--cache-reuse 256` (when pruning rewrites an old message, the cache of what follows
  is shifted along, not worked out again), `--ctx-checkpoints 8` and `--checkpoint-min-step` at an eighth of the
  context, no less than 512 (`local::checkpoint_step`; models whose cache can only go back to a checkpoint, such as
  Qwen3.5's recurrent layers and Gemma's sliding window, otherwise get none in a small context and read a changed
  chat again from the start), `--cache-ram` at an eighth of the PC's memory, between 1 and 8 GB (`local::cache_ram`;
  llama.cpp's own 8 GB pushes the model out on a 16 GB PC), `--spec-type ngram-mod` (text the model repeats from the
  chat, like code quoted in an edit, is drafted from the chat and only checked; no draft model) and
  `--reasoning-budget` (`local.reasoningBudget`, 1024 tokens unless changed; 0 is no thinking, -1 no limit).
  `local::preload` loads the model when it's chosen and at startup if the chat is on one, but only when nothing is
  loaded, so a model in use is never swapped out. Starts are one at a time (`ENSURE`). With the chat's system prompt
  and tools (`Agent::local_prefix`), the preload then has llama-server read them (`local::warm`: a request for a reply
  to a stand-in message, dropped as the reply begins), so a first message on a CPU doesn't wait for them; not once a
  message has been sent.
- **Drafting.** llama-server's `timings` for each reply (`StreamResult::timings`) go to `local::observe`, which adds
  up what was drafted, accepted, read and written (`Drafts`). Once 1,500 tokens were drafted and 1,000 read,
  `Drafts::pays` decides: checking a drafted token costs about a prompt token's read (with a quarter more), and an
  accepted one saves writing a token. The answer is kept in `speed.json` (`ngram`), and a server started later goes
  without `ngram-mod` when it didn't pay, which happens on CPUs that read slowly.
- **GPU or CPU.** With automatic offload on a GPU build, a model's first start times a short completion
  (`local::measure`, llama-server's own `timings`) on the GPU and, unless the GPU is clearly quick (`clearly_gpu`), on
  the CPU with `-ngl 0`. The CPU is kept only when a usual step (1,500 tokens read, 300 written) is a tenth quicker
  there, which happens with built-in GPUs such as Adreno. The answer is kept per build and model in
  `runtime/llama/speed.json`; delete the file to measure again. A layer count the user sets is never measured.
- **Expert cache.** When the GPU wins on CUDA or Vulkan, a mixture-of-experts model (`n_expert` in its log) that kept
  more than a sixth of its weights in the computer's memory (its `model buffer size` lines; `local::worth_moe_cache`)
  is started once more with `--moe-cache-mib` at a tenth of the model's size (`local::moe_cache_size`) and measured
  again (`local::try_moe_cache`). It's kept only when a usual step is a tenth quicker, and the answer goes in
  `speed.json` (`moeCache`, 0 when it wasn't worth trying or lost). A model measured before this is tried at its next
  start. Never with a layer count the user sets.
- **Small models.** Only for local models: a tool preset (`none`, `essential`, `standard`, `full`; see
  `local::tool_names`; MCP tools and `task` only in `full`), one tool call per reply (`parallel_tool_calls: false`, and
  extra calls are dropped from the reply), a short system prompt (`prompt::short_system_prompt`), compaction at ¾ and
  pruning at ½ of the context size, and tool output capped to about an eighth of it (reading it is the wait before the next step).

To move to a new llama.cpp build: change `BUILD`, then copy each asset's name, size and `digest` from
`https://api.github.com/repos/ggml-org/llama.cpp/releases/tags/<build>` into `ALL_BACKENDS`. To change a curated
model, take its `size` and `lfs.oid` from `https://huggingface.co/api/models/<repo>/tree/main`, and set `needs_mib`
to the memory it runs in; to change which model setup picks, edit `PICKS`.
