/* CodeChat VS Code webview client. */

"use strict";

const DEFAULT_SUPABASE_URL = "https://hhyrwfzqoszcwfklawjm.supabase.co";
const DEFAULT_SUPABASE_ANON_KEY =
  "sb_publishable_YqXoTDD7nbWCtNphVpwBEw_a-Wj1XqA";

const CHANNEL_NAME = "global-chat";
const MAX_MESSAGES = 100;
const MAX_TEXT_LEN = 300;
const HISTORY_LIMIT = 50;
const REJOIN_DELAY_MS = 3000;
const INVITE_URL = "https://codechat.live";
const EMOJIS = ["😀", "😂", "🤩", "🤔", "👍", "🎉", "❤️", "🔥", "✅", "👀", "🚀", "💻"];

const vscodeApi = acquireVsCodeApi();
const {
  expandEmojiShortcodes,
  normalizeMessage,
  presenceSnapshot,
  truncate,
  validUsername,
} = CodeChatUtils;
const els = {
  statusDot: document.getElementById("status-dot"),
  onlineCount: document.getElementById("online-count"),
  invite: document.getElementById("invite"),
  setup: document.getElementById("setup"),
  setupUsername: document.getElementById("setup-username"),
  setupSave: document.getElementById("setup-save"),
  setupError: document.getElementById("setup-error"),
  messages: document.getElementById("messages"),
  composer: document.getElementById("composer"),
  input: document.getElementById("input"),
  mention: document.getElementById("mention"),
  emoji: document.getElementById("emoji"),
  peopleMenu: document.getElementById("people-menu"),
  emojiMenu: document.getElementById("emoji-menu"),
  editing: document.getElementById("editing"),
  cancelEdit: document.getElementById("cancel-edit"),
};

let config = {};
let client = null;
let channel = null;
let connected = false;
let viewVisible = true;
let presenceTracked = false;
let rejoinTimer = null;
let historyLoaded = false;
let editingMessageId = null;
const onlineUsers = new Set();
const messageRows = new Map();

const presenceKey = crypto.randomUUID
  ? crypto.randomUUID()
  : `vs-${Date.now()}-${Math.floor(Math.random() * 1e9)}`;

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
  while (list.children.length > MAX_MESSAGES) {
    const removed = list.firstChild;
    if (removed.dataset.messageId) messageRows.delete(removed.dataset.messageId);
    removed.remove();
  }
  if (pinned) list.scrollTop = list.scrollHeight;
}

function mentionNames() {
  return [...new Set([config.username, ...onlineUsers].filter(Boolean))]
    .sort((a, b) => b.length - a.length);
}

function appendTextWithMentions(parent, text) {
  const names = mentionNames();
  if (!names.length) {
    parent.appendChild(document.createTextNode(text));
    return;
  }
  const escaped = names.map((name) => name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  const pattern = new RegExp(`(@(?:${escaped.join("|")}))(?=$|[\\s.,!?;:])`, "gi");
  let cursor = 0;
  for (const match of text.matchAll(pattern)) {
    parent.appendChild(document.createTextNode(text.slice(cursor, match.index)));
    const mention = document.createElement("span");
    mention.className = "mention";
    if (match[1].slice(1).toLocaleLowerCase() === String(config.username).toLocaleLowerCase()) {
      mention.classList.add("me");
    }
    mention.textContent = match[1];
    parent.appendChild(mention);
    cursor = match.index + match[0].length;
  }
  parent.appendChild(document.createTextNode(text.slice(cursor)));
}

function insertAtCursor(text) {
  const start = els.input.selectionStart ?? els.input.value.length;
  const end = els.input.selectionEnd ?? start;
  const next = `${els.input.value.slice(0, start)}${text}${els.input.value.slice(end)}`
    .slice(0, MAX_TEXT_LEN);
  els.input.value = next;
  const cursor = Math.min(start + text.length, next.length);
  els.input.focus();
  els.input.setSelectionRange(cursor, cursor);
}

function mentionUser(username) {
  insertAtCursor(`@${username} `);
  closePickers();
}

function buildChatRow(message) {
  const row = document.createElement("div");
  row.className = "msg";
  if (message.clientId && message.clientId === config.clientId) row.classList.add("you");
  if (message.id) {
    row.dataset.messageId = message.id;
    messageRows.set(message.id, row);
  }
  row._message = message;

  const body = document.createElement("span");
  body.className = "body";
  const name = document.createElement("span");
  name.className = "name";
  name.style.color = usernameColor(message.username);
  name.textContent = message.username;
  name.title = `Mention ${message.username}`;
  name.addEventListener("click", () => mentionUser(message.username));
  body.appendChild(name);
  body.appendChild(document.createTextNode(": "));
  const text = document.createElement("span");
  text.className = "text";
  appendTextWithMentions(text, message.text);
  body.appendChild(text);
  if (message.edited) {
    const edited = document.createElement("span");
    edited.className = "edited";
    edited.textContent = " (edited)";
    body.appendChild(edited);
  }

  const time = document.createElement("span");
  time.className = "time";
  time.textContent = formatTime(new Date(message.timestamp));
  row.appendChild(body);
  row.appendChild(time);

  if (message.id && message.clientId === config.clientId) {
    const actions = document.createElement("span");
    actions.className = "actions";
    const edit = document.createElement("button");
    edit.type = "button";
    edit.textContent = "edit";
    edit.title = "Edit message";
    edit.addEventListener("click", () => beginEdit(message.id));
    const remove = document.createElement("button");
    remove.type = "button";
    remove.textContent = "delete";
    remove.title = "Delete message";
    remove.addEventListener("click", () => deleteMessage(message.id));
    actions.append(edit, remove);
    row.appendChild(actions);
  }
  return row;
}

function renderChat(payload) {
  const message = normalizeMessage(payload, MAX_TEXT_LEN);
  if (!message) return;
  if (message.id && messageRows.has(message.id)) return;
  appendRow(buildChatRow(message));
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

function applyMessageEdit(payload) {
  const id = payload?.id == null ? null : String(payload.id);
  const row = id && messageRows.get(id);
  if (!row || typeof payload.text !== "string") return;
  const replacement = normalizeMessage({ ...row._message, text: payload.text, edited: true }, MAX_TEXT_LEN);
  const next = buildChatRow(replacement);
  row.replaceWith(next);
  messageRows.set(id, next);
}

function applyMessageDelete(payload) {
  const id = payload?.id == null ? null : String(payload.id);
  const row = id && messageRows.get(id);
  if (!row) return;
  row.remove();
  messageRows.delete(id);
  if (editingMessageId === id) cancelEdit();
}

function setConnected(value) {
  connected = value;
  els.statusDot.classList.toggle("online", value);
  els.statusDot.title = value ? "Connected" : "Disconnected";
  els.input.disabled = !value;
  els.input.placeholder = value ? "Send a message" : "Reconnecting…";
  if (!value) {
    els.onlineCount.textContent = "–";
    presenceTracked = false;
  }
}

async function loadHistory() {
  if (historyLoaded) return;
  historyLoaded = true;
  const { data, error } = await client
    .from("messages")
    .select("id,username,text,client_id,created_at,edited_at")
    .order("id", { ascending: false })
    .limit(HISTORY_LIMIT);
  if (error || !data) {
    renderSystem("no shared history — live messages only");
    return;
  }
  if (data.length === 0) return;

  const fragment = document.createDocumentFragment();
  for (const raw of data.reverse()) {
    const message = normalizeMessage(raw, MAX_TEXT_LEN);
    if (message && (!message.id || !messageRows.has(message.id))) {
      fragment.appendChild(buildChatRow(message));
    }
  }
  const divider = document.createElement("div");
  divider.className = "msg system";
  divider.textContent = "— you're caught up —";
  fragment.appendChild(divider);
  els.messages.insertBefore(fragment, els.messages.firstChild);
  while (els.messages.children.length > MAX_MESSAGES) els.messages.firstChild.remove();
  els.messages.scrollTop = els.messages.scrollHeight;
}

async function storeMessage(username, text) {
  const { data, error } = await client.rpc("create_message", {
    p_username: username,
    p_text: text,
    p_client_id: config.clientId,
    p_owner_token: config.ownerToken,
  });
  if (!error && Array.isArray(data) && data[0]) {
    return { id: data[0].message_id, createdAt: data[0].message_created_at };
  }

  // Existing self-hosted installations can keep chatting before re-running
  // schema.sql; edit/delete become available after the migration.
  const legacy = await client
    .from("messages")
    .insert({ username, text })
    .select("id,created_at")
    .single();
  if (!legacy.error && legacy.data) {
    return { id: legacy.data.id, createdAt: legacy.data.created_at, legacy: true };
  }
  return null;
}

function connect() {
  const url = config.supabaseUrl || DEFAULT_SUPABASE_URL;
  const key = config.supabaseAnonKey || DEFAULT_SUPABASE_ANON_KEY;
  client = supabase.createClient(url, key, {
    auth: { persistSession: false },
    realtime: { params: { eventsPerSecond: 10 } },
  });
  joinChannel();
}

function syncPresence(ch) {
  if (ch !== channel) return;
  const snapshot = presenceSnapshot(ch.presenceState());
  onlineUsers.clear();
  for (const username of snapshot.users) onlineUsers.add(username);
  els.onlineCount.textContent = String(snapshot.count);
  renderPeopleMenu();
}

async function updatePresence() {
  if (!channel || !connected) return;
  if (viewVisible && !presenceTracked) {
    const result = await channel.track({ username: config.username, joinedAt: Date.now() });
    presenceTracked = result === "ok";
  } else if (!viewVisible && presenceTracked) {
    await channel.untrack();
    presenceTracked = false;
  }
}

function joinChannel() {
  const ch = client.channel(CHANNEL_NAME, {
    config: {
      broadcast: { self: true },
      presence: { key: presenceKey },
    },
  });
  channel = ch;

  ch.on("broadcast", { event: "message" }, ({ payload }) => {
    if (ch === channel) renderChat(payload);
  });
  ch.on("broadcast", { event: "message_edit" }, ({ payload }) => {
    if (ch === channel) applyMessageEdit(payload);
  });
  ch.on("broadcast", { event: "message_delete" }, ({ payload }) => {
    if (ch === channel) applyMessageDelete(payload);
  });
  ch.on("presence", { event: "sync" }, () => syncPresence(ch));
  ch.on("presence", { event: "join" }, () => syncPresence(ch));
  ch.on("presence", { event: "leave" }, () => syncPresence(ch));

  ch.subscribe(async (status) => {
    if (ch !== channel) return;
    if (status === "SUBSCRIBED") {
      setConnected(true);
      await updatePresence();
      loadHistory();
    } else if (["CHANNEL_ERROR", "TIMED_OUT", "CLOSED"].includes(status)) {
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

async function broadcast(event, payload) {
  if (!channel) return false;
  return (await channel.send({ type: "broadcast", event, payload })) === "ok";
}

async function sendMessage(text) {
  const stored = await storeMessage(config.username, text);
  const payload = {
    id: stored?.id,
    username: config.username,
    text,
    clientId: stored?.legacy ? null : config.clientId,
    timestamp: stored?.createdAt ? Date.parse(stored.createdAt) : Date.now(),
  };
  if (!(await broadcast("message", payload))) {
    renderSystem("Message saved, but live delivery failed — reconnecting clients will still see it.");
  }
}

function beginEdit(id) {
  const row = messageRows.get(String(id));
  if (!row) return;
  editingMessageId = String(id);
  els.input.value = row._message.text;
  els.editing.classList.remove("hidden");
  els.input.focus();
  els.input.setSelectionRange(els.input.value.length, els.input.value.length);
}

function cancelEdit() {
  editingMessageId = null;
  els.editing.classList.add("hidden");
  els.input.value = "";
}

async function saveEdit(text) {
  const id = editingMessageId;
  const { data, error } = await client.rpc("edit_message", {
    p_message_id: Number(id),
    p_text: text,
    p_owner_token: config.ownerToken,
  });
  if (error || data !== true) {
    renderSystem("Could not edit that message.");
    return;
  }
  cancelEdit();
  applyMessageEdit({ id, text });
  await broadcast("message_edit", { id, text, editedAt: Date.now() });
}

async function deleteMessage(id) {
  if (!window.confirm("Delete this message for everyone?")) return;
  const { data, error } = await client.rpc("delete_message", {
    p_message_id: Number(id),
    p_owner_token: config.ownerToken,
  });
  if (error || data !== true) {
    renderSystem("Could not delete that message.");
    return;
  }
  applyMessageDelete({ id: String(id) });
  await broadcast("message_delete", { id: String(id) });
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

function closePickers() {
  els.peopleMenu.classList.add("hidden");
  els.emojiMenu.classList.add("hidden");
  els.mention.setAttribute("aria-expanded", "false");
  els.emoji.setAttribute("aria-expanded", "false");
}

function togglePicker(target, button) {
  const opening = target.classList.contains("hidden");
  closePickers();
  target.classList.toggle("hidden", !opening);
  button.setAttribute("aria-expanded", String(opening));
}

function renderPeopleMenu() {
  els.peopleMenu.replaceChildren();
  const users = [...onlineUsers].filter((name) => name !== config.username).sort();
  if (!users.length) {
    const empty = document.createElement("span");
    empty.className = "empty";
    empty.textContent = "No one else is active right now.";
    els.peopleMenu.appendChild(empty);
    return;
  }
  for (const username of users) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = `@${username}`;
    button.addEventListener("click", () => mentionUser(username));
    els.peopleMenu.appendChild(button);
  }
}

for (const emoji of EMOJIS) {
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = emoji;
  button.addEventListener("click", () => {
    insertAtCursor(emoji);
    closePickers();
  });
  els.emojiMenu.appendChild(button);
}

els.setupSave.addEventListener("click", submitSetup);
els.setup.addEventListener("keydown", (event) => {
  if (event.key === "Enter") submitSetup();
});
els.invite.addEventListener("click", () => vscodeApi.postMessage({ type: "copyInvite" }));
els.mention.addEventListener("click", () => togglePicker(els.peopleMenu, els.mention));
els.emoji.addEventListener("click", () => togglePicker(els.emojiMenu, els.emoji));
els.cancelEdit.addEventListener("click", cancelEdit);

let sendTokens = 5;
let sendRefill = Date.now();
function throttleOk() {
  const now = Date.now();
  sendTokens = Math.min(5, sendTokens + (now - sendRefill) / 2000);
  sendRefill = now;
  if (sendTokens < 1) return false;
  sendTokens -= 1;
  return true;
}

els.composer.addEventListener("submit", async (event) => {
  event.preventDefault();
  closePickers();
  const text = truncate(expandEmojiShortcodes(els.input.value.trim()), MAX_TEXT_LEN);
  if (!text || !connected || !channel) return;
  if (editingMessageId) {
    await saveEdit(text);
    return;
  }
  if (!throttleOk()) {
    renderSystem("slow down — you're sending too fast");
    return;
  }
  els.input.value = "";
  await sendMessage(text);
});

window.addEventListener("message", async (event) => {
  const msg = event.data;
  if (msg.type === "config") {
    config = msg.config || {};
    if (validUsername(config.username)) connect();
    else {
      els.setup.classList.remove("hidden");
      els.setupUsername.focus();
    }
  } else if (msg.type === "visibility") {
    viewVisible = Boolean(msg.visible);
    await updatePresence();
  } else if (msg.type === "inviteCopied") {
    renderSystem(`Invite link copied: ${INVITE_URL}`);
  }
});

window.addEventListener("pagehide", () => {
  if (channel && presenceTracked) channel.untrack();
});

vscodeApi.postMessage({ type: "ready" });
