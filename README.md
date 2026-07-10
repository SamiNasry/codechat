# CodeChat 💬

**A worldwide chat room inside your terminal, right next to Claude Code.**

Coding with an AI is powerful but quiet. CodeChat exists so it doesn't have to
be lonely: while Claude works, there's a little crowd of fellow builders from
around the world talking in the corner of your terminal. One global room, no
accounts, no setup — pick a name and say hi.

```
┌────────────────────────────────────────────┬──────────────┐
│                                            │ ▌CodeChat 12●│
│                                            │ 14:02 alice: │
│   Claude Code — exactly as it              │   anyone got │
│   looks today, full height,                │   tauri v2   │
│   ~85% of the width                        │   working?   │
│                                            │ 14:03 bob:   │
│                                            │   yes! check │
│                                            │   the docs   │
│                                            │              │
│                                            │ > _          │
└────────────────────────────────────────────┴──────────────┘
```

Works on **Linux** and **macOS** (on Windows, use WSL).

---

## ⚡ Install in 60 seconds

You need [Claude Code](https://claude.com/claude-code) already installed.
Then, three commands:

```bash
# 1 — tmux (the one dependency; it's what splits your terminal)
sudo apt install tmux              # Linux
brew install tmux                  # macOS

# 2 — CodeChat (downloads a prebuilt binary, sets everything up)
curl -fsSL https://raw.githubusercontent.com/SamiNasry/codechat/main/install.sh | bash

# 3 — open a NEW terminal (so the alias loads), then go!
claude --chat
```

**That's it.** Your terminal splits: Claude Code on the left, the chat strip
on the right. First launch asks for a username (2–20 characters) — press
Enter and the dot turns green, the online count appears, and the last 50
messages of the worldwide conversation load so you're never staring at an
empty room. Everything you type in the chat reaches every CodeChat user on
Earth, live.

> **No Rust, no compilers, no libraries needed.** The installer fetches a
> single static binary for your OS. If it can't find one for your platform it
> falls back to building from source (needs [Rust](https://rustup.rs)).

### The whole interface is one flag

| you type          | you get                                  |
| ----------------- | ---------------------------------------- |
| `claude …`        | plain Claude Code, byte-for-byte vanilla |
| `claude --chat …` | Claude Code **+ the worldwide chat**     |

The installer makes this work by adding a single line to your shell config
(`~/.bashrc` / `~/.zshrc`):

```bash
alias claude='codechat --no-chat'
```

Every other argument goes straight to Claude Code untouched —
`claude --chat --model opus -p "hi"` works exactly as you'd expect. Don't
want the alias? Delete that line; nothing else changes.

---

## 🕹 Using it

- **Type + Enter** — send (max 300 chars). Usernames get stable colors:
  "alice" is the same color on every machine, every day.
- **Scroll** — mouse wheel or `PageUp`/`PageDown`; `Esc` jumps back to the
  newest message. New messages never yank you down while you're reading.
- **Close the chat** — click the chat pane, press `Ctrl-C` (or type `/quit`).
  Claude Code instantly expands to full width.
- **Reopen it** — `claude --chat-only` from any pane, or just launch
  `claude --chat` again next time.
- **Quit Claude Code** — the whole thing closes, chat included.
- **Select text** — `Shift+drag` (plain drag is used for scrolling).
- **Change your name** — edit `~/.codechat/config.json`.
- **Chat width** — `CODECHAT_WIDTH=40 claude --chat` (default 32 columns).
- Already a tmux user? `claude --chat` inside your session just adds the pane
  to your current window. Your config and bindings are never touched.

### Want to see it work right now, alone?

The chat doesn't need Claude — simulate friends in a second terminal:

```bash
codechat-tui --username alice     # a fake user; your config isn't touched
codechat-tui --smoke              # headless self-test: connect → send → PASS
```

Open two terminals with two names and watch the messages and online count
move in real time.

---

## 🔧 How it works

Three small parts:

```
   one terminal window (tmux session, invisible chrome)
┌──────────────────────────────┬────────────────┐
│  claude (Claude Code CLI)    │  codechat-tui  │
└──────────────────────────────┴───────┬────────┘
                                       │ WebSocket
                                       ▼
                            ┌──────────────────────┐
                            │  Supabase Realtime   │
                            │ channel "global-chat"│
                            └──────────┬───────────┘
                                       │
                        every CodeChat user, worldwide
```

1. **`codechat`** — a bash wrapper. Starts a tmux session that looks like a
   plain terminal (no status bar): Claude Code big, chat strip small. Passes
   all your arguments through to the real `claude` binary (loop-safe even if
   you alias or symlink `claude` to it).
2. **`codechat-tui`** — the chat client. A single ~5 MB Rust binary with zero
   runtime dependencies that speaks Supabase's realtime protocol directly:
   **Broadcast** for messages, **Presence** for the online counter.
3. **Supabase Realtime** — the shared message bus. Its URL and *publishable*
   key are baked into the binary (that key is a client-side identifier
   designed to be public — like a radio frequency, not a password).

Messages are delivered live as ephemeral pub/sub. A tiny `messages` table
additionally keeps a rolling window of the newest ~1000 messages (older rows
delete themselves) purely so joiners get the last 50 as context.

### Repository layout

```
CodeChat/
├── codechat                 # the wrapper script (bash + tmux)
├── install.sh               # curl-able installer (downloads release binaries)
├── tui/                     # the chat client (Rust): UI + realtime protocol
├── supabase/schema.sql      # operator-run SQL: enables shared history
├── .github/workflows/       # release automation: git tag → prebuilt binaries
├── config.example.json      # template for ~/.codechat/config.json (optional)
├── scripts/ src/ src-tauri/ # LEGACY: an earlier floating-window overlay
│                            # (Tauri) — kept for reference, not supported
└── README.md
```

---

## 🛠 Building from source (contributors / unsupported platforms)

```bash
# Rust toolchain (once): https://rustup.rs
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

git clone https://github.com/SamiNasry/codechat
cd codechat/tui
cargo build --release          # ~2 minutes, no system libraries needed

# install both pieces
mkdir -p ~/.local/bin
cp target/release/codechat-tui ../codechat ~/.local/bin/
chmod +x ~/.local/bin/codechat
```

Make sure `~/.local/bin` is on your PATH:
`export PATH="$HOME/.local/bin:$PATH"` in your shell rc.

Release binaries are built automatically: pushing a tag like `v0.1.0`
triggers `.github/workflows/release.yml`, which compiles static binaries for
Linux (x86_64 + ARM64, musl) and macOS (Apple Silicon + Intel) and attaches
them — plus `codechat`, `install.sh` and `supabase/schema.sql` — to a GitHub
release. `install.sh` downloads from whatever the latest release is.

---

## 🗄 Operator guide (only for whoever runs the backend)

Normal users never read this section — the app ships pointing at the public
CodeChat backend. This is for the person operating it, or anyone forking
CodeChat into a private room.

### Creating the Supabase project (free)

1. <https://supabase.com> → sign up (no credit card) → **New project** —
   any name, any database password (never used by CodeChat), nearest region,
   Free plan. Wait ~2 minutes.
2. Gear icon (**Project Settings**) → **Data API** → copy the **Project URL**
   (`https://xxxx.supabase.co`).
3. **Project Settings** → **API Keys** → copy the **publishable** key
   (`sb_publishable_...`) — or the legacy `anon public` key; either works.
   ⚠️ Never the `service_role`/secret key.
4. Realtime (Broadcast/Presence) is enabled by default — nothing to switch on.

Wire it in: edit the two `DEFAULT_SUPABASE_*` constants at the top of
`tui/src/main.rs` and rebuild (that's the single source of truth for the
public backend). For a personal machine only, `supabaseUrl` /
`supabaseAnonKey` in `~/.codechat/config.json` override the baked-in values.

### Enabling shared history (recommended)

Run `supabase/schema.sql` once: Dashboard → **SQL Editor** → **New query** →
paste the file → **Run**. It creates the `messages` table with row-level
security (clients can only INSERT and SELECT — no edits, no deletes) and a
self-trimming trigger that keeps only the newest ~1000 rows, so storage stays
a few hundred KB forever. Clients detect it automatically — verify with
`codechat-tui --smoke` ("shared history OK").

### Wiping the history

The table maintains its size on its own, but if you ever want a clean slate
(spam cleanup, fresh launch), run this in the SQL Editor:

```sql
truncate table public.messages;
```

Notes: only you can do this (clients have no delete rights); people currently
connected keep what's already on their screen (that's their local memory —
up to 100 messages, gone when their pane closes); only *future joiners* are
affected, and new messages start accumulating again immediately.

### Production notes

- **Capacity:** free tier = 200 concurrent clients / 2M Realtime messages
  per month; Pro ($25/mo) = 500 / 5M, both raisable. Watch **Reports →
  Realtime** in the dashboard.
- **Free projects pause after ~1 week of inactivity** — every client shows a
  red dot until you click **Restore project**. For an always-on worldwide
  room, Pro (no pausing) is the safer choice.
- **Key rotation:** if the room is abused, rotate the publishable key
  (Project Settings → API Keys), update the constants, tag a new release.
  Old builds stop connecting immediately.

---

## 🚑 Troubleshooting

| Symptom | Fix |
| --- | --- |
| `error: unknown option '--chat'` | The alias isn't loaded in this terminal — open a **new** terminal (or `source ~/.bashrc`). If you installed manually without the installer, add `alias claude='codechat --no-chat'` to your shell rc. |
| `codechat: tmux is required` | Install tmux (see top). There is no non-tmux mode. |
| `codechat: chat binary 'codechat-tui' not found` | Rerun the installer, or point at it: `export CODECHAT_TUI_BIN=/path/to/codechat-tui`. |
| Chat says "reconnecting…" forever | Run `codechat-tui --smoke`. Usually the backend is paused (operator: dashboard → **Restore project**) or your `~/.codechat/config.json` has broken override values (delete the `supabaseUrl`/`supabaseAnonKey` lines to go back to the built-in backend). |
| "no shared history — live messages only" | Not an error — the operator hasn't run `supabase/schema.sql` (or the REST call blipped). Live chat is unaffected. |
| Colors look washed out | Your terminal needs truecolor inside tmux. Add `set -as terminal-features ',*:RGB'` to `~/.tmux.conf`, then `tmux kill-server`. |
| Online count stuck at `–` | You're disconnected — see "reconnecting" above. Presence takes ~10 s to re-sync after network blips. |
| Can't select text in the chat | Use `Shift+drag` — plain drag is captured for scrolling. |
| Claude Code doesn't start | `command -v claude` must print a path in a fresh shell. |
| Old messages still visible after the operator wiped the DB | That's your pane's local memory (last 100 msgs). Close the pane and reopen — the wipe only affects what future joiners load. |

### Uninstall

```bash
rm ~/.local/bin/codechat ~/.local/bin/codechat-tui
rm -rf ~/.codechat                      # your username config
# remove the alias line from ~/.bashrc / ~/.zshrc if you added it
```

---

## 🔒 Privacy & security

- **This is one worldwide public room.** Anyone with the app can read and
  write. No auth, no moderation, no blocking. Treat it like shouting in a
  public square: **never paste secrets, keys, or private code.**
- The publishable key is designed to be shipped in clients; Broadcast +
  Presence is exactly its intended use.
- Storage is a rolling window: at most the newest ~1000 messages exist in the
  operator's table (older rows self-delete). Your pane holds at most the last
  100 in memory. There is no permanent archive.
- Incoming payloads are length-clamped and rendered as plain text — no
  markup or escape-sequence injection.

## License

Do whatever you want with it. Have fun. 💜
