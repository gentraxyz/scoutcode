# Changelog

Each release has a `## <version>` section, newest first, matching `version` in `Cargo.toml`. The first line under the
heading is the one-line summary shown beside "Restart to update"; everything after it is the changelog shown in
Settings > Updates and in "What's new". A release can't be published without its section.

## 0.1.20
DeepSeek as a provider, with a refreshed model menu and Settings.

- Use DeepSeek models directly through the API, or through the DeepSeek web sign-in, from the model menu.
- Model menu, Settings, Accounts, Games and onboarding polish to match the new provider.

## 0.1.19
A built-in music player, the Agent Router that picks a model to fit each request, and clearer denied-action messages.

- Turn on the music player in Settings to play your own MP3s or SoundCloud tracks and playlists while Scout works.
- The Agent Router asks Jev how capable a model each request needs and starts at the right step, with an option to wait and retry on rate limits before moving on.
- Denied commands and edits now say what you denied ("You denied this command"), and each reply remembers the model and provider that wrote it.
- Model menu, Settings, Delta and onboarding polish, plus a smoother in-place install that no longer has to close Scout first.

## 0.1.18
Discord account linking in Rewards, terminal command title refinements, and tool execution improvements.

- Link your Discord account in Settings > Usage (or Rewards) to earn a one-time $0.25 balance reward.
- Improved ACP terminal title detection and live execution state for background commands.
- Model menu and accounts view refinements and polish across Delta issues and docs.

## 0.1.17
New onboarding walkthrough, 30 language localizations, and chat imports from Claude Code and Codex.

- A welcoming multi-step onboarding experience introduces Scout, guides setup, and configures key options.
- 30 languages supported across the interface and model response preferences.
- Automatically detects previous conversations from Claude Code and Codex and lets you import them with one click.
- Fresh installs default to a dedicated ~/Scout folder instead of opening the user home directory directly.
- Polish and styling fixes across Delta, Settings, Composer, and widget controls.

## 0.1.16
Daily login rewards and referrals, Delta docs and pictures, a built-in folder picker, and fixes for Fallbacks and large skills.

- Claim a daily login reward in Settings > Usage to add to your permanent balance, and share your referral code with up to 5 friends. New accounts have 3 days to redeem a referral code for $5 in credit.
- Delta now has Notion-style Docs with nested pages, a "/" menu, Markdown shortcuts and export, pasted or dropped pictures in docs, issues and comments, and right-click menus for issues, projects and cycles.
- Add projects with Scout's own folder picker instead of the Windows dialog, with places, drives, filter-as-you-type and keyboard navigation.
- Desktop sign-ins now get $5 of credit per week ($10 with ScoutPass) with 5-hour windows, and live provider quota meters show what's left across your providers.
- Fixed the model menu in Settings > Fallbacks going above the top of the window, and large skills on GitHub failing with "too large to install" because of extra media or assets.

## 0.1.15
Delta, an issue tracker built into Scout, with smooth animations throughout.

- Delta is in the sidebar: teams, issues, projects, cycles and a roadmap, in a list or on a board. Keep a workspace on this computer or in ScoutCode Cloud, and hand an issue to Scout to work on.
- Delta moves smoothly: menus, dialogs and the issue panel ease in and out, pages fade between views, groups fold open and shut, and cards lift as you drag them. With reduced motion on in Windows, it stays still.

## 0.1.14
An Intelligence Index in the model menu, hiding providers, and long commands that keep running.

- The model menu shows one to three bars by each model: the more bars, the smarter it is. Sort by "Smartest first" to put the strongest models on top.
- Right-click a provider in the model menu to hide it, or to sign in if it needs an account. Show hidden ones again from the "…" button under the providers.
- A command that takes longer than Scout expected keeps running in the background. Scout sees its last lines and decides whether to wait longer, change something, or get on with other work, and it hears when the command finishes.
- ScoutMote can switch the model and star models on this PC.

## 0.1.13
ScoutMote, file links in replies, glass window looks and new logos.

- ScoutMote: turn it on in Settings > ScoutMote, then use Scout on this PC from the ScoutCode website in any browser. Start chats, watch them work, answer what Scout asks and stop it.
- Files Scout mentions in its replies are links. Click one to open the file in the Files tab, at the line it points to.
- The model menu can sort by cheapest, most expensive, name or context, and show only free models. Cheapest comes first.
- OpenRouter's models no longer show as Free just because ScoutCode's copy of the same model is free, or the other way round.
- Settings > Appearance has window materials (Mica, Acrylic and Liquid Glass) and nine versions of the Scout logo, which the taskbar icon follows.
- With no chat open, the message box sits in the middle of the window.

## 0.1.12
Sign-in pages in the dark look, ScoutCode in Providers, and free models without an account.

- The page you see after signing in to Scout is dark now, like the rest of the site.
- ScoutCode is in Settings > Providers, with sign-in, free models, or an API key in one place. Settings > Providers also scrolls through a long list.
- Without signing in, ScoutCode's free models are listed in the model menu, with a suggestion to sign in for more.
- Codex has the ChatGPT logo in the model menu.

## 0.1.11
A game to play while Scout works, and a quieter gray look.

- Turn on Games in Settings > Experimental to play 2048 while Scout works. It can float over the chat, where you drag it wherever you like, or sit in a Games tab in the side panel. It can open when Scout starts and close when it finishes, and your game is kept for next time.
- The window has a calmer gray look, the attach button sits at the left of the message box, and the account menu shows your name and Scout's version.

## 0.1.10
A /plan command, experimental image generation, and file paths that work however a model writes them.

- Type / in the message box to see the commands. /plan followed by a task asks Scout to write a short plan before it starts.
- Turn on Settings > Experimental to let Scout make images (icons, illustrations, mockups) and save them in your project. It asks first, like any edit, and never replaces a file.
- On Windows, the file tools now understand Git Bash paths (/c/Users/…), paths that start with / meaning the project, ~ for your home folder, and paths whose capitals don't match the file. Models are also told which path style each tool expects.

## 0.1.9
Your own Claude Code and Codex plans, a Providers page for linking and unlinking every provider, and an optional Discord status.

- Sign in with your Claude Pro or Max plan to use Claude Code, or with your ChatGPT plan to use Codex, from the model menu or Settings > Providers. Scout installs its own copy of each, kept apart from any other on your PC.
- Settings > Providers lists every provider, linked ones first. Open one to sign in, add a key, sign out or unlink it. Settings > Account now only covers your ScoutCode account.
- Show “Using Scout” on your Discord profile, and optionally the open folder's name, from Settings > Discord. It's off unless you turn it on.

## 0.1.8
Smoother and sturdier: the app animates and feels more polished, falls back to another model when one fails, and takes pasted images.

- Menus, dialogs, messages, tool output and cards ease in, and buttons and rows ease on hover and press. With reduced motion turned on in Windows, Scout doesn't animate.
- Buttons, inputs, toggles and panels have more depth, and the message box glows gently while Scout is working.
- When a model fails, Scout can switch to another one. The first time, it asks you to set a fallback list for every model, one for just that model, or to only show the error. Edit them in Settings > Fallbacks.
- Paste an image from the clipboard into the chat with Ctrl+V.

## 0.1.7
Chatbots up top: the MCP chatbots option now sits right below Starred in the model menu, with buttons that open Claude, Gemini and Grok in your browser.

- The MCP chatbots tab moved below Starred and has a chat icon.
- Buttons in the chatbots panel open Claude, Gemini and Grok in your default browser, ready to add the connector.
- The model menu opens with the lists saved last time, and loads your chosen model's provider first.

## 0.1.6
Web search: the agent can now search the web (powered by Exa) and read the pages it finds. Clearer error messages when the server answers with an error page.
