//! CodeChat TUI — the chat pane that lives beside Claude Code in tmux.
//!
//! Plain terminal app: header (online count), scrolling message list, input
//! line. Designed for a narrow pane (~32 columns) but works at any size.
//! `codechat-tui --smoke` runs a headless connectivity self-test instead of
//! the UI (connect → join → broadcast → expect echo), handy for debugging.

mod realtime;

use chrono::{Local, TimeZone, Utc};
use ratatui::crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use realtime::{RtCommand, RtEvent};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::mpsc;
use unicode_width::UnicodeWidthStr;

// The public CodeChat backend, baked in so every install lands in the same
// worldwide room. The publishable key is a client-side key designed to be
// shipped in apps — not a secret. Keep in sync with vscode/media/chat.js.
// ~/.codechat/config.json may override both for self-hosted backends.
const DEFAULT_SUPABASE_URL: &str = "https://hhyrwfzqoszcwfklawjm.supabase.co";
const DEFAULT_SUPABASE_ANON_KEY: &str = "sb_publishable_YqXoTDD7nbWCtNphVpwBEw_a-Wj1XqA";

const MAX_MESSAGES: usize = 100; // scroll-back cap, same as the overlay
const MAX_TEXT_LEN: usize = 300;
const MAX_NAME_LEN: usize = 20;
const INVITE_URL: &str = "https://codechat.live";

// Anti-spam send throttle (token bucket): a short burst is fine, sustained
// spam is not — refills one token every 2s, capped at SEND_BURST.
const SEND_BURST: f64 = 5.0;
const SEND_REFILL_PER_SEC: f64 = 0.5;

const PURPLE: Color = Color::Rgb(145, 70, 255); // Twitch purple
const MUTED: Color = Color::Rgb(173, 173, 184);
const GREEN: Color = Color::Rgb(0, 245, 147);
const RED: Color = Color::Rgb(233, 25, 22);

// ---------------------------------------------------------------------------
// Config (~/.codechat/config.json — same file as the rest of CodeChat)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    // Shared by the terminal and VS Code clients so messages created by one
    // surface can be managed from the other installation surface.
    #[serde(skip_serializing_if = "Option::is_none")]
    client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supabase_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supabase_anon_key: Option<String>,
}

impl Config {
    fn ensure_identity(&mut self) -> bool {
        let mut changed = false;
        if self.client_id.is_none() {
            self.client_id = Some(uuid::Uuid::new_v4().to_string());
            changed = true;
        }
        if self.owner_token.is_none() {
            self.owner_token = Some(format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ));
            changed = true;
        }
        changed
    }
}

fn config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".codechat").join("config.json"))
}

fn load_config() -> Config {
    config_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_config(config: &Config) -> Result<(), String> {
    let path = config_path().ok_or("could not determine home directory")?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn backend(config: &Config) -> (String, String) {
    (
        config
            .supabase_url
            .clone()
            .unwrap_or_else(|| DEFAULT_SUPABASE_URL.to_string()),
        config
            .supabase_anon_key
            .clone()
            .unwrap_or_else(|| DEFAULT_SUPABASE_ANON_KEY.to_string()),
    )
}

// ---------------------------------------------------------------------------
// Shared history (optional). Live delivery stays pure Broadcast; a small
// `messages` table (created by supabase/schema.sql) additionally keeps the
// most recent messages so someone who just joined sees the conversation
// instead of an empty room. If the operator never created the table, both
// functions fail quietly and everything else keeps working.
// ---------------------------------------------------------------------------

const HISTORY_LIMIT: usize = 50;

#[derive(Deserialize)]
struct HistoryRow {
    id: i64,
    client_id: Option<String>,
    username: String,
    text: String,
    created_at: String,
    edited_at: Option<String>,
}

async fn fetch_history(url: &str, key: &str) -> Result<Vec<HistoryRow>, String> {
    let endpoint = format!(
        "{}/rest/v1/messages?select=id,client_id,username,text,created_at,edited_at&order=id.desc&limit={HISTORY_LIMIT}",
        url.trim_end_matches('/')
    );
    let resp = reqwest::Client::new()
        .get(&endpoint)
        .header("apikey", key)
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let mut rows: Vec<HistoryRow> = resp.json().await.map_err(|e| e.to_string())?;
    rows.reverse(); // the query returns newest-first; render oldest-first
    Ok(rows)
}

#[derive(Deserialize)]
struct CreatedRow {
    message_id: i64,
    message_created_at: String,
}

#[derive(Deserialize)]
struct LegacyCreatedRow {
    id: i64,
    created_at: String,
}

struct CreatedMessage {
    id: Option<String>,
    timestamp_ms: i64,
    editable: bool,
}

async fn create_message(
    url: &str,
    key: &str,
    username: &str,
    text: &str,
    client_id: &str,
    owner_token: &str,
) -> Result<CreatedMessage, String> {
    let http = reqwest::Client::new();
    let rpc = format!("{}/rest/v1/rpc/create_message", url.trim_end_matches('/'));
    let response = http
        .post(&rpc)
        .header("apikey", key)
        .header("Authorization", format!("Bearer {key}"))
        .json(&json!({
            "p_username": username,
            "p_text": text,
            "p_client_id": client_id,
            "p_owner_token": owner_token,
        }))
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if response.status().is_success() {
        let rows: Vec<CreatedRow> = response.json().await.map_err(|error| error.to_string())?;
        if let Some(row) = rows.first() {
            return Ok(CreatedMessage {
                id: Some(row.message_id.to_string()),
                timestamp_ms: parse_timestamp(&row.message_created_at),
                editable: true,
            });
        }
    }

    // Older self-hosted schemas still receive live and stored messages. The
    // owner controls become available after schema.sql is re-run.
    let endpoint = format!("{}/rest/v1/messages", url.trim_end_matches('/'));
    let response = http
        .post(&endpoint)
        .header("apikey", key)
        .header("Authorization", format!("Bearer {key}"))
        .header("Prefer", "return=representation")
        .json(&json!({ "username": username, "text": text }))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let rows: Vec<LegacyCreatedRow> = response.json().await.map_err(|error| error.to_string())?;
    let row = rows.first().ok_or("message was not returned")?;
    Ok(CreatedMessage {
        id: Some(row.id.to_string()),
        timestamp_ms: parse_timestamp(&row.created_at),
        editable: false,
    })
}

async fn update_owned_message(
    url: &str,
    key: &str,
    function: &str,
    message_id: &str,
    owner_token: &str,
    text: Option<&str>,
) -> Result<bool, String> {
    let endpoint = format!("{}/rest/v1/rpc/{function}", url.trim_end_matches('/'));
    let mut payload = json!({
        "p_message_id": message_id.parse::<i64>().map_err(|_| "invalid message id")?,
        "p_owner_token": owner_token,
    });
    if let Some(text) = text {
        payload["p_text"] = json!(text);
    }
    let response = reqwest::Client::new()
        .post(endpoint)
        .header("apikey", key)
        .header("Authorization", format!("Bearer {key}"))
        .json(&payload)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    response
        .json::<bool>()
        .await
        .map_err(|error| error.to_string())
}

fn parse_timestamp(rfc3339: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|date| date.timestamp_millis())
        .unwrap_or_else(|_| Utc::now().timestamp_millis())
}

fn fmt_created_at(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|dt| dt.with_timezone(&Local).format("%H:%M").to_string())
        .unwrap_or_else(|_| "--:--".into())
}

// ---------------------------------------------------------------------------
// Username colors — identical hash to the JS overlay, so "alice" renders the
// same hue in the terminal pane as in the floating window.
// ---------------------------------------------------------------------------

fn username_color(name: &str) -> Color {
    let mut hash: u32 = 0;
    for ch in name.chars() {
        hash = hash.wrapping_mul(31).wrapping_add(ch as u32);
    }
    let (r, g, b) = hsl_to_rgb(f64::from(hash % 360), 0.70, 0.65);
    Color::Rgb(r, g, b)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

enum Item {
    Chat {
        id: Option<String>,
        time: String,
        name: String,
        text: String,
        own: bool,
        mentioned: bool,
        edited: bool,
    },
    System(String),
}

enum Mode {
    NamePrompt,
    Chat,
}

struct App {
    mode: Mode,
    my_name: String,
    input: String,
    error: String,
    items: VecDeque<Item>,
    online: usize,
    online_users: Vec<String>,
    connected: bool,
    /// 0 = pinned to the newest message; >0 = user scrolled up N lines.
    /// New messages must not yank the view down while reading history.
    scroll_up: usize,
    max_scroll: usize, // recomputed every draw, clamps scroll_up
}

impl App {
    fn push(&mut self, item: Item) {
        self.items.push_back(item);
        while self.items.len() > MAX_MESSAGES {
            self.items.pop_front();
        }
    }

    fn system(&mut self, text: &str) {
        self.push(Item::System(text.to_string()));
    }

    fn latest_owned_message_id(&self) -> Option<String> {
        self.items.iter().rev().find_map(|item| match item {
            Item::Chat {
                id: Some(id),
                own: true,
                ..
            } => Some(id.clone()),
            _ => None,
        })
    }
}

fn clamp_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn fmt_time(ms: Option<i64>) -> String {
    let dt = ms
        .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
        .map(|utc| utc.with_timezone(&Local))
        .unwrap_or_else(Local::now);
    dt.format("%H:%M").to_string()
}

enum ChatInput {
    Quit,
    Help,
    Invite,
    Users,
    Send(String),
    Edit(String),
    Delete { confirmed: bool },
    Notice(String),
}

fn expand_emoji_shortcodes(text: &str) -> String {
    const ALIASES: [(&str, &str); 8] = [
        (":smile:", "😀"),
        (":joy:", "😂"),
        (":heart:", "❤️"),
        (":fire:", "🔥"),
        (":rocket:", "🚀"),
        (":thumbsup:", "👍"),
        (":check:", "✅"),
        (":eyes:", "👀"),
    ];
    ALIASES
        .iter()
        .fold(text.to_string(), |current, (alias, emoji)| {
            current.replace(alias, emoji)
        })
}

fn parse_chat_input(input: &str) -> ChatInput {
    let trimmed = input.trim();
    match trimmed {
        "/quit" => ChatInput::Quit,
        "/help" => ChatInput::Help,
        "/invite" => ChatInput::Invite,
        "/users" => ChatInput::Users,
        "/delete" => ChatInput::Delete { confirmed: false },
        "/delete confirm" => ChatInput::Delete { confirmed: true },
        _ if trimmed.starts_with("/edit ") => {
            let text = expand_emoji_shortcodes(trimmed[6..].trim());
            if text.is_empty() {
                ChatInput::Notice("usage: /edit <new text>".into())
            } else {
                ChatInput::Edit(clamp_chars(&text, MAX_TEXT_LEN))
            }
        }
        _ if trimmed.starts_with("/mention ") => {
            let rest = trimmed[9..].trim();
            match rest.split_once(" -- ") {
                Some((username, message))
                    if !username.trim().is_empty() && !message.trim().is_empty() =>
                {
                    let text = format!("@{} {}", username.trim(), message.trim());
                    ChatInput::Send(clamp_chars(&expand_emoji_shortcodes(&text), MAX_TEXT_LEN))
                }
                _ => ChatInput::Notice("usage: /mention <username> -- <message>".into()),
            }
        }
        _ if trimmed.starts_with('/') => {
            ChatInput::Notice("unknown command — type /help to see terminal shortcuts".into())
        }
        _ => ChatInput::Send(clamp_chars(&expand_emoji_shortcodes(trimmed), MAX_TEXT_LEN)),
    }
}

enum BackgroundResult {
    Send {
        text: String,
        result: Result<CreatedMessage, String>,
    },
    Edit {
        id: String,
        text: String,
        result: Result<bool, String>,
    },
    Delete {
        id: String,
        result: Result<bool, String>,
    },
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Wrap one item into styled lines at the given width. Wrapping is done here
/// (not by the widget) so the scroll math in draw() is exact.
fn wrap_item(item: &Item, width: usize) -> Vec<Line<'static>> {
    let width = width.max(8);
    let opts = textwrap::Options::new(width).subsequent_indent("  ");

    match item {
        Item::System(text) => textwrap::wrap(&format!("· {text}"), &opts)
            .into_iter()
            .map(|l| {
                Line::from(Span::styled(
                    l.into_owned(),
                    Style::default().fg(MUTED).add_modifier(Modifier::ITALIC),
                ))
            })
            .collect(),

        Item::Chat {
            id: _,
            time,
            name,
            text,
            own,
            mentioned,
            edited,
        } => {
            let suffix = if *edited { " (edited)" } else { "" };
            let full = format!("{time} {name}: {text}{suffix}");
            let head = format!("{time} {name}:");
            let mut lines = Vec::new();
            for (i, wrapped) in textwrap::wrap(&full, &opts).into_iter().enumerate() {
                let line = wrapped.into_owned();
                // First line normally starts with "HH:MM name:" — style those
                // parts. At absurdly narrow widths textwrap may break inside
                // the head; then we just render the line unstyled.
                if i == 0 && line.starts_with(&head) {
                    let mut name_style = Style::default()
                        .fg(username_color(name))
                        .add_modifier(Modifier::BOLD);
                    if *own {
                        // Subtle marker for our own messages.
                        name_style = name_style.add_modifier(Modifier::UNDERLINED);
                    }
                    lines.push(Line::from(vec![
                        Span::styled(format!("{time} "), Style::default().fg(MUTED)),
                        Span::styled(name.clone(), name_style),
                        Span::styled(
                            line[head.len() - 1..].to_string(),
                            if *mentioned {
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                            },
                        ), // ":" onward
                    ]));
                } else {
                    lines.push(Line::from(Span::raw(line)));
                }
            }
            lines
        }
    }
}

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    let [header_area, msg_area, input_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let w = frame.area().width as usize;

    // -- header: "▌CodeChat" left, "● 12" right ------------------------------
    let left = "▌CodeChat";
    let right = if app.connected {
        format!("● {}", app.online)
    } else {
        "○ –".to_string()
    };
    let pad = w.saturating_sub(left.width() + right.width());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                left.to_string(),
                Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(pad)),
            Span::styled(
                right,
                Style::default().fg(if app.connected { GREEN } else { RED }),
            ),
        ])),
        header_area,
    );

    // -- message list ---------------------------------------------------------
    let mut lines: Vec<Line> = Vec::new();
    if matches!(app.mode, Mode::NamePrompt) {
        lines.extend(wrap_item(
            &Item::System("Welcome to CodeChat — one worldwide room.".into()),
            w,
        ));
        lines.extend(wrap_item(
            &Item::System("Pick a username (2–20 chars) below and press Enter.".into()),
            w,
        ));
        if !app.error.is_empty() {
            lines.push(Line::from(Span::styled(
                app.error.clone(),
                Style::default().fg(RED),
            )));
        }
    } else {
        for item in &app.items {
            lines.extend(wrap_item(item, w));
        }
    }

    let height = msg_area.height as usize;
    app.max_scroll = lines.len().saturating_sub(height);
    app.scroll_up = app.scroll_up.min(app.max_scroll);
    let top = app.max_scroll - app.scroll_up;
    let visible: Vec<Line> = lines.into_iter().skip(top).take(height).collect();
    frame.render_widget(Paragraph::new(visible), msg_area);

    // -- input line -----------------------------------------------------------
    let prompt = match app.mode {
        Mode::NamePrompt => "name> ",
        Mode::Chat => "> ",
    };
    let dim_input = matches!(app.mode, Mode::Chat) && !app.connected;
    let shown = if app.input.is_empty() && dim_input {
        "reconnecting…".to_string()
    } else {
        // Keep the tail visible if the input outgrows the pane width.
        let budget = w.saturating_sub(prompt.len() + 1);
        let chars: Vec<char> = app.input.chars().collect();
        let start = chars.len().saturating_sub(budget);
        chars[start..].iter().collect()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(prompt, Style::default().fg(PURPLE)),
            Span::styled(
                shown.clone(),
                if dim_input {
                    Style::default().fg(MUTED)
                } else {
                    Style::default()
                },
            ),
        ])),
        input_area,
    );
    let cursor_x = (prompt.width()
        + if app.input.is_empty() && dim_input {
            0
        } else {
            shown.width()
        })
    .min(w.saturating_sub(1)) as u16;
    frame.set_cursor_position((input_area.x + cursor_x, input_area.y));
}

// ---------------------------------------------------------------------------
// Interactive UI
// ---------------------------------------------------------------------------

async fn run_ui(username_override: Option<String>) {
    let mut config = load_config();
    let identity_error = if config.ensure_identity() {
        save_config(&config).err()
    } else {
        None
    };
    let client_id = config
        .client_id
        .clone()
        .expect("message identity should be initialized");
    let owner_token = config
        .owner_token
        .clone()
        .expect("message owner token should be initialized");
    let (url, key) = backend(&config);

    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);

    // Terminal input arrives on a plain blocking thread; forwarding through a
    // channel lets the main loop select! over keyboard and network together.
    let (term_tx, mut term_rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while let Ok(ev) = ratatui::crossterm::event::read() {
            if term_tx.send(ev).is_err() {
                break;
            }
        }
    });

    let mut app = App {
        mode: Mode::NamePrompt,
        my_name: String::new(),
        input: String::new(),
        error: String::new(),
        items: VecDeque::new(),
        online: 0,
        online_users: Vec::new(),
        connected: false,
        scroll_up: 0,
        max_scroll: 0,
    };
    if let Some(error) = identity_error {
        app.system(&format!("could not save message controls: {error}"));
    }

    let mut rt_tx: Option<mpsc::UnboundedSender<RtCommand>> = None;
    let mut rt_rx: Option<mpsc::UnboundedReceiver<RtEvent>> = None;
    let (background_tx, mut background_rx) = mpsc::unbounded_channel::<BackgroundResult>();
    // Shared-history state: fetched once per launch, after the first
    // successful join (reconnects must not duplicate it).
    let mut history_rx: Option<tokio::sync::oneshot::Receiver<Result<Vec<HistoryRow>, String>>> =
        None;
    let mut history_done = false;

    // Username priority: --username flag (never persisted — handy for
    // testing several "users" on one machine) → config file → prompt.
    let preset = username_override
        .filter(|n| valid_name(n))
        .or_else(|| config.username.clone().filter(|n| valid_name(n)));
    if let Some(name) = preset {
        app.my_name = name.clone();
        app.mode = Mode::Chat;
        let (tx, rx) = realtime::spawn(url.clone(), key.clone(), name);
        rt_tx = Some(tx);
        rt_rx = Some(rx);
        app.system("connecting…");
    }

    // Send throttle (anti-spam): token bucket, refilled over time.
    let mut send_tokens = SEND_BURST;
    let mut send_refill = Instant::now();

    loop {
        if terminal.draw(|f| draw(f, &mut app)).is_err() {
            break;
        }

        tokio::select! {
            ev = term_rx.recv() => {
                let Some(ev) = ev else { break };
                match ev {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                        match k.code {
                            KeyCode::Char('c') if ctrl => break,
                            KeyCode::Char('d') if ctrl => break,
                            KeyCode::Char('u') if ctrl => app.input.clear(),
                            KeyCode::Char(c) if !ctrl => {
                                let max = match app.mode {
                                    Mode::NamePrompt => MAX_NAME_LEN,
                                    Mode::Chat => MAX_TEXT_LEN,
                                };
                                if app.input.chars().count() < max {
                                    app.input.push(c);
                                }
                            }
                            KeyCode::Backspace => { app.input.pop(); }
                            KeyCode::PageUp => app.scroll_up = (app.scroll_up + 5).min(app.max_scroll),
                            KeyCode::PageDown => app.scroll_up = app.scroll_up.saturating_sub(5),
                            KeyCode::Esc | KeyCode::End => app.scroll_up = 0,
                            KeyCode::Enter => match app.mode {
                                Mode::NamePrompt => {
                                    let name = app.input.trim().to_string();
                                    if !valid_name(&name) {
                                        app.error = "2–20 characters, try again".into();
                                    } else {
                                        config.username = Some(name.clone());
                                        if let Err(e) = save_config(&config) {
                                            app.error = format!("could not save config: {e}");
                                        } else {
                                            app.error.clear();
                                            app.input.clear();
                                            app.my_name = name.clone();
                                            app.mode = Mode::Chat;
                                            app.system(&format!("Welcome, {name}! Connecting…"));
                                            let (tx, rx) = realtime::spawn(url.clone(), key.clone(), name);
                                            rt_tx = Some(tx);
                                            rt_rx = Some(rx);
                                        }
                                    }
                                }
                                Mode::Chat => {
                                    let input = parse_chat_input(&app.input);
                                    app.input.clear();
                                    match input {
                                        ChatInput::Quit => break,
                                        ChatInput::Help => {
                                            app.system("commands: /users · /mention <name> -- <message> · /edit <text> · /delete · /invite · /quit");
                                            app.system("emoji aliases: :smile: :joy: :heart: :fire: :rocket: :thumbsup: :check: :eyes:");
                                        }
                                        ChatInput::Invite => app.system(&format!("invite: {INVITE_URL}")),
                                        ChatInput::Users => {
                                            if app.online_users.is_empty() {
                                                app.system("no named users are currently visible");
                                            } else {
                                                app.system(&format!("online: {}", app.online_users.join(", ")));
                                            }
                                        }
                                        ChatInput::Notice(message) => app.system(&message),
                                        ChatInput::Delete { confirmed: false } => {
                                            app.system("type /delete confirm to delete your latest message");
                                        }
                                        ChatInput::Delete { confirmed: true } => {
                                            if let Some(id) = app.latest_owned_message_id() {
                                                let (task_url, task_key, task_token, task_tx) = (
                                                    url.clone(), key.clone(), owner_token.clone(), background_tx.clone()
                                                );
                                                tokio::spawn(async move {
                                                    let result = update_owned_message(
                                                        &task_url, &task_key, "delete_message", &id, &task_token, None,
                                                    ).await;
                                                    let _ = task_tx.send(BackgroundResult::Delete { id, result });
                                                });
                                            } else {
                                                app.system("no editable message from this installation");
                                            }
                                        }
                                        ChatInput::Edit(text) => {
                                            if let Some(id) = app.latest_owned_message_id() {
                                                let (task_url, task_key, task_token, task_tx, task_text) = (
                                                    url.clone(), key.clone(), owner_token.clone(), background_tx.clone(), text.clone()
                                                );
                                                tokio::spawn(async move {
                                                    let result = update_owned_message(
                                                        &task_url, &task_key, "edit_message", &id, &task_token, Some(&task_text),
                                                    ).await;
                                                    let _ = task_tx.send(BackgroundResult::Edit { id, text: task_text, result });
                                                });
                                            } else {
                                                app.system("no editable message from this installation");
                                            }
                                        }
                                        ChatInput::Send(text) if !text.is_empty() && app.connected => {
                                            let now = Instant::now();
                                            send_tokens = (send_tokens
                                                + now.duration_since(send_refill).as_secs_f64() * SEND_REFILL_PER_SEC)
                                                .min(SEND_BURST);
                                            send_refill = now;
                                            if send_tokens < 1.0 {
                                                app.system("slow down — you're sending too fast");
                                            } else {
                                                send_tokens -= 1.0;
                                                let (task_url, task_key, task_name, task_client_id, task_token, task_tx, task_text) = (
                                                    url.clone(), key.clone(), app.my_name.clone(), client_id.clone(),
                                                    owner_token.clone(), background_tx.clone(), text.clone(),
                                                );
                                                tokio::spawn(async move {
                                                    let result = create_message(
                                                        &task_url, &task_key, &task_name, &task_text, &task_client_id, &task_token,
                                                    ).await;
                                                    let _ = task_tx.send(BackgroundResult::Send { text: task_text, result });
                                                });
                                            }
                                        }
                                        ChatInput::Send(text) if !text.is_empty() => {
                                            app.system("not connected — message was not sent");
                                        }
                                        ChatInput::Send(_) => {}
                                    }
                                }
                            },
                            _ => {}
                        }
                    }
                    Event::Mouse(m) => match m.kind {
                        MouseEventKind::ScrollUp => {
                            app.scroll_up = (app.scroll_up + 3).min(app.max_scroll);
                        }
                        MouseEventKind::ScrollDown => {
                            app.scroll_up = app.scroll_up.saturating_sub(3);
                        }
                        _ => {}
                    },
                    _ => {} // Resize redraws on the next loop pass
                }
            }

            result = background_rx.recv() => {
                let Some(result) = result else { continue };
                match result {
                    BackgroundResult::Send { text, result } => {
                        let (id, timestamp, sender_client_id) = match result {
                            Ok(created) => (
                                created.id,
                                created.timestamp_ms,
                                created.editable.then(|| client_id.clone()),
                            ),
                            Err(_) => {
                                app.system("message could not be saved — sent live only");
                                (None, Utc::now().timestamp_millis(), None)
                            }
                        };
                        if let Some(tx) = &rt_tx {
                            let _ = tx.send(RtCommand::broadcast("message", json!({
                                "id": id,
                                "username": app.my_name,
                                "text": text,
                                "clientId": sender_client_id,
                                "timestamp": timestamp,
                            })));
                        }
                    }
                    BackgroundResult::Edit { id, text, result: Ok(true) } => {
                        if let Some(tx) = &rt_tx {
                            let _ = tx.send(RtCommand::broadcast("message_edit", json!({
                                "id": id,
                                "text": text,
                                "editedAt": Utc::now().timestamp_millis(),
                            })));
                        }
                    }
                    BackgroundResult::Delete { id, result: Ok(true) } => {
                        if let Some(tx) = &rt_tx {
                            let _ = tx.send(RtCommand::broadcast("message_delete", json!({ "id": id })));
                        }
                    }
                    BackgroundResult::Edit { result: Ok(false), .. }
                    | BackgroundResult::Delete { result: Ok(false), .. } => {
                        app.system("message control was rejected; only your own messages can be changed");
                    }
                    BackgroundResult::Edit { result: Err(_), .. }
                    | BackgroundResult::Delete { result: Err(_), .. } => {
                        app.system("message control failed; check the server schema and connection");
                    }
                }
            }

            ev = async {
                match rt_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match ev {
                    Some(RtEvent::Connected) => {
                        if !app.connected {
                            app.connected = true;
                            app.system("connected — you're live worldwide ✓");
                        }
                        // First successful join → load recent shared history
                        // in the background (never re-runs on reconnects).
                        if !history_done && history_rx.is_none() {
                            let (htx, hrx) = tokio::sync::oneshot::channel();
                            let (hu, hk) = (url.clone(), key.clone());
                            tokio::spawn(async move {
                                let _ = htx.send(fetch_history(&hu, &hk).await);
                            });
                            history_rx = Some(hrx);
                        }
                    }
                    Some(RtEvent::Disconnected) => {
                        if app.connected {
                            app.connected = false;
                            app.system("connection lost — reconnecting…");
                        }
                        app.online = 0;
                        app.online_users.clear();
                    }
                    Some(RtEvent::Presence { count, usernames }) => {
                        app.online = count;
                        app.online_users = usernames;
                    }
                    Some(RtEvent::Chat { id, client_id: sender_client_id, username, text, timestamp_ms }) => {
                        // The key is public, so any client could broadcast
                        // junk — clamp instead of trusting the sender.
                        let name = clamp_chars(&username, MAX_NAME_LEN);
                        let own = sender_client_id.as_deref() == Some(client_id.as_str());
                        let mentioned = mentions(&text, &app.my_name);
                        app.push(Item::Chat {
                            id,
                            time: fmt_time(timestamp_ms),
                            name,
                            text: clamp_chars(&text, MAX_TEXT_LEN),
                            own,
                            mentioned,
                            edited: false,
                        });
                    }
                    Some(RtEvent::MessageEdit { id, text }) => {
                        if let Some(Item::Chat { text: current, edited, mentioned, .. }) = app
                            .items
                            .iter_mut()
                            .find(|item| matches!(item, Item::Chat { id: Some(item_id), .. } if item_id == &id))
                        {
                            *current = clamp_chars(&text, MAX_TEXT_LEN);
                            *edited = true;
                            *mentioned = mentions(current, &app.my_name);
                        }
                    }
                    Some(RtEvent::MessageDelete { id }) => {
                        app.items.retain(|item| {
                            !matches!(item, Item::Chat { id: Some(item_id), .. } if item_id == &id)
                        });
                    }
                    None => {
                        app.connected = false;
                        app.system("chat engine stopped");
                        rt_rx = None;
                    }
                }
            }

            res = async {
                match history_rx.as_mut() {
                    Some(rx) => rx.await.unwrap_or_else(|_| Err("history task dropped".into())),
                    None => std::future::pending().await,
                }
            } => {
                history_rx = None;
                history_done = true;
                match res {
                    Ok(rows) if rows.is_empty() => {}
                    Ok(rows) => {
                        // History goes ABOVE anything that already streamed in
                        // live, separated by a marker, so scrolling up reads as
                        // one continuous conversation.
                        let mut seeded: VecDeque<Item> = rows
                            .iter()
                            .map(|r| Item::Chat {
                                id: Some(r.id.to_string()),
                                time: fmt_created_at(&r.created_at),
                                name: clamp_chars(&r.username, MAX_NAME_LEN),
                                text: clamp_chars(&r.text, MAX_TEXT_LEN),
                                own: r.client_id.as_deref() == Some(client_id.as_str()),
                                mentioned: mentions(&r.text, &app.my_name),
                                edited: r.edited_at.is_some(),
                            })
                            .collect();
                        seeded.push_back(Item::System("— you're caught up —".into()));
                        seeded.extend(app.items.drain(..));
                        app.items = seeded;
                        while app.items.len() > MAX_MESSAGES {
                            app.items.pop_front();
                        }
                    }
                    // Table missing (operator didn't run supabase/schema.sql)
                    // or transient REST failure — live chat is unaffected.
                    Err(_) => app.system("no shared history — live messages only"),
                }
            }
        }
    }

    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
}

fn valid_name(name: &str) -> bool {
    let n = name.chars().count();
    (2..=MAX_NAME_LEN).contains(&n)
}

fn mentions(text: &str, username: &str) -> bool {
    text.to_lowercase()
        .contains(&format!("@{}", username.to_lowercase()))
}

// ---------------------------------------------------------------------------
// Headless smoke test: `codechat-tui --smoke`
// ---------------------------------------------------------------------------

async fn run_smoke() -> i32 {
    let config = load_config();
    let (url, key) = backend(&config);
    println!("smoke: connecting to {url}");

    match fetch_history(&url, &key).await {
        Ok(rows) => println!("smoke: shared history OK ({} stored messages)", rows.len()),
        Err(e) => println!(
            "smoke: shared history unavailable ({e}) — operator can enable it with supabase/schema.sql"
        ),
    }

    let (tx, mut rx) = realtime::spawn(url, key, "smoke-test".into());
    let token = format!("smoke-{}", Utc::now().timestamp_millis());
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(15);

    loop {
        let Ok(ev) = tokio::time::timeout_at(deadline, rx.recv()).await else {
            println!("smoke: FAIL — no echo within 15s");
            return 2;
        };
        match ev {
            Some(RtEvent::Connected) => {
                println!("smoke: joined channel '{}'", realtime::CHANNEL);
                let _ = tx.send(RtCommand::broadcast(
                    "message",
                    json!({
                        "username": "smoke-test",
                        "text": token,
                        "timestamp": Utc::now().timestamp_millis(),
                    }),
                ));
            }
            Some(RtEvent::Presence { count, .. }) => println!("smoke: presence count = {count}"),
            Some(RtEvent::Chat { username, text, .. }) => {
                println!("smoke: received <{username}> {text}");
                if text == token {
                    println!("smoke: PASS — broadcast round-trip OK");
                    return 0;
                }
            }
            Some(RtEvent::MessageEdit { .. } | RtEvent::MessageDelete { .. }) => {}
            Some(RtEvent::Disconnected) => println!("smoke: disconnected, retrying…"),
            None => {
                println!("smoke: FAIL — engine stopped");
                return 2;
            }
        }
    }
}

#[tokio::main]
async fn main() {
    // Select the TLS crypto backend once, up front (see Cargo.toml note).
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--smoke") {
        std::process::exit(run_smoke().await);
    }
    // --username NAME joins as NAME without touching the config file —
    // useful for simulating extra users while testing.
    let username_override = args
        .iter()
        .position(|a| a == "--username")
        .and_then(|i| args.get(i + 1).cloned());
    run_ui(username_override).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trip_preserves_message_identity() {
        let raw = r#"{
            "username": "alice",
            "clientId": "public-client-id",
            "ownerToken": "private-owner-token"
        }"#;

        let config: Config = serde_json::from_str(raw).expect("valid config");
        let saved = serde_json::to_value(config).expect("serializable config");

        assert_eq!(saved["clientId"], "public-client-id");
        assert_eq!(saved["ownerToken"], "private-owner-token");
    }

    #[test]
    fn identity_is_generated_once() {
        let mut config = Config::default();
        assert!(config.ensure_identity());
        let first_id = config.client_id.clone();
        let first_token = config.owner_token.clone();

        assert!(!config.ensure_identity());
        assert_eq!(config.client_id, first_id);
        assert_eq!(config.owner_token, first_token);
        assert_eq!(config.owner_token.expect("owner token").len(), 64);
    }

    #[test]
    fn terminal_commands_support_mentions_and_message_controls() {
        match parse_chat_input("/mention Ada Lovelace -- ship :rocket:") {
            ChatInput::Send(text) => assert_eq!(text, "@Ada Lovelace ship 🚀"),
            _ => panic!("expected a message"),
        }
        assert!(matches!(
            parse_chat_input("/delete"),
            ChatInput::Delete { confirmed: false }
        ));
        assert!(matches!(
            parse_chat_input("/delete confirm"),
            ChatInput::Delete { confirmed: true }
        ));
        match parse_chat_input("/edit fixed :check:") {
            ChatInput::Edit(text) => assert_eq!(text, "fixed ✅"),
            _ => panic!("expected an edit"),
        }
    }

    #[test]
    fn emoji_aliases_expand_in_regular_messages() {
        match parse_chat_input("looks good :thumbsup: :fire:") {
            ChatInput::Send(text) => assert_eq!(text, "looks good 👍 🔥"),
            _ => panic!("expected a message"),
        }
    }
}
