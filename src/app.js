/* CodeChat overlay frontend.
 *
 * Plain JS, no framework, no bundler. Two integrations:
 *   - Tauri IPC  (window.__TAURI__, enabled by withGlobalTauri in tauri.conf.json)
 *     for config load/save and window close.
 *   - Supabase Realtime on channel "global-chat": Broadcast for messages,
 *     Presence for the online counter. Nothing ever touches Postgres —
 *     messages are ephemeral pub/sub and vanish when delivered.
 */

'use strict';

// ---------------------------------------------------------------------------
// Guards & constants
// ---------------------------------------------------------------------------

if (!window.__TAURI__) {
  // Opened in a normal browser instead of the Tauri webview — nothing works.
  document.body.innerHTML =
    '<p style="padding:16px">This page only works inside the CodeChat overlay app.</p>';
  throw new Error('Tauri API not available');
}

const { invoke } = window.__TAURI__.core;
const appWindow = window.__TAURI__.window.getCurrentWindow();

const CHANNEL_NAME = 'global-chat';
const MAX_MESSAGES = 100; // DOM cap — oldest rows are dropped past this
const MAX_TEXT_LEN = 300;
const REJOIN_DELAY_MS = 3000;

// The public CodeChat backend, baked in so every install lands in the same
// worldwide room with zero setup. Supabase *publishable* keys are client-side
// keys designed to be shipped in apps (like a Firebase config) — this is not
// a leaked secret. Power users / self-hosters can still point elsewhere by
// adding supabaseUrl/supabaseAnonKey to ~/.codechat/config.json.
const DEFAULT_SUPABASE_URL = 'https://hhyrwfzqoszcwfklawjm.supabase.co';
const DEFAULT_SUPABASE_ANON_KEY =
  'sb_publishable_YqXoTDD7nbWCtNphVpwBEw_a-Wj1XqA';

const els = {
  statusDot: document.getElementById('status-dot'),
  onlineCount: document.getElementById('online-count'),
  closeBtn: document.getElementById('close-btn'),
  setup: document.getElementById('setup'),
  setupUsername: document.getElementById('setup-username'),
  setupSave: document.getElementById('setup-save'),
  setupError: document.getElementById('setup-error'),
  messages: document.getElementById('messages'),
  composer: document.getElementById('composer'),
  input: document.getElementById('input'),
};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let config = {};   // { username, supabaseUrl, supabaseAnonKey }
let client = null; // Supabase client
let channel = null; // the *current* realtime channel (stale ones are ignored)
let connected = false;
let rejoinTimer = null;

// Presence is keyed per *connection*, not per username: if two people picked
// the same name (or one person opened two overlays), keying by username would
// merge them into one presence entry and undercount. A random key per launch
// keeps the count honest.
const presenceKey = crypto.randomUUID
  ? crypto.randomUUID()
  : `cc-${Date.now()}-${Math.floor(Math.random() * 1e9)}`;

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/* Deterministic username → color. The name is hashed into a hue (0–359) while
 * saturation/lightness stay fixed, so:
 *   - "alice" gets the same color on every client and every session
 *     (the hash has no randomness), and
 *   - every color is readable on the dark background (S/L are pinned to
 *     values that always yield a bright pastel, whatever the hue). */
function usernameColor(name) {
  let hash = 0;
  for (const ch of name) {
    // 31 is the classic string-hash multiplier; >>> 0 keeps it in uint32.
    hash = (hash * 31 + ch.codePointAt(0)) >>> 0;
  }
  return `hsl(${hash % 360} 70% 65%)`;
}

function formatTime(date) {
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

/* Append a row, keeping the DOM capped at MAX_MESSAGES and only auto-scrolling
 * if the user was already reading the newest messages. If they scrolled up to
 * read history, new messages must NOT yank the view back down — so we check
 * "were we pinned to the bottom?" BEFORE appending. */
function appendRow(row) {
  const list = els.messages;
  const pinned =
    list.scrollHeight - list.scrollTop - list.clientHeight < 48;

  list.appendChild(row);
  while (list.children.length > MAX_MESSAGES) {
    list.firstChild.remove();
  }
  if (pinned) {
    list.scrollTop = list.scrollHeight;
  }
}

/* All user-provided strings go through textContent (never innerHTML), so a
 * message like "<img onerror=...>" renders as literal text. */
function renderChat(payload) {
  // The anon key is public by design, so any client could broadcast a
  // malformed or oversized payload. Validate the shape and clamp lengths
  // instead of trusting the sender.
  if (!payload || typeof payload.username !== 'string' || typeof payload.text !== 'string') {
    return;
  }
  const username = payload.username.slice(0, 20) || 'anon';
  const text = payload.text.slice(0, MAX_TEXT_LEN);
  const when =
    typeof payload.timestamp === 'number' ? new Date(payload.timestamp) : new Date();

  const row = document.createElement('div');
  row.className = 'msg';
  if (username === config.username) row.classList.add('you');

  const body = document.createElement('span');
  body.className = 'body';

  const name = document.createElement('span');
  name.className = 'name';
  name.style.color = usernameColor(username);
  name.textContent = username;

  body.appendChild(name);
  body.appendChild(document.createTextNode(': ' + text));

  const time = document.createElement('span');
  time.className = 'time';
  time.textContent = formatTime(when);

  row.appendChild(body);
  row.appendChild(time);
  appendRow(row);
}

function renderSystem(text) {
  const row = document.createElement('div');
  row.className = 'msg system';
  const body = document.createElement('span');
  body.className = 'body';
  body.textContent = text;
  row.appendChild(body);
  appendRow(row);
}

function setConnected(value) {
  connected = value;
  els.statusDot.classList.toggle('online', value);
  els.statusDot.title = value ? 'Connected' : 'Disconnected';
  els.input.disabled = !value;
  els.input.placeholder = value ? 'Send a message' : 'Reconnecting…';
  if (!value) els.onlineCount.textContent = '–';
}

// ---------------------------------------------------------------------------
// Supabase realtime
// ---------------------------------------------------------------------------

function connect() {
  if (!window.supabase) {
    renderSystem(
      'Could not load the Supabase library from the CDN. ' +
        'Check your internet connection, then restart the overlay.'
    );
    return;
  }

  // Config-file values (self-hosters) win over the baked-in public backend.
  const url = config.supabaseUrl || DEFAULT_SUPABASE_URL;
  const key = config.supabaseAnonKey || DEFAULT_SUPABASE_ANON_KEY;

  client = window.supabase.createClient(url, key, {
    auth: { persistSession: false }, // no login flow — the anon key is enough
    realtime: { params: { eventsPerSecond: 10 } },
  });

  joinChannel();
}

function joinChannel() {
  // `ch` is captured in every callback below; comparing `ch !== channel`
  // filters out events from an old channel we already abandoned during a
  // reconnect (removeChannel() fires a final CLOSED that must not trigger
  // yet another rejoin).
  const ch = client.channel(CHANNEL_NAME, {
    config: {
      // self: true → our own broadcasts echo back to us, giving one single
      // render path for everyone's messages (ours included).
      broadcast: { self: true },
      presence: { key: presenceKey },
    },
  });
  channel = ch;

  ch.on('broadcast', { event: 'message' }, ({ payload }) => {
    if (ch !== channel) return;
    renderChat(payload);
  });

  // 'sync' fires after the initial join, after every join/leave, AND after a
  // reconnect re-syncs state — recomputing the count here is what keeps the
  // online number correct when the network blips.
  ch.on('presence', { event: 'sync' }, () => {
    if (ch !== channel) return;
    els.onlineCount.textContent = String(Object.keys(ch.presenceState()).length);
  });

  ch.subscribe(async (status) => {
    if (ch !== channel) return;

    if (status === 'SUBSCRIBED') {
      setConnected(true);
      // track() announces us to Presence. It must re-run on every successful
      // (re)join — which is exactly why it lives in this callback and not in
      // connect(): after a drop, the rejoined channel re-tracks automatically.
      await ch.track({ username: config.username, joinedAt: Date.now() });
    } else if (status === 'CHANNEL_ERROR' || status === 'TIMED_OUT' || status === 'CLOSED') {
      setConnected(false);
      scheduleRejoin();
    }
  });
}

/* The supabase-js client already reconnects its WebSocket with backoff and
 * rejoins channels. This extra layer handles the cases it doesn't: a channel
 * stuck in CHANNEL_ERROR/TIMED_OUT is torn down and recreated from scratch. */
function scheduleRejoin() {
  if (rejoinTimer) return; // one pending rejoin at a time
  rejoinTimer = setTimeout(() => {
    rejoinTimer = null;
    const old = channel;
    channel = null; // makes every `ch !== channel` guard reject the old channel
    if (old) client.removeChannel(old);
    joinChannel();
  }, REJOIN_DELAY_MS + Math.floor(Math.random() * 2000)); // jitter: don't stampede
}

async function sendMessage(text) {
  const resp = await channel.send({
    type: 'broadcast',
    event: 'message',
    payload: {
      username: config.username,
      text,
      timestamp: Date.now(),
    },
  });
  // resp is 'ok' | 'timed out' | 'error'. We don't locally echo on failure —
  // with broadcast.self=true a rendered message means it really went out.
  if (resp !== 'ok') {
    renderSystem('Message failed to send (connection hiccup) — try again.');
  }
}

// ---------------------------------------------------------------------------
// First-run setup
// ---------------------------------------------------------------------------

function validUsername(name) {
  return typeof name === 'string' && name.length >= 2 && name.length <= 20;
}

function showSetup() {
  els.setup.classList.remove('hidden');
  els.setupUsername.value = config.username || '';
  els.setupUsername.focus();
}

async function submitSetup() {
  els.setupError.textContent = '';

  const username = els.setupUsername.value.trim();
  if (!validUsername(username)) {
    els.setupError.textContent = 'Username must be 2–20 characters.';
    return;
  }

  // Spread keeps any self-host overrides the user hand-added to the file.
  const merged = { ...config, username };
  try {
    await invoke('save_config', { config: merged });
  } catch (err) {
    els.setupError.textContent = 'Could not save config: ' + err;
    return;
  }

  config = merged;
  els.setup.classList.add('hidden');
  renderSystem(`Welcome, ${username}! You're in the worldwide chat.`);
  connect();
}

// ---------------------------------------------------------------------------
// Wiring & boot
// ---------------------------------------------------------------------------

els.closeBtn.addEventListener('click', () => appWindow.close());

els.composer.addEventListener('submit', (event) => {
  event.preventDefault(); // Enter submits the form; never navigate
  const text = els.input.value.trim().slice(0, MAX_TEXT_LEN);
  if (!text || !connected || !channel) return;
  els.input.value = '';
  sendMessage(text);
});

els.setupSave.addEventListener('click', submitSetup);
// Enter inside any setup field submits too.
els.setup.addEventListener('keydown', (event) => {
  if (event.key === 'Enter') submitSetup();
});

async function boot() {
  let stored = null;
  try {
    stored = await invoke('load_config'); // null on first ever run
  } catch (err) {
    // e.g. hand-edited config with a JSON typo — tell the user instead of
    // silently overwriting their file.
    renderSystem('Config problem: ' + err);
    renderSystem('Fix ~/.codechat/config.json and restart the overlay.');
    return;
  }

  config = stored || {};

  if (!validUsername(config.username)) {
    showSetup();
  } else {
    connect();
  }
}

boot();
