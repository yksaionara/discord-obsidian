# Desktop shell and native modules

Source: `core.asar` (`bundle.js` main process, `mainScreenPreload.js`) and
each module's `index.js` under `modules/`.

## window.DiscordNative

The preload builds one object and exposes it with
`contextBridge.exposeInMainWorld("DiscordNative", …)`:

`nativeModules`, `process`, `os`, `app`, `clipboard`, `ipc`, `gpuSettings`,
`window`, `powerMonitor`, `ntpClock`, `spellCheck`, `crashReporter`,
`desktopCapture`, `activityCapture`, `fileManager`, `clips`, `processUtils`,
`powerSaveBlocker`, `frameDebugger`, `frameView`, `http`, `accessibility`,
`features`, `settings`, `userDataCache`, `thumbar`, `safeStorage`,
`hardware`, `tracing`, `riotGames`, `cs2Gsi`, `dotaGsi`,
`rocketLeagueStatsApi`, `webAuthn`, `gcEvents`, `sysimg`.

`nativeModules.requireModule(name)` loads a `discord_*` module into the
renderer. `ensureModule(name)` asks the updater to install it first. Each
namespace is a thin wrapper over IPC to the main process. The channels are all
named `DISCORD_<AREA>_<ACTION>` (212 of them), for example
`DISCORD_DESKTOP_CAPTURER_GET_SOURCES`, `DISCORD_SAFE_STORAGE_ENCRYPT_STRING`,
`DISCORD_SYSTEM_TRAY_SET_ICON`, `DISCORD_POWER_MONITOR_LOCK_SCREEN`,
`DISCORD_NTP_CLOCK_SYNCED`, `DISCORD_APP_SET_BADGE_COUNT`,
`DISCORD_NOTIFICATIONS_SEND_NOTIFICATION`, `DISCORD_FRAME_VIEW_*`
(out-of-process activity views) and `DISCORD_ACTIVITY_CAPTURE_*`.
To list them all:
`grep -oE '"DISCORD_[A-Z0-9_]+"' mainScreenPreload.js | sort -u`.

What the desktop builds use these for, beyond what a browser can do:

- **Encrypted storage**: when `safeStorage.isEncryptionAvailable()` is true
  (DPAPI on Windows), the client encrypts stored values with
  `encryptString` and marks them with a prefix, so it can tell them apart from
  plain values. The token is presumably one of them, but this read didn't
  trace which keys go through it.
- **Idle and lock**: `powerMonitor` events drive the AFK and idle state,
  which feeds the QoS `active` flag and analytics.
- **Game detection**: `discord_game_utils` and `discord_dispatch` list
  processes and match them against `/games/detectable` (and
  `/applications/non-games/detectable`). Some games
  get richer state from game state integrations (`cs2Gsi`, `dotaGsi`,
  `rocketLeagueStatsApi`, `riotGames` with its own `riotgames.pem`).
- **Overlay**: `discord_hook` injects `DiscordHook64.dll` (helpers in
  `DiscordHookHelper64.exe`). The same hook captures game frames for Go Live
  (`video_hook_source`).
- **Push to talk**: `discord_utils` `input_capture.js` hooks the keyboard and
  mouse globally.
- **Clips**: `discord_clips` keeps a replay buffer through the voice engine
  (`setClipRecordUser`) and serves the files over the privileged clip
  protocol.

## discord_voice

`index.js` wraps the addon as `VoiceEngine`. At top level:
`initialize`, `createVoiceConnectionWithOptions`,
`createOwnStreamConnectionWithOptions` (Go Live sender),
`createReplayConnection`, `createTransport`, `getCodecCapabilities`,
`setVideoOutputSink`, `addDirectVideoOutputSink`, `setKrispPath`,
`setMLPath` (MediaPipe background blur, `selfie_segmentation*.tflite`),
`setAudioSubsystem` (standard or legacy), `isSpatialAudioEnabled`.

Per connection: `setLocalMute`, `setLocalVolume`, `setLocalPan`,
`setPTTActive`, `mergeUsers`, `destroyUser`, `setDesktopSource[WithOptions]`,
`presentDesktopSourcePicker`, `setGoLiveDevices`, `getStats`,
`getFilteredStats`, `getEncryptionModes`, `fastUdpReconnect`, DAVE
(`prepareSecureFramesTransition`, `executeSecureFramesTransition`,
`prepareSecureFramesEpoch`, `getMLSKeyPackage`, `processMLSProposals`,
`processMLSWelcome`, `prepareMLSCommitTransition`,
`getMLSPairwiseFingerprint`), and callbacks (`setOnSpeakingCallback`,
`setOnVideoCallback`, `setOnSoundshare*`, `setPingCallback`,
`setOnConnectionFailedCallback`, `setOnFirstFrame*`).

The JavaScript keeps the voice gateway. The addon owns UDP, RTP and SRTP-like
encryption, codecs, jitter buffers, AEC and noise suppression, and DAVE frame
encryption. Hardware encoding runs out of process in `gpu_encoder_helper.exe`,
and audio effects in `audio_effects_helper.exe`.

## Local RPC (discord_rpc)

- IPC: named pipe `\\?\pipe\discord-ipc-0` … `-9` on Windows, or
  `$XDG_RUNTIME_DIR/discord-ipc-N` elsewhere. The client takes the first
  free slot of ten.
- WebSocket: a server on localhost. The upgrade passes `req.headers.origin`
  on for the client to check. The port range isn't in the module's JS, and
  this read didn't find where it's set. The public RPC docs say 6463–6472.
- IPC frames, as the public RPC docs describe them: a header of
  `[op u32 LE][len u32 LE]`, then JSON, with ops HANDSHAKE, FRAME, CLOSE,
  PING and PONG. The module emits `handshake`, `request` and `pong` events
  that match.
