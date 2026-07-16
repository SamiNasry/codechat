(function exposeCodeChatUtils(root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.CodeChatUtils = api;
})(typeof globalThis === "object" ? globalThis : this, function createCodeChatUtils() {
  "use strict";

  function truncate(text, maxLength) {
    return [...String(text)].slice(0, maxLength).join("");
  }

  function validUsername(name) {
    const length = typeof name === "string" ? [...name].length : 0;
    return length >= 2 && length <= 20;
  }

  function expandEmojiShortcodes(text) {
    const aliases = {
      ":smile:": "😀",
      ":joy:": "😂",
      ":heart:": "❤️",
      ":fire:": "🔥",
      ":rocket:": "🚀",
      ":thumbsup:": "👍",
      ":check:": "✅",
      ":eyes:": "👀",
    };
    return String(text).replace(
      /:(?:smile|joy|heart|fire|rocket|thumbsup|check|eyes):/g,
      (shortcode) => aliases[shortcode],
    );
  }

  function normalizeMessage(raw, maxTextLength = 300) {
    if (!raw || typeof raw.username !== "string" || typeof raw.text !== "string") return null;
    const id = raw.id == null ? null : String(raw.id);
    const parsedDate = Date.parse(raw.created_at || raw.createdAt || "");
    const timestamp = typeof raw.timestamp === "number"
      ? raw.timestamp
      : Number.isNaN(parsedDate) ? Date.now() : parsedDate;
    return {
      id,
      username: truncate(raw.username, 20) || "anon",
      text: truncate(raw.text, maxTextLength),
      clientId: raw.clientId || raw.client_id || null,
      timestamp,
      edited: Boolean(raw.edited || raw.edited_at || raw.editedAt),
    };
  }

  function presenceSnapshot(state) {
    const users = new Set();
    let count = 0;
    for (const presences of Object.values(state || {})) {
      if (!Array.isArray(presences)) continue;
      count += presences.length;
      for (const presence of presences) {
        if (typeof presence.username === "string") users.add(presence.username);
      }
    }
    return { count, users: [...users] };
  }

  return {
    expandEmojiShortcodes,
    normalizeMessage,
    presenceSnapshot,
    truncate,
    validUsername,
  };
});
