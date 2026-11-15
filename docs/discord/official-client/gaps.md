# Where Discord Obsidian differs from the official client

Discord Obsidian presents itself as the web client in Firefox. The web client
runs the same `GatewaySocket` and `RTCControlSocket` code as desktop, minus
the native modules, so the web paths below are the ones to match. Snapshot:
web build 673764.

## Wire differences a server can see

| Area | Official client (web path) | Discord Obsidian | Source |
|---|---|---|---|
| Gateway heartbeat | op **40** `QOS_HEARTBEAT` `{seq, qos: {active, ver: 31, reasons}}` on every beat | op 1 with the bare sequence | `gateway.rs` `HEARTBEAT` |
| Heartbeat when activity changes | an extra beat when the client goes from inactive to active | none | |
| Identify `qos_token` | sent (base64 `QosToken` protobuf), and `derived` is kept per user once the server has signed it | absent | `gateway.rs` identify |
| First gateway heartbeat | after random `0..interval` | after random `0..interval` | same |
| Voice gateway version | `?v=9`, or `v=10` with the SFU-update experiment | `?v=8` | `voice_connection.rs` `VERSION` |
| Voice heartbeat interval | **capped at 5 s on the web** (desktop uses the server's value) | the server's value, kept between 1 s and 300 s | `voice_connection.rs` `heartbeat()` |
| Voice op 16 after READY | sends `{}` when server v ≥ 6 | not sent | `voice_connection.rs` |
| Gateway encoding | `json` + `zlib-stream` on the web | `json` + `zlib-stream` | same |
| Capabilities | 1734653 (1767421 with channel obfuscation) | 1734653 | same |

The heartbeat opcode is the biggest tell. Every client running the current
bundle sends op 40, so op 1 marks a client that isn't one. Matching it
takes `{seq, qos}` with `qos: null` until the first activity report, then
`{active, ver: 31, reasons}`, where `reasons` lists `foregrounded` while the
window is in front and `rtc_connected` while a call is connected.

## Gateway events Discord Obsidian doesn't handle

`data/gateway-events-unhandled.txt` lists 131 dispatches that the official
client handles and `obsidian-discord` never names. Most concern billing,
quests, Activities, notification center, OAuth, game servers and developer
features. The ones a chat-and-voice client shows to users:

- `CHANNEL_PINS_UPDATE`, `CHANNEL_PINS_ACK`: the pins badge and its unread state
- `GUILD_MEMBER_ADD`: someone joins while the member list is open
- `VOICE_CHANNEL_STATUS_UPDATE`, `VOICE_CHANNEL_START_TIME_UPDATE`: the
  status line under a voice channel and "in voice for …"
- `VOICE_STATE_UPDATE_BATCH`: several voice state changes in one dispatch
- `STAGE_INSTANCE_CREATE/UPDATE/DELETE`: Stage topics
- `SESSIONS_REPLACE`: your own other sessions, used for your shown status
  and "active on mobile"
- `PRESENCES_REPLACE`: a whole presence refresh
- `INTERACTION_CREATE/SUCCESS/FAILURE`, `INTERACTION_MODAL_CREATE`: slash
  command and button feedback, and app modals
- `APPLICATION_COMMAND_AUTOCOMPLETE_RESPONSE`: slash command option autocomplete
- `THREAD_MEMBER_LIST_UPDATE`: a thread's member sidebar
- `SAVED_MESSAGE_CREATE/DELETE`: "Save for later" / bookmarks
- `RECENT_MENTION_DELETE`: removing an item from the inbox's mentions
- `GUILD_SCHEDULED_EVENT_USER_ADD/REMOVE`: the "interested" count on events
- `CHANNEL_MEMBER_COUNT_UPDATE`, `CHANNEL_INFO`: answers to ops 39 and 43
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
- 39 `REQUEST_CHANNEL_MEMBER_COUNT`, 43 `REQUEST_CHANNEL_INFO`
- 35 `SEARCH_RECENT_MEMBERS`: the paged member search in moderation views
- 41 `UPDATE_TIME_SPENT_SESSION_ID`: analytics, and optional
