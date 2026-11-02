# Changelog

Each release has a `## <version>` section, newest first, matching `version` in `Cargo.toml`. The first line under the
heading is the one-line summary shown beside "Restart to update"; everything after it is the changelog shown in
Settings > Updates and in "What's new". A release can't be published without its section.

## 0.1.7
Chatbots up top: the MCP chatbots option now sits right below Starred in the model menu, with buttons that open Claude, Gemini and Grok in your browser.

- The MCP chatbots tab moved below Starred and has a chat icon.
- Buttons in the chatbots panel open Claude, Gemini and Grok in your default browser, ready to add the connector.
- The model menu opens with the lists saved last time, and loads your chosen model's provider first.

## 0.1.6
Web search: the agent can now search the web (powered by Exa) and read the pages it finds. Clearer error messages when the server answers with an error page.
