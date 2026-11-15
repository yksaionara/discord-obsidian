# Voice gateway (RTC control socket)

Source: the `RTCControlSocket` class (search for `RTCControlSocket(`), the
`RTCConnection` that drives it, and `discord_voice.node` for media. The same
socket class serves voice (`context DEFAULT`) and Go Live (`STREAM`).

## Version and URL

`wss://<endpoint>/?v=<9|10>`. The client asks for **v10** when the native
engine supports `UDP_ENDPOINT_UPDATE` and an experiment allows it, and **v9**
otherwise. The version the server reports in `HELLO.v` (default 3) switches
behaviour:

| server version | changes |
|---|---|
| ≤ 3 | `speaking` is a boolean. Heartbeat interval × 0.25 on desktop, × 0.1 on the web |
| ≥ 5 | the client sends op 15 `MEDIA_SINK_WANTS` |
| ≥ 6 | the client sends op 16 `{}` after `READY` and gets the backend version back |
| ≥ 8 | heartbeats are `{t, seq_ack}`, the ack is `{t}`, and binary frames from the server start with a 2-byte sequence number |
| ≥ 10 | op 33 SFU update (the SFU can move the UDP endpoint mid-call) |

For v ≥ 4 the interval is `heartbeat_interval × modifier`. **The web client
caps it at 5 s**, while desktop uses the server's value. The hello timeout is
20 s, and a resume needs an ack within the last minute. Backoff runs from 1 s
to 5 s. After more than 3 failures, or on close codes 4004, 4006, 4011 or 4015,
the socket gives up and emits `disconnect`.

## Frames

JSON: `{"op": n, "d": ..., "seq": n?}`. The client tracks `seq` for
`seq_ack`. Binary frames from the server are `[seq u16 BE if v≥8][op u8][payload]`.
Binary frames from the client are `[op u8][payload]`. A binary frame shorter
than its header, or a DAVE payload that doesn't validate, closes the socket
with **4805**.

## Client → server

| op | meaning | payload |
|---|---|---|
| 0 | IDENTIFY | `{server_id, channel_id, user_id, session_id, token, max_dave_protocol_version, video, streams}` |
| 1 | SELECT_PROTOCOL | `{protocol: "udp", data: {address, port, mode}, address, port, mode, codecs, rtc_connection_id, experiments}`, or `{protocol: "webrtc", data: "<sdp>", sdp, codecs, rtc_connection_id}` |
| 3 | HEARTBEAT | v≥8 `{t: now_ms, seq_ack}`, older servers get the bare `now_ms` |
| 5 | SPEAKING | `{speaking: flags, delay, ssrc}` |
| 7 | RESUME | `{token, session_id, server_id, channel_id, seq_ack}` |
| 12 | VIDEO | `{audio_ssrc, video_ssrc, rtx_ssrc, streams}` |
| 14 | SESSION_UPDATE | `{codecs}`, for a codec change mid-call |
| 15 | MEDIA_SINK_WANTS | `{<ssrc or "any">: quality 0–100, ...}`, which streams to receive and at what quality |
| 16 | VOICE_BACKEND_VERSION | `{}` |
| 23 | DAVE_TRANSITION_READY | `{transition_id}` |
| 26 | MLS_KEY_PACKAGE | binary |
| 28 | MLS_COMMIT_WELCOME | binary |
| 31 | MLS_INVALID_COMMIT_WELCOME | `{transition_id}` |
| 32 | NO_ROUTE | `{}`. UDP never connected |
| 34 | START_WEBTRANSPORT | `{}` |

`streams` entries: `{type, rid, ssrc, active, quality, rtx_ssrc, max_bitrate,
max_framerate, max_resolution: {type, width, height}}`. The audio stream is
`{type: audio, rid: "", max_bitrate: 64000}`, and Go Live audio carries
`soundshare: true`.

## Server → client

| op | meaning | client action |
|---|---|---|
| 2 | READY | `{ip, port, modes, ssrc, streams, experiments}`. Starts UDP, then sends op 1 |
| 3 | HEARTBEAT request | beat now |
| 4 | SESSION_DESCRIPTION | `{mode, secret_key}` or `{sdp}`, plus `audio_codec`, `video_codec`, `media_session_id`, `keyframe_interval`, `bandwidth_estimation_experiment`, and **`dave_protocol_version`, which is required: without it the client closes with 4805** |
| 5 | SPEAKING | `{user_id, ssrc, speaking}` |
| 6 | HEARTBEAT_ACK | `{t}` (v≥8). The client reports half the round trip as ping |
| 8 | HELLO | `{v, heartbeat_interval}` |
| 9 | RESUMED | |
| 11 | CLIENTS_CONNECT | `{user_ids}` |
| 12 | VIDEO | `{user_id, audio_ssrc, video_ssrc, streams}` |
| 13 | CLIENT_DISCONNECT | `{user_id}` |
| 14 | SESSION_UPDATE | `{audio_codec?, video_codec?, media_session_id?, keyframe_interval?}` |
| 15 | MEDIA_SINK_WANTS | the server's wants for what this client sends |
| 16 | VOICE_BACKEND_VERSION | `{voice, rtc_worker}` |
| 18 | CLIENT_FLAGS | `{user_id, flags}` |
| 20 | CLIENT_PLATFORM | `{user_id, platform}` |
| 21 | DAVE_PREPARE_TRANSITION | `{transition_id, protocol_version}` |
| 22 | DAVE_EXECUTE_TRANSITION | `{transition_id}` |
| 24 | DAVE_PREPARE_EPOCH | `{epoch, protocol_version}`, where `epoch` must be a safe non-negative integer |
| 25 | MLS_EXTERNAL_SENDER | binary |
| 27 | MLS_PROPOSALS | binary |
| 29 | MLS_ANNOUNCE_COMMIT_TRANSITION | binary: `transition_id u16` + commit |
| 30 | MLS_WELCOME | binary: `transition_id u16` + welcome |
| 33 | SFU_UPDATE | new UDP endpoint (v10) |
| 35 | WEBTRANSPORT_ACK | `{token, certificates: [{algorithm, hash(base64)}]}` |

The bundle has no names for opcodes, only numbers in a switch. The names here
follow docs.discord.food where it has them. 32–35 are named after what
the client does with them.

The client's own close codes: 4800 heartbeat timeout, 4801 cannot resume,
4802 reset backoff, 4803 repeated invalid MLS commit/welcome, 4804 refused
DAVE downgrade to version 0, 4805 invalid DAVE message.

## Media, from discord_voice.node

- **Codecs** in SELECT_PROTOCOL: Opus first at payload type 120, then
  each video codec the engine reports, in order of preference: payload type
  `101 + 2i`, RTX `102 + 2i`, `priority` × 1000, with `encode` and `decode`
  flags. AV1 goes over the wire as `AV1X`. Seen: AV1, H265, H264, VP8, VP9.
- **Transport encryption** that the engine knows: `aead_aes256_gcm_rtpsize`,
  `aead_xchacha20_poly1305_rtpsize`, plus the retired `aead_aes256_gcm` and
  `xsalsa20_poly1305[_lite|_suffix][_rtpsize]`. The mode is picked from
  `READY.modes` and confirmed by op 4.
- **E2EE**: libdave in C++ on BoringSSL, with MLS sessions and a persisted
  key pair (Windows CNG on Windows). Frames are encrypted per codec by
  `frame_processors`.
- **Bandwidth and quality** are WebRTC field trials, set as
  `DiscordFieldTrials`, many of them from voice-server `experiments`.
- **Go Live** goes through a second RTCConnection with context `STREAM`.
  The streamer's encoder gets a "fake" 1080p pixel count when the user's
  plan allows 1080p. Capture uses `video_source_win` and `video_hook_source`
  (the game hook) or `gpu_encoder_helper.exe`. `soundshare` captures app
  audio.
