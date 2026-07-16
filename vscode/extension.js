// CodeChat VS Code extension — hosts the chat webview in the sidebar.
//
// The extension host does exactly two jobs: serve the webview HTML, and
// read/write ~/.codechat/config.json (the same file the terminal client
// uses, so you are the same person in both). All chat logic lives in the
// webview (media/chat.js).

const vscode = require("vscode");
const crypto = require("crypto");
const fs = require("fs");
const os = require("os");
const path = require("path");

const CONFIG_PATH = path.join(os.homedir(), ".codechat", "config.json");

function loadConfig() {
  try {
    const config = JSON.parse(fs.readFileSync(CONFIG_PATH, "utf8"));
    return ensureIdentity(config);
  } catch {
    return ensureIdentity({});
  }
}

function ensureIdentity(config) {
  let changed = false;
  if (!config.clientId) {
    config.clientId = crypto.randomUUID();
    changed = true;
  }
  if (!config.ownerToken) {
    config.ownerToken = crypto.randomBytes(32).toString("hex");
    changed = true;
  }
  if (changed) saveConfig(config);
  return config;
}

function saveConfig(config) {
  try {
    fs.mkdirSync(path.dirname(CONFIG_PATH), { recursive: true });
    fs.writeFileSync(CONFIG_PATH, JSON.stringify(config, null, 2));
    fs.chmodSync(CONFIG_PATH, 0o600);
  } catch (err) {
    vscode.window.showWarningMessage(`CodeChat: could not save config: ${err}`);
  }
}

class ChatViewProvider {
  constructor(extensionUri) {
    this.extensionUri = extensionUri;
  }

  resolveWebviewView(view) {
    view.webview.options = {
      enableScripts: true,
      localResourceRoots: [vscode.Uri.joinPath(this.extensionUri, "media")],
    };
    view.webview.html = this.html(view.webview);

    const sendVisibility = () =>
      view.webview.postMessage({ type: "visibility", visible: view.visible });
    view.onDidChangeVisibility(sendVisibility);

    view.webview.onDidReceiveMessage(async (msg) => {
      if (msg.type === "ready") {
        view.webview.postMessage({ type: "config", config: loadConfig() });
        sendVisibility();
      } else if (msg.type === "saveUsername") {
        const config = loadConfig();
        config.username = msg.username;
        saveConfig(config);
      } else if (msg.type === "copyInvite") {
        await vscode.env.clipboard.writeText("https://codechat.live");
        view.webview.postMessage({ type: "inviteCopied" });
      }
    });
  }

  html(webview) {
    const uri = (file) =>
      webview.asWebviewUri(vscode.Uri.joinPath(this.extensionUri, "media", file));
    // Webviews get a strict CSP: local scripts/styles only, network access
    // restricted to Supabase (websocket + REST for the history table).
    return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta http-equiv="Content-Security-Policy" content="default-src 'none';
  script-src ${webview.cspSource};
  style-src ${webview.cspSource} 'unsafe-inline';
  connect-src https://*.supabase.co wss://*.supabase.co;
  img-src ${webview.cspSource} data:;" />
<link rel="stylesheet" href="${uri("chat.css")}" />
</head>
<body>
  <div id="app">
    <header>
      <span class="title">CodeChat</span>
      <button id="invite" class="icon-button" title="Copy invite link" aria-label="Copy invite link">↗</button>
      <span id="status-dot" class="dot" title="Disconnected"></span>
      <span id="online-count" title="Users online">–</span>
    </header>
    <div id="setup" class="hidden">
      <p class="hint">One worldwide room. Pick a name and you're in.</p>
      <input id="setup-username" type="text" placeholder="Username (2–20 chars)" maxlength="20" autocomplete="off" />
      <button id="setup-save">Join chat</button>
      <p id="setup-error" class="error"></p>
    </div>
    <main id="messages"></main>
    <form id="composer">
      <div id="editing" class="hidden"><span>Editing message</span><button id="cancel-edit" type="button">Cancel</button></div>
      <div id="composer-row">
        <button id="mention" class="icon-button" type="button" title="Mention someone" aria-label="Mention someone">@</button>
        <button id="emoji" class="icon-button" type="button" title="Add emoji" aria-label="Add emoji">😊</button>
        <input id="input" type="text" maxlength="300" placeholder="Send a message" autocomplete="off" spellcheck="false" disabled />
      </div>
      <div id="people-menu" class="picker hidden" role="menu"></div>
      <div id="emoji-menu" class="picker hidden" role="menu"></div>
    </form>
  </div>
  <script src="${uri("supabase.js")}"></script>
  <script src="${uri("chat-utils.js")}"></script>
  <script src="${uri("chat.js")}"></script>
</body>
</html>`;
  }
}

function activate(context) {
  const provider = new ChatViewProvider(context.extensionUri);
  context.subscriptions.push(
    vscode.window.registerWebviewViewProvider("codechat.chat", provider, {
      // Keep the webview (and its websocket) alive when the panel is hidden,
      // so switching away doesn't disconnect you from the room.
      webviewOptions: { retainContextWhenHidden: true },
    })
  );
}

function deactivate() {}

module.exports = { activate, deactivate };
