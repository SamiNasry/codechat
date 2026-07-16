"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const {
  expandEmojiShortcodes,
  normalizeMessage,
  presenceSnapshot,
  truncate,
  validUsername,
} = require("../media/chat-utils.js");

test("emoji shortcodes match the terminal aliases", () => {
  assert.equal(
    expandEmojiShortcodes("ship :rocket: :thumbsup: unknown :wave:"),
    "ship 🚀 👍 unknown :wave:",
  );
});

test("emoji truncation keeps complete Unicode code points", () => {
  assert.equal(truncate("😀😂🚀", 2), "😀😂");
  assert.equal(normalizeMessage({ username: "dev", text: "😀😂" }, 1).text, "😀");
});

test("username length is measured by characters", () => {
  assert.equal(validUsername("😀a"), true);
  assert.equal(validUsername("a"), false);
  assert.equal(validUsername("a".repeat(21)), false);
});

test("presence snapshot reflects every active presence and unique names", () => {
  const snapshot = presenceSnapshot({
    one: [{ username: "alice" }],
    two: [{ username: "bob" }, { username: "alice" }],
  });
  assert.equal(snapshot.count, 3);
  assert.deepEqual(snapshot.users.sort(), ["alice", "bob"]);
});

test("message normalization preserves stable identifiers", () => {
  const message = normalizeMessage({
    id: 42,
    username: "alice",
    text: "hello",
    client_id: "client-id",
    created_at: "2026-07-16T12:00:00Z",
    edited_at: "2026-07-16T12:01:00Z",
  });
  assert.equal(message.id, "42");
  assert.equal(message.clientId, "client-id");
  assert.equal(message.edited, true);
});
