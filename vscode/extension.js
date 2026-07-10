// CodeChat VS Code extension — hosts the chat webview in the sidebar.
//
// The extension host does exactly two jobs: serve the webview HTML, and
// read/write ~/.codechat/config.json (the same file the terminal client
// uses, so you are the same person in both). All chat logic lives in the
// webview (media/chat.js).

const vscode = require("vscode");
const fs = require("fs");
const os = require("os");
const path = require("path");

const CONFIG_PATH = path.join(os.homedir(), ".codechat", "config.json");

function loadConfig() {
  try {
    return JSON.parse(fs.readFileSync(CONFIG_PATH, "utf8"));
  } catch {
    return {};
  }
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

    view.webview.onDidReceiveMessage((msg) => {
      if (msg.type === "ready") {
        view.webview.postMessage({ type: "config", config: loadConfig() });
      } else if (msg.type === "saveUsername") {
        const config = loadConfig();
        config.username = msg.username;
        saveConfig(config);
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
      <input id="input" type="text" maxlength="300" placeholder="Send a message" autocomplete="off" spellcheck="false" disabled />
    </form>
  </div>
  <script src="${uri("supabase.js")}"></script>
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
