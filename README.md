# CodeChat

A worldwide chat room that lives **inside your terminal, right next to Claude
Code**. Run `codechat` instead of `claude` and your terminal splits: Claude
Code keeps almost all the space, and a slim chat strip appears on the right —
everyone running CodeChat, anywhere on Earth, is in that one room.

Coding with an AI is powerful but quiet. CodeChat exists so it doesn't have
to be lonely: while Claude works, there's a little crowd of fellow builders
from around the world talking in the corner of your terminal.

```
┌────────────────────────────────────────────┬──────────────┐
│                                            │ ▌CodeChat 12●│
│                                            │ 14:02 alice: │
│   Claude Code — full height, ~85% width,   │   anyone got │
│   exactly as it looks today                │   tauri v2   │
│                                            │   working?   │
│                                            │ 14:03 bob:   │
│                                            │   yes! check │
│                                            │   the docs   │
│                                            │              │
│                                            │ > _          │
└────────────────────────────────────────────┴──────────────┘
```

- **One global room.** No accounts, no channels, no setup. Pick a username
  once and you're in. The public backend is baked into the binary.
- **Claude Code is untouched.** It runs exactly as normal in its own pane.
  Close the chat any time with `Ctrl-C` inside it — Claude Code instantly
  expands to full width.
- **You join mid-conversation, not an empty room.** The last 50 messages
  load above a "— you're caught up —" marker; scroll up to read them.
  (Live delivery is ephemeral pub/sub; only a small rolling window of ~1000
  recent messages is kept, in the operator's Supabase table.)
- **One small binary.** The chat client is pure Rust — no Node, no GTK,
  no system libraries.
- **tmux is required.** That's the one hard dependency — it's what puts both
  programs in one terminal window. `sudo apt install tmux` / `brew install tmux`.

Works on **Linux** and **macOS**.

---

## Quick install (from GitHub)

```bash
# 1. tmux (the one dependency)
sudo apt install tmux        # macOS: brew install tmux

# 2. CodeChat — downloads a prebuilt binary from the latest release
curl -fsSL https://raw.githubusercontent.com/YOURUSER/codechat/main/install.sh | bash

# 3. go
codechat
```

> Replace `YOURUSER` with the actual GitHub account once the repo is
> published. No Rust needed — the installer grabs a prebuilt binary for your
> OS/arch and only falls back to a source build if none exists. Building from
> source instead? See section 2 below.

---

## How the pieces fit together

Three parts:

1. **`codechat` (wrapper script)** — starts a tmux session that looks like a
   plain terminal (no status bar): Claude Code in the big left pane, the chat
   client in a fixed 32-column right pane.
2. **`codechat-tui` (chat client)** — a small Rust terminal app. It speaks
   Supabase's realtime protocol directly over a WebSocket: **Broadcast** for
   messages, **Presence** for the online counter. It never touches a database.
3. **Supabase Realtime** — the message bus everyone shares. Its URL and
   *publishable* key are compiled into the client — that key is a client-side
   identifier designed to be public, not a secret.

```
   one terminal window (tmux session, invisible chrome)
┌──────────────────────────────┬────────────────┐
│  claude (Claude Code CLI)    │  codechat-tui  │
└──────────────────────────────┴───────┬────────┘
                                       │ WebSocket:
                                       │ Broadcast + Presence
                                       ▼
                            ┌──────────────────────┐
                            │  Supabase Realtime   │
                            │ channel "global-chat"│
                            └──────────┬───────────┘
                                       │
                        every CodeChat user, worldwide
```

### Repository layout

```
CodeChat/
├── codechat                 # the wrapper script (bash + tmux)
├── install.sh               # curl-able installer (downloads release binaries)
├── config.example.json      # template for ~/.codechat/config.json (optional)
├── README.md
├── tui/                     # the chat client (Rust)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs          # terminal UI: rendering, input, config, history
│       └── realtime.rs      # Supabase Realtime protocol (Phoenix/WebSocket)
├── supabase/schema.sql      # operator-run SQL: enables shared history
├── .github/workflows/       # release automation: tag → prebuilt binaries
├── scripts/gen_icons.py     # only used by the legacy overlay below
├── src/ + src-tauri/        # LEGACY: earlier floating-window overlay (Tauri).
│                            # Not part of the supported setup — kept for reference.
```

---

## 1. Prerequisites

### 1a. tmux (required)

```bash
# Linux (Debian/Ubuntu)
sudo apt install -y tmux

# macOS
brew install tmux
```

Verify: `tmux -V` (any 3.x is fine).

### 1b. Rust + Cargo (to build; users of a prebuilt binary skip this)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Accept the defaults, restart your terminal (or `source "$HOME/.cargo/env"`),
then verify with `cargo --version`. Anything ≥ 1.77 works.

That's it. **No system libraries are needed** — the chat client uses rustls
with bundled certificates, so it builds and runs on a bare machine.

### 1c. Claude Code

`command -v claude` should print a path. If not:
<https://claude.com/claude-code>

---

## 2. Build

```bash
cd tui
cargo build --release
cd ..
```

First build takes ~1–3 minutes. The result is a single binary:

```
tui/target/release/codechat-tui
```

Quick check without the UI — a headless connectivity self-test:

```bash
./tui/target/release/codechat-tui --smoke
# smoke: joined channel 'global-chat'
# smoke: presence count = 1
# smoke: PASS — broadcast round-trip OK
```

### Distributing binaries (production)

Users you share this with need exactly three things: **tmux**, the
**`codechat` script**, and the **`codechat-tui` binary** for their platform
(plus Claude Code itself, obviously). Build the binary on each OS you target
(Rust doesn't cross-compile between macOS/Linux out of the box):

```bash
# maximum-compatibility Linux build (static, runs on any distro):
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

Since the backend is baked in, anyone who gets the two files is instantly in
the same worldwide chat — zero configuration.

---

## 3. Install

Put both pieces on your `PATH`:

```bash
chmod +x codechat
mkdir -p ~/.local/bin
cp codechat ~/.local/bin/
cp tui/target/release/codechat-tui ~/.local/bin/
```

If `~/.local/bin` isn't on your PATH, add this to `~/.bashrc` / `~/.zshrc`:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### Optional: get `claude --chat` (recommended alias)

```bash
# bash — add to ~/.bashrc        # zsh — add to ~/.zshrc
alias claude='codechat --no-chat'
```

Then `source ~/.bashrc` (or `~/.zshrc`). Because the **last** chat flag wins,
this gives you the best of both worlds:

| you type            | you get                                  |
| ------------------- | ---------------------------------------- |
| `claude …`          | plain Claude Code, byte-for-byte vanilla |
| `claude --chat …`   | Claude Code + the chat pane              |
| `codechat …`        | Claude Code + the chat pane              |

Prefer chat on *every* launch? Use `alias claude='codechat'` instead (and
`claude --no-chat` opts out per run).

This is loop-safe: the wrapper resolves the real `claude` binary on PATH and
skips itself, so even a *symlink* named `claude` pointing at `codechat` can't
loop. (If you go the symlink route, put it in a directory that comes earlier
in PATH than the real binary — and never overwrite the real `claude` file
with it.)

---

## 4. Run

```bash
codechat                # Claude Code + chat pane — args pass through:
codechat --model opus   # works exactly like claude --model opus
```

The wrapper consumes exactly three flags of its own; **everything else goes
to Claude Code untouched** (`--model`, `-p`, `--dangerously-skip-permissions`,
anything):

- `--chat` — force the chat pane on (for the `claude --chat` alias style)
- `--no-chat` — force it off: plain Claude Code, tmux not even required
- `--chat-only` — just open a chat pane in the current tmux window (no
  Claude Code); this is how you reopen the chat after closing it

If several are given, the last one wins.

**What success looks like:**

1. Your terminal becomes the split shown at the top: Claude Code left, chat
   strip right. No tmux status bar, no visible chrome.
2. First launch: the chat pane asks for a username (2–20 chars). Press Enter
   and it connects — header dot turns **green** with the online count.
3. Type in the chat pane, press **Enter** — everyone running CodeChat sees it,
   each username in its own stable color (same colors on every machine).
4. Scroll the chat with the mouse wheel or PageUp/PageDown (Esc jumps back to
   the newest message). Select text with **Shift+drag** (the plain drag is
   captured for scrolling).

**Closing and reopening the chat:**

- **Close:** click into the chat pane and press `Ctrl-C` (or type `/quit`).
  The pane vanishes and Claude Code takes the full width.
- **Reopen:** run `codechat --chat-only` from any pane in the session (e.g.
  open a second tmux pane), or just start `codechat` again next time.
- **Quit Claude Code** (as usual) → the whole session ends, chat included.
- Already a tmux user? Running `codechat` inside your own session simply adds
  the chat pane to your current window and runs Claude Code in place — your
  config and bindings are untouched.

**Width:** the chat strip defaults to 32 columns. `CODECHAT_WIDTH=40 codechat`
changes it.

### Simulating more users (testing)

The chat is completely independent of Claude — you don't need a second
Claude subscription (or even one at all) to test the room. Each extra "user"
is just another chat client:

```bash
codechat-tui --username alice     # joins as alice, in any other terminal
codechat-tui --username bob       # and bob, and…
```

`--username` never touches your config file, so your real name stays put.
Open two terminals side by side and watch the messages and the online count
move in real time. `codechat-tui --smoke` is a headless self-test that also
reports whether shared history is enabled.

### The config file (optional)

`~/.codechat/config.json` is created automatically when you pick a username:

```json
{
  "username": "your-name"
}
```

Edit it to rename yourself. Two extra keys, `supabaseUrl` and
`supabaseAnonKey`, point the client at a **different** backend (see below);
absent, the built-in public backend is used.

---

## Running your own backend (operators / forks)

Normal users never need this — the app ships pointing at the public CodeChat
backend. Read on if you **operate** that backend or are forking CodeChat.

### Creating the Supabase project

Supabase's free tier includes Realtime (200 concurrent connections, 2 million
messages/month); CodeChat never writes to the database, so DB quota stays at
zero.

1. Go to <https://supabase.com>, sign up (no credit card), click **New project**.
2. Any name, any database password (never used by CodeChat), nearest region,
   Free plan. Wait ~2 minutes.
3. Click the **gear icon (Project Settings)** in the left sidebar:
   - **Data API** page → copy the **Project URL** (`https://xxxx.supabase.co`).
   - **API Keys** page → copy the **publishable** key (`sb_publishable_...`),
     or the legacy **`anon` `public`** key (`eyJ...`) — either works.
     Never the `service_role`/secret key.
4. **Realtime is on by default** for Broadcast/Presence — nothing to enable,
   no tables to create.

### Wiring it in

- **Your machine only:** add `supabaseUrl` + `supabaseAnonKey` to
  `~/.codechat/config.json`.
- **For everyone (a fork):** edit the two `DEFAULT_SUPABASE_*` constants at
  the top of `tui/src/main.rs` and rebuild. (The legacy overlay has the same
  pair in `src/app.js`.)

### Enabling shared history (recommended)

Out of the box, messages are pure pub/sub: someone who joins sees an empty
room until the next message arrives. To give newcomers the last 50 messages
of context, run `supabase/schema.sql` once:

1. Dashboard → **SQL Editor** → **New query**
2. Paste the contents of [`supabase/schema.sql`](supabase/schema.sql) → **Run**

It creates a single `messages` table with row-level security (clients can
only INSERT and SELECT — no edits, no deletes) and a self-trimming trigger
that keeps only the newest ~1000 rows, so storage stays a few hundred KB
forever. Clients detect the table automatically — no rebuild needed; verify
with `codechat-tui --smoke` ("shared history OK").

### Production notes for operators

- **Capacity:** free tier = 200 concurrent clients / 2M messages per month;
  Pro ($25/mo) = 500 concurrent / 5M, both raisable. Watch
  **Reports → Realtime** in the dashboard.
- **Free projects pause after ~1 week of inactivity** — every client shows a
  red dot until you click **Restore project**. For an always-on worldwide
  room, Pro (no pausing) is the safer choice.
- **Key rotation:** if the room is abused, rotate the publishable key
  (Project Settings → API Keys), update the constant, ship a new build. Old
  builds stop connecting immediately.

---

## Troubleshooting

### `codechat: tmux is required and was not found`

Exactly what it says — install tmux (section 1a) and rerun. There is no
non-tmux mode.

### `codechat: chat binary 'codechat-tui' not found`

Build it (section 2) and either install it next to the script (section 3) or
point at it directly: `export CODECHAT_TUI_BIN=/path/to/codechat-tui`.

### The chat pane shows "reconnecting…" forever

- Run the self-test to see what's happening: `codechat-tui --smoke`.
- Most likely the **public backend is paused or down** — if you're the
  operator, check the Supabase dashboard (*Paused* → **Restore project**).
- If you added backend overrides to `~/.codechat/config.json`, check them:
  URL exactly `https://xxxx.supabase.co` (no trailing slash, not the
  dashboard URL), the full publishable/anon key. Deleting both lines returns
  you to the built-in backend.

### Username colors look wrong / washed out

The client uses 24-bit color. Inside tmux that requires the outer terminal to
support truecolor (nearly all modern ones do). If colors are off, add to
`~/.tmux.conf`:

```
set -as terminal-features ',*:RGB'
```

then restart tmux (`tmux kill-server`).

### The online count seems stuck

It updates on presence sync events and can lag a few seconds after network
blips. A count of `–` with a red `○` means disconnected — see above. Each
running client counts once (two panes on one machine = 2, correctly).

### Mouse selection doesn't work in the chat pane

Wheel scrolling captures the mouse; use **Shift+drag** to select text (works
in most terminals).

### Claude Code doesn't start

`command -v claude` must print a path in a fresh shell. The wrapper passes
all arguments through to the real binary and never wraps itself (symlink-safe).

---

## Privacy & security notes

- **This is one worldwide public room.** Anyone with the app (or the
  publishable key inside it) can read and write. No auth, no moderation, no
  history, no blocking. Treat it like shouting in a public square —
  **never paste secrets, keys, or private code.**
- The publishable key is designed to be shipped in clients; Broadcast +
  Presence on a channel is exactly its intended use.
- Storage is minimal and rolling: if the operator enabled shared history,
  only the newest ~1000 messages exist in their Supabase table (older rows
  are deleted automatically); otherwise nothing is persisted at all.
  Your pane shows at most the last 100 messages either way.
- Incoming payloads are length-clamped and rendered as plain text — no markup
  or escape-sequence injection.

## License

Do whatever you want with it. Have fun. 💜
