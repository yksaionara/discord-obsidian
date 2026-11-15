# Architecture of the official client

Three layers, one process tree:

```
Discord.exe (Electron main)
├─ resources/_app.asar        bootstrap: splash, updater (updater.node), module installer
├─ modules/discord_desktop_core/core.asar
│   ├─ bundle.js              main process: windows, tray, IPC handlers, settings, protocol handlers
│   ├─ mainScreenPreload.js   preload: builds window.DiscordNative via contextBridge
│   └─ frameViewPreload.js    preload for embedded frame views (activities, overlays)
├─ renderer: https://discord.com/app    ← the whole client, loaded from the network
│   web.<hash>.js (12.6 MB runtime + core) + ~7,000 lazily loaded chunks
└─ native modules (modules/discord_*-1/), loaded into the renderer through DiscordNative.nativeModules
    discord_voice     WebRTC fork + libdave (DAVE/MLS), capture, encoders, Krisp hookup
    discord_media     media helpers        discord_krisp   noise suppression models (.kef)
    discord_zstd      zstd-stream inflate for the gateway
    discord_erlpack   ETF encoding for the gateway
    discord_utils     input capture (push to talk), system helpers
    discord_hook      game overlay injection (DiscordHook64.dll)
    discord_game_utils, discord_dispatch, discord_cloudsync, discord_clips,
    discord_desktop_overlay, discord_notifications, discord_rpc, discord_spellcheck, discord_modules
```

The desktop app has no product code of its own. Its window loads the same
`discord.com/app` that browsers do, and it decides which code paths to run by
whether `window.DiscordNative` exists (`isPlatformEmbedded`). Everything a
browser can't do goes through that bridge: native voice, screen capture,
the overlay, game detection, zstd and ETF. In a browser the same code falls
back to the browser's WebRTC, `zlib-stream` and JSON.

`webPreferences` for the main window: `contextIsolation: true`,
`nodeIntegration: false`, `sandbox: false`, preload `mainScreenPreload.js`,
plus extra Blink features. The shell registers a privileged
`DISCORD_CLIP_PROTOCOL` scheme for serving clips. The updater talks to
`https://updates.discord.com/`.

## Inside the web client

- **Flux**: one dispatcher (`h.h.dispatch({type: ...})`) with stores
  subscribed by action type. Gateway dispatches are turned into Flux
  actions in one module, the one that registers handlers with
  `eH([...names], fn)`. See `data/gateway-events.txt`.
- **GatewaySocket**: the main gateway connection (see [gateway.md](gateway.md)).
  A "fast connect" path lets an inline script in `app.html` open the socket
  and identify before the bundle has loaded. The socket then adopts it
  (`window._ws`).
- **RTCConnection / RTCControlSocket**: one per voice or stream context
  (`DEFAULT` for voice, `STREAM` for Go Live). The control socket is the voice
  gateway (see [voice-gateway.md](voice-gateway.md)). Media runs in
  `discord_voice` on desktop and in browser WebRTC (`protocol: "webrtc"`,
  SDP) on the web.
- **HTTP**: superagent behind an `IA({prepareRequest, interceptResponse})`
  hook (see [http.md](http.md)).
- **Experiments**: user and guild experiments arrive in `READY` and are
  read through `getConfig({location})`. Many protocol choices hang on them,
  for example voice gateway v10 and the channel-obfuscation capability.
- **libdiscore**: a WebAssembly core (`libdiscore-wasm-fetch.*.js`) that
  has started taking over some stores. The socket dispatches
  `LIBDISCORE_RESET` when it resets.
- **Protobufs**: user settings (`USER_SETTINGS_PROTO_UPDATE`), experiments,
  surveys and the QoS token are `@bufbuild/protobuf` messages
  (`data/protos.txt`).
