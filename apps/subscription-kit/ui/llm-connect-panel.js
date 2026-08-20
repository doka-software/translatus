// <llm-connect-panel> — framework-agnostic Web Component implementing the
// Pencil-style "Connect your AI subscription or key" flow against a
// llm-subscription-kit runner.
//
// Usage:
//   <script type="module" src=".../llm-connect-panel.js"></script>
//   <llm-connect-panel runner-url="http://127.0.0.1:8765"></llm-connect-panel>
//
// Events (bubbling, composed):
//   connect-changed  detail: { provider, status, mode }   — probe result landed
//   provider-picked  detail: { provider, model }          — user confirmed a working provider
//
// Theming: CSS custom properties (all optional) —
//   --lsk-bg, --lsk-card-bg, --lsk-fg, --lsk-muted, --lsk-border,
//   --lsk-accent, --lsk-radius, --lsk-font
// Hosts restyle freely by overriding these custom properties.
//
// UX decisions carried from the teardown (tech-design/PENCIL-UX-TEARDOWN.md):
//   - zero-key subscription radio + instant real probe (Pencil §0)
//   - 4-state chip: checking(pulse)/connected/not-connected/none (§3)
//   - two-step setup: install+login link, then "Authenticate with:" (§4)
//   - beyond Pencil: layered diagnosis line, >10s Keychain hint, policy note (§7-8)

const STATUS_LABEL = {
  checking: "檢查中…",
  connected: "已連接",
  "not-connected": "未連接",
};

// HTML-escape every runtime value before it goes into innerHTML. Probe results
// carry provider/SDK error strings (untrusted); without this an error body like
// `<img onerror=…>` would execute in the host page (XSS). All `${}`
// interpolations of dynamic data below MUST pass through esc().
function esc(v) {
  return String(v == null ? "" : v)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}
// For href values: escape AND only allow http(s); otherwise drop to "#".
function escUrl(v) {
  const s = String(v == null ? "" : v);
  return /^https?:\/\//i.test(s) ? esc(s) : "#";
}
// Exported for tests only.
export { esc, escUrl };

const tpl = document.createElement("template");
tpl.innerHTML = `
<style>
  :host {
    display: block;
    font-family: var(--lsk-font, -apple-system, "PingFang TC", sans-serif);
    color: var(--lsk-fg, #1c1c1c);
    background: var(--lsk-bg, transparent);
  }
  * { box-sizing: border-box; }
  h3 { margin: 0 0 6px; font-size: 22px; font-weight: 600; letter-spacing: -0.3px; }
  .sub { margin: 0 0 18px; font-size: 13px; line-height: 1.6; color: var(--lsk-muted, #767676); max-width: 56ch; }
  .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 12px; }
  .card {
    display: flex; flex-direction: column; justify-content: space-between; gap: 14px;
    min-height: 150px; padding: 16px; text-align: left; cursor: pointer;
    border: 1px solid var(--lsk-border, #d9d4cb); border-radius: var(--lsk-radius, 10px);
    background: var(--lsk-card-bg, rgba(255,255,255,.55)); transition: border-color .15s ease;
  }
  .card:hover, .card:focus-visible { border-color: var(--lsk-accent, #2f2f2f); outline: none; }
  .card h4 { margin: 0; font-size: 15px; font-weight: 600; white-space: pre-line; }
  .card .reco { font-size: 10px; color: var(--lsk-muted, #767676); letter-spacing: .08em; }
  .chip {
    display: inline-flex; align-items: center; gap: 6px; width: fit-content;
    padding: 2px 9px; border-radius: 999px; font-size: 10.5px; font-weight: 500;
    border: 1px solid var(--lsk-border, #d9d4cb); color: var(--lsk-muted, #767676);
  }
  .chip .dot { width: 6px; height: 6px; border-radius: 50%; background: currentColor; }
  .chip[data-s="connected"]     { color: #166534; border-color: rgba(22,101,52,.25); background: rgba(22,101,52,.07); }
  .chip[data-s="not-connected"] { color: #92400e; border-color: rgba(146,64,14,.25); background: rgba(146,64,14,.07); }
  .chip[data-s="checking"] .dot { animation: lsk-pulse 1.1s ease-in-out infinite; }
  @keyframes lsk-pulse { 50% { opacity: .25; } }
  .setupbtn {
    padding: 8px 0; text-align: center; font-size: 13px; border-radius: 8px;
    border: 1px solid var(--lsk-border, #d9d4cb); background: transparent; color: inherit;
  }
  .back { background: none; border: 0; padding: 0; font-size: 12px; color: var(--lsk-muted, #767676); cursor: pointer; }
  .back:hover { color: inherit; }
  .head { display: flex; align-items: center; gap: 10px; margin: 14px 0 16px; }
  .head h4 { margin: 0; font-size: 17px; font-weight: 600; }
  .step { font-size: 13px; margin: 14px 0 6px; font-weight: 600; }
  .steptext { font-size: 13px; color: var(--lsk-fg, #1c1c1c); }
  .steptext a { color: inherit; }
  label.radio { display: flex; align-items: center; gap: 8px; font-size: 13px; padding: 5px 0; cursor: pointer; }
  .policy {
    margin: 8px 0 0 24px; padding: 8px 10px; font-size: 11.5px; line-height: 1.6;
    color: #92400e; background: rgba(146,64,14,.06); border: 1px solid rgba(146,64,14,.18);
    border-radius: 8px; max-width: 60ch;
  }
  .keyrow { display: flex; gap: 8px; margin: 8px 0 0 24px; max-width: 420px; }
  .keyrow input { flex: 1; padding: 6px 9px; font-size: 12px; border: 1px solid var(--lsk-border, #d9d4cb); border-radius: 7px; background: transparent; color: inherit; }
  .keyrow button { padding: 6px 14px; font-size: 12px; border-radius: 7px; border: 1px solid var(--lsk-border, #d9d4cb); background: transparent; color: inherit; cursor: pointer; }
  .hintline { margin: 10px 0 0; font-size: 12px; line-height: 1.6; color: var(--lsk-muted, #767676); max-width: 60ch; }
  .hintline.err { color: #92400e; }
  .diag { margin: 6px 0 0; font-size: 11.5px; color: var(--lsk-muted, #767676); }
  .diag span { margin-right: 12px; }
  .recheck { background: none; border: 0; cursor: pointer; font-size: 12px; color: var(--lsk-muted, #767676); text-decoration: underline; padding: 0; }
  .checked-at { font-size: 10.5px; color: var(--lsk-muted, #767676); margin-left: 8px; }
</style>
<div id="root"></div>
`;

export class LlmConnectPanel extends HTMLElement {
  static get observedAttributes() {
    return ["runner-url", "runner-token"];
  }

  constructor() {
    super();
    this.attachShadow({ mode: "open" }).appendChild(tpl.content.cloneNode(true));
    this._view = { kind: "picker" };
    this._providers = [];
    this._status = {}; // id -> { status, result }
    this._mode = {}; // id -> "subscription" | "api-key"
    this._keys = {}; // id -> apiKey (held in memory only; host may persist)
    this._checkingSince = {};
    this._hintTimer = null;
  }

  get runnerUrl() {
    return this.getAttribute("runner-url") || "http://127.0.0.1:8765";
  }

  get runnerToken() {
    return this.getAttribute("runner-token") || "";
  }

  // Webview hosts whose fetch can't reach localhost cleanly (CORS, custom
  // schemes) inject `window.__lskFetch(path, init) -> Promise<{status, json}>`
  // — e.g. a Tauri command proxy. Falls back to plain fetch.
  async _req(path, init = {}) {
    const headers = new Headers(init.headers || {});
    if (this.runnerToken) headers.set("Authorization", `Bearer ${this.runnerToken}`);
    const request = { ...init, headers };
    if (typeof window.__lskFetch === "function") {
      return window.__lskFetch(path, request);
    }
    const r = await fetch(`${this.runnerUrl}${path}`, request);
    return { status: r.status, json: await r.json() };
  }

  // Probe results persist across opens (localStorage): reopening settings
  // shows the cached verdict + its timestamp instead of burning a probe each
  // time. Auto-probe only when a provider has never been checked.
  _loadCache() {
    try {
      return JSON.parse(localStorage.getItem("lsk-probe-cache") || "{}");
    } catch {
      return {};
    }
  }
  _saveCache(id, status, result) {
    const c = this._loadCache();
    c[id] = { status, checkedAt: Date.now(), hint: result && result.hint, mode: this._mode[id] };
    localStorage.setItem("lsk-probe-cache", JSON.stringify(c));
  }

  async connectedCallback() {
    try {
      const r = await this._req("/v1/providers");
      this._providers = r.json.providers;
      for (const p of this._providers) this._mode[p.id] = p.authModes[0];
    } catch {
      this._providers = [];
    }
    const cache = this._loadCache();
    for (const p of this._providers) {
      const c = cache[p.id];
      if (c && c.status) {
        this._status[p.id] = { status: c.status, result: { hint: c.hint }, checkedAt: c.checkedAt, cached: true };
        if (c.mode) this._mode[p.id] = c.mode;
      }
    }
    this._render();
    // 只有「從未檢查過」的 provider 才自動 probe；其餘顯示上次結果＋時間。
    for (const p of this._providers) {
      if (!this._status[p.id]) this._probe(p.id);
    }
  }

  disconnectedCallback() {
    clearInterval(this._hintTimer);
  }

  async _probe(id) {
    this._status[id] = { status: "checking" };
    this._checkingSince[id] = Date.now();
    this._render();
    clearInterval(this._hintTimer);
    // Past ~10s a silent spinner is indistinguishable from a hang; surface the
    // most likely cause (macOS Keychain prompt) while we wait.
    this._hintTimer = setInterval(() => this._render(), 4000);
    try {
      const headers = { "Content-Type": "application/json" };
      const key = this._mode[id] === "api-key" ? this._keys[id] : undefined;
      if (key) headers["X-LLM-API-Key"] = key;
      const r = await this._req("/v1/auth/probe", {
        method: "POST",
        headers,
        body: JSON.stringify({ provider: id }),
      });
      const result = r.json;
      this._status[id] = { status: result.status, result, checkedAt: Date.now() };
      this._saveCache(id, result.status, result);
    } catch (e) {
      this._status[id] = { status: "not-connected", result: { error: { reason: "runner", message: String(e) } }, checkedAt: Date.now() };
    }
    clearInterval(this._hintTimer);
    this._render();
    const s = this._status[id];
    this.dispatchEvent(
      new CustomEvent("connect-changed", {
        bubbles: true,
        composed: true,
        detail: { provider: id, status: s.status, mode: this._mode[id] },
      }),
    );
    if (s.status === "connected") {
      const p = this._providers.find((x) => x.id === id);
      this.dispatchEvent(
        new CustomEvent("provider-picked", {
          bubbles: true,
          composed: true,
          detail: { provider: id, model: p?.defaultModel },
        }),
      );
    }
  }

  _setMode(id, mode) {
    this._mode[id] = mode;
    // Selecting a radio acts immediately — no save button (Pencil pattern).
    if (mode === "subscription" || this._keys[id]) this._probe(id);
    else this._render();
  }

  _saveKey(id, value) {
    if (!value.trim()) return;
    this._keys[id] = value.trim();
    this._probe(id);
  }

  _fmtTime(ts) {
    if (!ts) return "";
    const d = new Date(ts);
    const today = new Date().toDateString() === d.toDateString();
    const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
    return today ? hm : `${d.getMonth() + 1}/${d.getDate()} ${hm}`;
  }

  _chip(id) {
    const st = this._status[id];
    const s = st?.status;
    if (!s) return "";
    const long =
      s === "checking" && Date.now() - (this._checkingSince[id] || 0) > 10_000
        ? `<div class="hintline">仍在檢查… 若系統跳出鑰匙圈授權視窗，請允許。</div>`
        : "";
    const when =
      s !== "checking" && st.checkedAt
        ? `<span class="checked-at">上次檢查 ${esc(this._fmtTime(st.checkedAt))}</span>`
        : "";
    return `<span class="chip" data-s="${esc(s)}"><span class="dot"></span>${esc(STATUS_LABEL[s] ?? s)}</span>${when}${long}`;
  }

  _render() {
    const root = this.shadowRoot.getElementById("root");
    if (!this._providers.length) {
      root.innerHTML = `<p class="sub">無法連到本機 runner（${esc(this.runnerUrl)}）。請先啟動 llm-subscription-kit。</p>`;
      return;
    }
    if (this._view.kind === "picker") {
      root.innerHTML = `
        <h3>連接你的 AI 訂閱或金鑰</h3>
        <p class="sub">直接使用你既有的 Codex/ChatGPT 或 Claude Code，也可以改用 API key。你的憑證只留在這台機器上，不會傳給我們。</p>
        <div class="cards">
          ${this._providers
            .map(
              (p) => `
            <button class="card" data-id="${esc(p.id)}">
              <div>
                <h4>${esc(p.name)}</h4>
                <div class="reco">${p.recommended ? "推薦 · " : ""}${esc(p.subtitle)}</div>
                <div style="margin-top:10px">${this._chip(p.id)}</div>
              </div>
              <span class="setupbtn">設定</span>
            </button>`,
            )
            .join("")}
        </div>`;
      root.querySelectorAll(".card").forEach((el) =>
        el.addEventListener("click", () => {
          this._view = { kind: "setup", id: el.dataset.id };
          this._render();
        }),
      );
      return;
    }

    const p = this._providers.find((x) => x.id === this._view.id);
    const st = this._status[p.id] || {};
    const res = st.result || {};
    const diag = res.diagnosis;
    root.innerHTML = `
      <button class="back">‹ 返回</button>
      <div class="head"><h4>${esc(p.name)}</h4>${this._chip(p.id)} <button class="recheck">重新檢查</button></div>
      <div class="step">步驟 1</div>
      <div class="steptext"><a href="${escUrl(p.install.url)}" target="_blank" rel="noopener">${esc(p.install.label)}</a> 並完成登入</div>
      <div class="step">步驟 2 · 認證方式</div>
      ${p.authModes
        .map(
          (m) => `
        <label class="radio">
          <input type="radio" name="auth" value="${esc(m)}" ${this._mode[p.id] === m ? "checked" : ""}>
          ${m === "subscription" ? `使用你的 ${esc(p.name)} 既有設定（例如訂閱）` : "API key"}
        </label>
        ${m === "subscription" && p.policyNote ? `<div class="policy">⚠ ${esc(p.policyNote)}</div>` : ""}`,
        )
        .join("")}
      ${
        this._mode[p.id] === "api-key"
          ? `<div class="keyrow">
               <input type="password" placeholder="${esc(p.apiKey.placeholder)}">
               <button class="savekey">儲存</button>
             </div>
             <div class="hintline">API Key · <a href="${escUrl(p.apiKey.consoleUrl)}" target="_blank" rel="noopener">${esc(p.apiKey.consoleLabel)}</a></div>`
          : ""
      }
      ${
        st.status === "not-connected"
          ? `<div class="hintline err">${esc(res.hint || res.error?.message || "")}</div>
             ${
               diag
                 ? `<div class="diag">
                      <span>執行環境 ${diag.runtime?.ok ? "✓" : "✗"}</span>
                      <span>本機憑證 ${diag.credentials?.ok ? "✓" : "✗"}</span>
                      <span>連線測試 ✗（${esc(res.error?.reason || "?")}）</span>
                    </div>`
                 : ""
             }`
          : ""
      }
      ${st.status === "connected" ? `<div class="hintline">已可使用（${res.mode === "subscription" ? "訂閱" : "API key"} · ${esc(res.elapsedMs)}ms）</div>` : ""}
    `;
    root.querySelector(".back").addEventListener("click", () => {
      this._view = { kind: "picker" };
      this._render();
    });
    root.querySelector(".recheck").addEventListener("click", () => this._probe(p.id));
    root.querySelectorAll('input[name="auth"]').forEach((el) =>
      el.addEventListener("change", () => this._setMode(p.id, el.value)),
    );
    const save = root.querySelector(".savekey");
    if (save) {
      const input = root.querySelector('.keyrow input');
      save.addEventListener("click", () => this._saveKey(p.id, input.value));
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") this._saveKey(p.id, input.value);
      });
    }
  }
}

customElements.define("llm-connect-panel", LlmConnectPanel);
