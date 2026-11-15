# Where Discord Obsidian differs from the official client

Discord Obsidian presents itself as the web client in Firefox. The web client
runs the same `GatewaySocket` and `RTCControlSocket` code as desktop, minus
the native modules, so the web paths below are the ones to match. Snapshot:
web build 673764.

## Wire differences a server can see

Matched now:

- Heartbeats go as op 40 `QOS_HEARTBEAT` `{seq, qos}`, `qos` reporting
  `foregrounded` and `rtc_connected` since the last beat, with an extra
  beat when the client goes from unused to used (`qos.rs`, `gateway.rs`).
- IDENTIFY sends `qos_token`, and keeps READY's signed `qos_token` per
  account for the next identify (`cache.rs` `discord-qos-token`).
- The voice gateway is `?v=9`. It sends op 16 after READY from a version 6
  server, caps the heartbeat at 5 s as the web client does, and sends
  SELECT_PROTOCOL's address, port and mode at the top level with an
  `rtc_connection_id` (`voice_connection.rs`).
- `VOICE_STATE_UPDATE_BATCH` and `PRESENCES_REPLACE` are handled, split
  into the single events.
- Voice channel statuses: op 43 `REQUEST_CHANNEL_INFO` the first time a
  server opens, `CHANNEL_INFO` and `VOICE_CHANNEL_STATUS_UPDATE`.
- `GUILD_SCHEDULED_EVENT_USER_ADD/REMOVE` keep interested counts current.
- `CHANNEL_PINS_UPDATE` reloads pins while they're open.
- Stage topics show under stage channels (`STAGE_INSTANCE_*`), and
  `GUILD_MEMBER_ADD` teaches the new member's name. The member count stays
  as READY said, since counting would copy the server on every join.
- Apps' buttons press (`POST /interactions`, type 3), and
  `INTERACTION_SUCCESS`/`FAILURE` answer them.

Still different:

| Area | Official client (web path) | Discord Obsidian |
|---|---|---|
| Voice transport | the web client speaks WebRTC (`protocol: "webrtc"`, SDP); desktop speaks UDP | UDP with RTP, as desktop does, while presenting as the web client |
| Voice v10 | with the SFU-update experiment, plus op 33 | not asked for |
| SELECT_PROTOCOL `experiments` | the voice server's experiments the client supports | `[]` |

## Gateway events Discord Obsidian doesn't handle

`data/gateway-events-unhandled.txt` lists 131 dispatches that the official
client handles and `obsidian-discord` never names. Most concern billing,
quests, Activities, notification center, OAuth, game servers and developer
features. The ones a chat-and-voice client shows to users:

- `CHANNEL_PINS_ACK`: the pins' unread state
- `SESSIONS_REPLACE`: your own other sessions, used for your shown status
  and "active on mobile"
- `INTERACTION_MODAL_CREATE`: app modals
- `APPLICATION_COMMAND_AUTOCOMPLETE_RESPONSE`: slash command option autocomplete
- `THREAD_MEMBER_LIST_UPDATE`: a thread's member sidebar
- `SAVED_MESSAGE_CREATE/DELETE`: "Save for later" / bookmarks
- `RECENT_MENTION_DELETE`: removing an item from the inbox's mentions
- `LAST_MESSAGES`: the answer to op 34
- `FORUM_UNREADS`: the answer to op 28
- `GUILD_JOIN_REQUEST_*`: membership screening / applications
- `WEBHOOKS_UPDATE`, `GUILD_INTEGRATIONS_UPDATE`: server settings pages
- `USER_CONNECTIONS_UPDATE`: connected accounts on profiles

## Gateway commands Discord Obsidian doesn't send

Compare with the command table in [gateway.md](gateway.md). Discord Obsidian
sends 1–4, 6, 8, 18–20, 22, 31 and 37 (`gateway.rs`). Of the rest:

- 13 `CALL_CONNECT` `{channel_id}`: for DM calls. The callers weren't traced
- 21 `STREAM_PING` `{stream_key}`: for Go Live streams. The callers weren't traced
- 5 `VOICE_SERVER_PING`
- 28 `REQUEST_FORUM_UNREADS`: unread state for the threads of a forum
- 34 `REQUEST_LAST_MESSAGES`: the official client fetches the last message
  of many channels at once, for sorting and previews
- 39 `REQUEST_CHANNEL_MEMBER_COUNT`
- 35 `SEARCH_RECENT_MEMBERS`: the paged member search in moderation views
- 41 `UPDATE_TIME_SPENT_SESSION_ID`: analytics, and optional
