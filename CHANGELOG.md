# Changelog

Each release has a `## <version>` section, newest first, matching `version` in `Cargo.toml`. The first line under the
heading is the one-line summary shown beside "Restart to update"; everything after it is the changelog shown in
Settings > Updates and in "What's new". A release can't be published without its section.

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
