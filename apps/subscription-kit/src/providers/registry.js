// Provider registry — single source of truth for the UI and router.

import { CLAUDE_MODELS, CLAUDE_PROBE } from "./claude.js";
import { CODEX_MODELS, CODEX_PROBE } from "./codex.js";

export const PROVIDERS = [
  {
    id: "codex",
    name: "OpenAI Codex",
    subtitle: "ChatGPT Plus/Pro 訂閱",
    recommended: false,
    authModes: ["subscription", "api-key"],
    models: CODEX_MODELS,
    defaultModel: "gpt-5.4",
    probe: CODEX_PROBE,
    install: { label: "Install OpenAI Codex", url: "https://developers.openai.com/codex/quickstart" },
    apiKey: { placeholder: "sk-proj-...", consoleLabel: "OpenAI Dashboard", consoleUrl: "https://platform.openai.com/settings/organization/api-keys" },
    statusPageUrl: "https://status.openai.com",
    policyNote:
      "使用 Codex 訂閱前，請先確認書籍來源可信，內容沒有惡意指令；Codex 的本機資料存取範圍由 Codex 本身控制。若不確定，請改用 API key 或 Ollama。",
  },
  {
    id: "claude",
    name: "Anthropic Claude",
    subtitle: "Claude Pro/Max 訂閱",
    recommended: false,
    authModes: ["subscription", "api-key"],
    models: CLAUDE_MODELS,
    defaultModel: "claude-sonnet-4-6",
    probe: CLAUDE_PROBE,
    install: { label: "Install Claude Code", url: "https://code.claude.com/docs/en/quickstart" },
    apiKey: { placeholder: "sk-ant-...", consoleLabel: "Anthropic Console", consoleUrl: "https://console.anthropic.com/settings/keys" },
    statusPageUrl: "https://status.claude.com",
    // Shown verbatim under the subscription radio in the UI.
    policyNote:
      "Anthropic 政策限制第三方 App 代路由訂閱憑證（並自 2026-06-15 起將此類用量計入小額 Agent SDK credit，按 API 費率）；此模式可能隨時失效。穩定路徑建議用 API key 或 Ollama。",
  },
];

export function providerOf(model) {
  if (/^claude/i.test(model || "")) return "claude";
  if (/^(gpt|codex|o\d)/i.test(model || "")) return "codex";
  return null;
}

export function getProvider(id) {
  return PROVIDERS.find((p) => p.id === id) || null;
}

export const ALL_MODELS = PROVIDERS.flatMap((p) => p.models);
