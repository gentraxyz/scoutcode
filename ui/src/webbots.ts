// Web chatbots that can use Scout here over MCP, with how to add Scout to each (shown in onboarding and in the model
// menu's Chatbots tab). Claude and Gemini also have an automatic setup (ClaudeSetup.tsx).

/** External chatbots that can use Scout's tools here through the MCP link. */
export interface WebBot {
  id: string;
  name: string;
  logo: string;
  url?: string;
  aliases: string[];
  steps: { title: string; detail: string; imageCaption: string }[];
}

export const WEB_BOTS: WebBot[] = [
  {
    id: 'chatgpt',
    name: 'ChatGPT',
    logo: 'codex',
    url: 'https://chatgpt.com',
    aliases: ['chatgpt', 'openai'],
    steps: [
      { title: 'Enable Developer mode in ChatGPT', detail: 'Open Settings → Apps & Connectors → Advanced settings, and turn on Developer mode.', imageCaption: 'ChatGPT → Settings → Apps & Connectors' },
      { title: 'Create a custom MCP connector', detail: 'Return to Apps & Connectors, click Create, and paste your Scout MCP address below.', imageCaption: 'Paste the Scout MCP URL into the connector dialog' },
      { title: 'Sign in and approve Scout', detail: 'Complete the ScoutCode sign-in prompt in ChatGPT, then ask it to read or edit files in your project.', imageCaption: 'Approve ScoutCode OAuth in your browser' },
    ],
  },
  {
    id: 'gemini',
    name: 'Gemini',
    logo: 'gemini',
    url: 'https://gemini.google.com',
    aliases: ['gemini', 'google'],
    steps: [
      { title: 'Open Gemini Connectors', detail: 'In Gemini, open Customize → Connectors (gemini.google.com/customize/connectors) and click + (Create custom app).', imageCaption: 'Gemini → Customize → Connectors' },
      { title: 'Paste your Scout MCP address', detail: 'Paste the MCP address below as the MCP Server URL and click Next. Leave Additional settings empty.', imageCaption: 'Connect to an MCP server' },
      { title: 'Accept and connect', detail: 'Read Gemini’s notice, tick the box to accept it, click Connect, then sign in to ScoutCode and approve Link.', imageCaption: 'Connect this MCP server' },
    ],
  },
  {
    id: 'grok-web',
    name: 'Grok',
    logo: 'grok',
    url: 'https://grok.com',
    aliases: ['grok', 'xai'],
    steps: [
      { title: 'Open Grok integrations', detail: "In Grok, open Settings → Integrations / Connectors and add a custom MCP server. If there's no such option, your Grok plan doesn't take custom connectors yet: use Claude or ChatGPT instead.", imageCaption: 'Grok → Settings → Custom connector' },
      { title: 'Paste your Scout MCP address', detail: 'Enter Scout as the name and paste the MCP endpoint URL below.', imageCaption: 'Configure Scout MCP endpoint in Grok' },
      { title: 'Sign in and allow access', detail: 'Approve the ScoutCode sign-in prompt and start chatting with your local project.', imageCaption: 'Confirm ScoutCode access' },
    ],
  },
  {
    id: 'claude-web',
    name: 'Claude',
    logo: 'claude',
    url: 'https://claude.ai',
    aliases: ['claude', 'anthropic'],
    steps: [
      { title: 'Open Claude Connectors', detail: 'In claude.ai, open Customize → Connectors (claude.ai/new#customize/connectors) and click Add custom connector.', imageCaption: 'claude.ai → Customize → Connectors' },
      { title: 'Add your Scout MCP URL', detail: 'Name it Scout, paste the MCP address below, and click Add.', imageCaption: 'Add custom connector modal in Claude' },
      { title: 'Connect your ScoutCode account', detail: 'Click Connect next to Scout, sign in to ScoutCode, and allow access to your workspace.', imageCaption: 'Authorize Claude connector with ScoutCode' },
    ],
  },
  {
    id: 'any',
    name: 'Any chatbot',
    logo: 'mcp',
    aliases: [],
    steps: [
      { title: 'Find custom MCP / connector settings', detail: 'Open your chatbot or client settings where remote MCP servers or custom connectors are added.', imageCaption: 'Custom MCP server settings in your client' },
      { title: 'Add the Scout MCP endpoint', detail: 'Create a new connector and paste the Scout MCP URL below.', imageCaption: 'Enter the remote MCP server URL' },
      { title: 'Sign in when prompted', detail: 'Sign in with your ScoutCode account over OAuth. For local apps like Cursor or Claude Desktop, see Settings → Chatbots.', imageCaption: 'Complete OAuth sign-in to link your project' },
    ],
  },
];
