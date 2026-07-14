/* CodeChat webview client.
 *
 * Same room, same protocol, same message shape as the terminal client
 * (tui/) — just running inside a VS Code webview. Config (username, and
 * optional backend overrides) comes from ~/.codechat/config.json via the
 * extension host, so terminal and VS Code share one identity.
 */

"use strict";

// The public CodeChat backend. Publishable keys are client-side identifiers,
// designed to be shipped in apps. Keep in sync with tui/src/main.rs.
const DEFAULT_SUPABASE_URL = "https://hhyrwfzqoszcwfklawjm.supabase.co";
const DEFAULT_SUPABASE_ANON_KEY =
  "sb_publishable_YqXoTDD7nbWCtNphVpwBEw_a-Wj1XqA";

const CHANNEL_NAME = "global-chat";
const MAX_MESSAGES = 100;
const MAX_TEXT_LEN = 300;
const HISTORY_LIMIT = 50;
const REJOIN_DELAY_MS = 3000;

const vscodeApi = acquireVsCodeApi();

const els = {
  statusDot: document.getElementById("status-dot"),
  onlineCount: document.getElementById("online-count"),
  setup: document.getElementById("setup"),
  setupUsername: document.getElementById("setup-username"),
  setupSave: document.getElementById("setup-save"),
  setupError: document.getElementById("setup-error"),
  messages: document.getElementById("messages"),
  composer: document.getElementById("composer"),
  input: document.getElementById("input"),
};

let config = {};
let client = null;
let channel = null;
let connected = false;
let rejoinTimer = null;
let historyLoaded = false;

// Presence is keyed per webview instance so duplicate usernames still count.
const presenceKey = crypto.randomUUID
  ? crypto.randomUUID()
  : `vs-${Date.now()}-${Math.floor(Math.random() * 1e9)}`;

// ---------------------------------------------------------------------------
// Rendering (same deterministic name→color hash as every other client)
// ---------------------------------------------------------------------------

function usernameColor(name) {
  let hash = 0;
  for (const ch of name) hash = (hash * 31 + ch.codePointAt(0)) >>> 0;
  return `hsl(${hash % 360} 70% 55%)`;
}

function formatTime(date) {
  return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function appendRow(row) {
  const list = els.messages;
  const pinned = list.scrollHeight - list.scrollTop - list.clientHeight < 48;
  list.appendChild(row);
  while (list.children.length > MAX_MESSAGES) list.firstChild.remove();
  if (pinned) list.scrollTop = list.scrollHeight;
}

function buildChatRow(username, text, when) {
  const row = document.createElement("div");
  row.className = "msg";
  if (username === config.username) row.classList.add("you");

  const body = document.createElement("span");
  body.className = "body";
  const name = document.createElement("span");
  name.className = "name";
  name.style.color = usernameColor(username);
  name.textContent = username;
  body.appendChild(name);
  body.appendChild(document.createTextNode(": " + text));

  const time = document.createElement("span");
  time.className = "time";
  time.textContent = formatTime(when);

  row.appendChild(body);
  row.appendChild(time);
  return row;
}

function renderChat(payload) {
  if (!payload || typeof payload.username !== "string" || typeof payload.text !== "string") return;
  const username = payload.username.slice(0, 20) || "anon";
  const text = payload.text.slice(0, MAX_TEXT_LEN);
  const when = typeof payload.timestamp === "number" ? new Date(payload.timestamp) : new Date();
  appendRow(buildChatRow(username, text, when));
}

function renderSystem(text) {
  const row = document.createElement("div");
  row.className = "msg system";
  const body = document.createElement("span");
  body.className = "body";
  body.textContent = text;
  row.appendChild(body);
  appendRow(row);
}

function setConnected(value) {
  connected = value;
  els.statusDot.classList.toggle("online", value);
  els.statusDot.title = value ? "Connected" : "Disconnected";
  els.input.disabled = !value;
  els.input.placeholder = value ? "Send a message" : "Reconnecting…";
  if (!value) els.onlineCount.textContent = "–";
}

// ---------------------------------------------------------------------------
// Shared history — read once per launch, right after the first join.
// ---------------------------------------------------------------------------

async function loadHistory() {
  if (historyLoaded) return;
  historyLoaded = true;
  const { data, error } = await client
    .from("messages")
    .select("username,text,created_at")
    .order("id", { ascending: false })
    .limit(HISTORY_LIMIT);
  if (error || !data) {
    renderSystem("no shared history — live messages only");
    return;
  }
  if (data.length === 0) return;

  // Oldest first, inserted ABOVE anything that already streamed in live.
  const fragment = document.createDocumentFragment();
  for (const row of data.reverse()) {
    fragment.appendChild(
      buildChatRow(String(row.username).slice(0, 20), String(row.text).slice(0, MAX_TEXT_LEN), new Date(row.created_at))
    );
  }
  const divider = document.createElement("div");
  divider.className = "msg system";
  divider.textContent = "— you're caught up —";
  fragment.appendChild(divider);
  els.messages.insertBefore(fragment, els.messages.firstChild);
  while (els.messages.children.length > MAX_MESSAGES) els.messages.firstChild.remove();
  els.messages.scrollTop = els.messages.scrollHeight;
}

function storeMessage(username, text) {
  // Fire and forget: the broadcast already delivered it live.
  client.from("messages").insert({ username, text }).then(() => {}, () => {});
}

// ---------------------------------------------------------------------------
// Realtime
// ---------------------------------------------------------------------------

function connect() {
  const url = config.supabaseUrl || DEFAULT_SUPABASE_URL;
  const key = config.supabaseAnonKey || DEFAULT_SUPABASE_ANON_KEY;
  client = supabase.createClient(url, key, {
    auth: { persistSession: false },
    realtime: { params: { eventsPerSecond: 10 } },
  });
  joinChannel();
}

function joinChannel() {
  const ch = client.channel(CHANNEL_NAME, {
    config: {
      broadcast: { self: true }, // our sends echo back → one render path
      presence: { key: presenceKey },
    },
  });
  channel = ch;

  ch.on("broadcast", { event: "message" }, ({ payload }) => {
    if (ch !== channel) return;
    renderChat(payload);
  });

  ch.on("presence", { event: "sync" }, () => {
    if (ch !== channel) return;
    els.onlineCount.textContent = String(Object.keys(ch.presenceState()).length);
  });

  ch.subscribe(async (status) => {
    if (ch !== channel) return;
    if (status === "SUBSCRIBED") {
      setConnected(true);
      await ch.track({ username: config.username, joinedAt: Date.now() });
      loadHistory();
    } else if (status === "CHANNEL_ERROR" || status === "TIMED_OUT" || status === "CLOSED") {
      setConnected(false);
      scheduleRejoin();
    }
  });
}

function scheduleRejoin() {
  if (rejoinTimer) return;
  rejoinTimer = setTimeout(() => {
    rejoinTimer = null;
    const old = channel;
    channel = null;
    if (old) client.removeChannel(old);
    joinChannel();
  }, REJOIN_DELAY_MS + Math.floor(Math.random() * 2000));
}

async function sendMessage(text) {
  const resp = await channel.send({
    type: "broadcast",
    event: "message",
    payload: { username: config.username, text, timestamp: Date.now() },
  });
  if (resp !== "ok") {
    renderSystem("Message failed to send — try again.");
    return;
  }
  storeMessage(config.username, text);
}

// ---------------------------------------------------------------------------
// Setup + wiring
// ---------------------------------------------------------------------------

function validUsername(name) {
  return typeof name === "string" && name.length >= 2 && name.length <= 20;
}

function submitSetup() {
  els.setupError.textContent = "";
  const username = els.setupUsername.value.trim();
  if (!validUsername(username)) {
    els.setupError.textContent = "Username must be 2–20 characters.";
    return;
  }
  config.username = username;
  vscodeApi.postMessage({ type: "saveUsername", username });
  els.setup.classList.add("hidden");
  renderSystem(`Welcome, ${username}!`);
  connect();
}

els.setupSave.addEventListener("click", submitSetup);
els.setup.addEventListener("keydown", (e) => {
  if (e.key === "Enter") submitSetup();
});

// Anti-spam token bucket: a short burst is fine, sustained spam is not.
let sendTokens = 5, sendRefill = Date.now();
function throttleOk() {
  const now = Date.now();
  sendTokens = Math.min(5, sendTokens + (now - sendRefill) / 2000);
  sendRefill = now;
  if (sendTokens < 1) return false;
  sendTokens -= 1;
  return true;
}

els.composer.addEventListener("submit", (e) => {
  e.preventDefault();
  const text = els.input.value.trim().slice(0, MAX_TEXT_LEN);
  if (!text || !connected || !channel) return;
  if (!throttleOk()) { renderSystem("slow down — you're sending too fast"); return; }
  els.input.value = "";
  sendMessage(text);
});

window.addEventListener("message", (event) => {
  const msg = event.data;
  if (msg.type !== "config") return;
  config = msg.config || {};
  if (validUsername(config.username)) {
    connect();
  } else {
    els.setup.classList.remove("hidden");
    els.setupUsername.focus();
  }
});

vscodeApi.postMessage({ type: "ready" });
