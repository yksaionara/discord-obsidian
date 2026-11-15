# Main gateway, as the official client speaks it

Source: the `GatewaySocket` module (search for `"[CONNECT] "` and
`"_connect called with already existing websocket"`) and the opcode enum that
starts `HEARTBEAT = 1` in `data/enums.txt`.

## Connecting

URL: `GLOBAL_ENV.GATEWAY_ENDPOINT` (`wss://gateway.discord.gg`), or the
`resume_gateway_url` from `READY` once one is known, with any trailing `/`
cut off. The query is built in a fixed order:

```
?encoding=<etf|json>&v=9&compress=<zstd-stream|zlib-stream>
```

- `encoding`: `etf` when the `discord_erlpack` native module loads (desktop),
  `json` otherwise.
- `compress`: `zstd-stream` when `discord_zstd` loads, `zlib-stream`
  otherwise. If the module is missing, the client asks the updater to install
  it and uses zlib this time.
- The hello timeout is 30 s. Without `HELLO` in that time the socket closes and
  the resume URL is dropped.
- Reconnect backoff runs from 1 s to 60 s. After 4 failures in a row the
  session is reset, so the next connect identifies afresh. Close code 4004
  (authentication failed) closes for good.

## Heartbeat

The interval comes from `HELLO`. The first beat waits a random
`0..interval`, and every beat after it is sent **as op 40 `QOS_HEARTBEAT`**,
not op 1:

```json
{"op": 40, "d": {"seq": 1234, "qos": {"active": true, "ver": 31, "reasons": ["foregrounded", "rtc_connected"]}}}
```

`qos` is `null` until the activity tracker has reported once. `active` is
true while the window is in front (`foregrounded`) or a call is connected
(`rtc_connected`). The client also beats right away when it switches from
inactive to active. When the server sends op 1, the client answers at once
and restarts its interval. A beat sent with no ack since the last one means a
timeout: the client closes with 4000 and reconnects with backoff. An
"expedited heartbeat" (after a network change or on resuming from sleep) beats
now and gives up after the given timeout.

## Identify (op 2)

```json
{
  "token": "...",
  "capabilities": 1734653,
  "properties": { /* super properties, see http.md */ },
  "presence": {"status": "online", "since": 0, "activities": [], "afk": false},
  "compress": false,
  "client_state": {"guild_versions": {}},
  "qos_token": "<base64 discord_protos.qos_token.v1.QosToken>"
}
```

- `capabilities` is `1734653`, which is bits 0, 2–10, 12–14, 17, 19 and 20. With the
  channel-obfuscation experiment on (`useChannelObfuscation`) it is
  `1767421`, which adds bit 15.
- `compress` is true only for legacy per-message compression. It is false
  with either streaming transport.
- `client_state` carries the cache versions (`guild_versions`,
  `highest_last_message_id`, `read_state_version`,
  `user_guild_settings_version`, `user_settings_version`,
  `private_channels_version`, `api_code_version`, `initial_guild_id`) only
  when the client has a local cache it may reuse. Otherwise it sends
  `{"guild_versions": {}}`.
- `qos_token` is a protobuf `QosToken {client_provided: {is_active}, derived}`.
  `derived` is a signed `DerivedQosData {claims, signature, key_id}` that the
  server handed out earlier and the client keeps per user. A first identify
  has only `client_provided`.

## Resume (op 6)

`{token, session_id, seq}`. The client resumes rather than identifying again
only when it has a session and its last ack came within 3 minutes. On
`INVALID_SESSION` (op 9) with `d: true` it tries to resume, otherwise it
identifies.

## Commands the client sends

| op | name | payload |
|---|---|---|
| 3 | PRESENCE_UPDATE | `{status, since, activities, afk}` |
| 4 | VOICE_STATE_UPDATE | `{guild_id, channel_id, self_mute, self_deaf, self_video, flags}`, plus `preferred_region` and `preferred_regions` when joining and the experiment allows, plus `tracks: [{type, rid, quality}]` when video stream parameters are known |
| 5 | VOICE_SERVER_PING | `null` |
| 8 | REQUEST_GUILD_MEMBERS | `{guild_id, query, limit, user_ids, presences, nonce}` |
| 13 | CALL_CONNECT | `{channel_id}` |
| 18 | STREAM_CREATE | `{type, guild_id, channel_id, preferred_region}` |
| 19 | STREAM_DELETE | `{stream_key}` |
| 20 | STREAM_WATCH | `{stream_key}` |
| 21 | STREAM_PING | `{stream_key}` |
| 22 | STREAM_SET_PAUSED | `{stream_key, paused}` |
| 24 | REQUEST_GUILD_APPLICATION_COMMANDS | (enum only, no sender found in the socket) |
| 28 | REQUEST_FORUM_UNREADS | `{guild_id, channel_id, threads: [{thread_id, ack_message_id}]}` |
| 29 | REMOTE_COMMAND | `{target_session_id, payload}` |
| 30 | GET_DELETED_ENTITY_IDS_NOT_MATCHING_HASH | `{guild_id, channel_ids_hash, role_ids_hash, emoji_ids_hash, sticker_ids_hash}` |
| 31 | REQUEST_SOUNDBOARD_SOUNDS | `{guild_ids}` |
| 34 | REQUEST_LAST_MESSAGES | `{guild_id, channel_ids}` |
| 35 | SEARCH_RECENT_MEMBERS | `{guild_id, query, continuation_token}` |
| 37 | GUILD_SUBSCRIPTIONS_BULK | `{subscriptions: {guild_id: {...}}}`, split so each frame's subscriptions stay under 15,360 bytes of JSON |
| 38 | GUILD_CHANNELS_RESYNC | `{guild_id, obfuscated_channel_ids}` |
| 39 | REQUEST_CHANNEL_MEMBER_COUNT | `{guild_id, channel_id}` |
| 40 | QOS_HEARTBEAT | `{seq, qos}`, see above |
| 41 | UPDATE_TIME_SPENT_SESSION_ID | `{initialization_timestamp, session_id, client_launch_id}`, followed by a heartbeat |
| 43 | REQUEST_CHANNEL_INFO | `{guild_id, fields}` |

Op 14 (`GUILD_SUBSCRIPTIONS`) is still in the enum, but the client only sends
37. Commands sent while the session is not established are dropped with a
warning, except identify, resume and heartbeat.

## Dispatches

The client registers 201 dispatch names (`data/gateway-events.txt`). Most
become one Flux action of the same name. Some are batched or preloaded
first: messages, members and presences load their referenced users and
guilds before dispatching. `READY` sets the session and resume URL.
`READY_SUPPLEMENTAL` and `RESUMED` mark the session established. When a store
throws while handling a dispatch, the socket resets and clears caches
(`CLEAR_CACHES`), and it reconnects at once the first time.
