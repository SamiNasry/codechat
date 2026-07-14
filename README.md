# CodeChat

A small worldwide chat that lives in your terminal next to Claude Code.
Everyone who installs it lands in the same room. No accounts, no setup —
pick a name and you're in.

Coding with an AI gets quiet. This fixes that.

Website: [codechat.live](https://codechat.live)

```
┌─────────────────────────────────────┬──────────────┐
│                                     │ CodeChat  12 │
│   Claude Code                       │ alice: hey   │
│   (works exactly as usual)          │ bob: hi      │
│                                     │              │
│                                     │ > _          │
└─────────────────────────────────────┴──────────────┘
```

Linux and macOS. Windows works through WSL.

## Install

You need [Claude Code](https://claude.com/claude-code) and tmux:

```bash
sudo apt install tmux        # macOS: brew install tmux
curl -fsSL https://codechat.live/install.sh | bash
```

Open a new terminal, then:

```bash
claude --chat
```

Claude Code opens on the left, the chat on the right. First launch asks for
a username. Plain `claude` keeps working exactly as before — the installer
adds one alias (`alias claude='codechat --no-chat'`) and that is the only
thing it touches besides its own two files in `~/.local/bin`.

No compilers or libraries needed; the installer downloads one static binary.

Uninstall: delete `~/.local/bin/codechat*`, `~/.codechat`, and the alias line.

### VS Code instead of a terminal?

```bash
curl -L https://github.com/SamiNasry/codechat/releases/latest/download/codechat.vsix -o codechat.vsix && code --install-extension codechat.vsix
```

Then click the CodeChat bubble in the activity bar. Same room, same username
(it reads the same `~/.codechat/config.json`), works on Windows too, no tmux
needed. The extension lives in `vscode/`.

## Using it

- Enter sends. 300 characters max.
- Scroll with the mouse wheel or PageUp/PageDown. Esc jumps back down.
- Close the chat with Ctrl-C inside it — Claude Code takes the full width.
  Reopen with `claude --chat-only`.
- Quitting Claude Code closes everything.
- Select text with Shift+drag.
- Rename yourself in `~/.codechat/config.json`.
- Chat width: `CODECHAT_WIDTH=40 claude --chat`.
- Already using tmux? It just adds a pane to your window, nothing else.

Want to see it move without a second person? Open another terminal and run
`codechat-tui --username somebody`. `codechat-tui --smoke` is a quick
connection test.

## How it works

Three parts: a bash wrapper (`codechat`) that makes the tmux split and passes
all arguments through to the real `claude`; a single-binary Rust chat client
(`codechat-tui`); and a Supabase Realtime channel every client connects to.
Messages are broadcast live. A small table keeps the newest ~1000 so people
who join get the last 50 as context; older rows delete themselves.

The Supabase URL and publishable key are baked into the binary. A publishable
key is meant to be shipped in clients — it's an address, not a secret.

## Building from source

```bash
git clone https://github.com/SamiNasry/codechat
cd codechat/tui
cargo build --release
cp target/release/codechat-tui ../codechat ~/.local/bin/
```

Needs Rust (rustup.rs), nothing else. Release binaries are built by
`.github/workflows/release.yml` whenever a `v*` tag is pushed.

## Running your own room

Fork it and point it at your own free Supabase project:

1. supabase.com → New project (any name, password, region).
2. Project Settings → Data API: copy the project URL.
   Project Settings → API Keys: copy the publishable key
   (never the service_role key).
3. Put both in the `DEFAULT_SUPABASE_*` constants in `tui/src/main.rs`,
   rebuild, tag a release.
4. For join history, run `supabase/schema.sql` once in the SQL Editor.
   To wipe it later: `truncate table public.messages;` — people currently
   connected keep what's on screen, only new joiners are affected.

The free tier allows 200 concurrent users and pauses the project after a
week of inactivity (restore it from the dashboard, or use Pro). If the room
is abused, rotate the publishable key and tag a new release.

## Troubleshooting

| problem | fix |
| --- | --- |
| `unknown option '--chat'` | The alias isn't loaded yet — open a new terminal. |
| `tmux is required` | Install tmux. There's no non-tmux mode. |
| stuck on "reconnecting…" | Run `codechat-tui --smoke`. Usually the backend is paused, or `~/.codechat/config.json` has broken override values. |
| washed-out colors | Add `set -as terminal-features ',*:RGB'` to `~/.tmux.conf`, then `tmux kill-server`. |
| old messages still visible after a DB wipe | That's your pane's local memory. Close and reopen it. |

## Privacy

It's one public room. Anyone can read and write. Don't paste secrets or
private code. At most ~1000 recent messages exist server-side, at most 100
on your screen. Messages render as plain text — no markup, no injection.

## License

Do whatever you want with it.
