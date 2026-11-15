# Discord's own client, from its shipped code

How Discord's official desktop app works: it was read from its install
and from the web client that the app loads, with
[REA](https://github.com/morluto/rea) and the scripts in
`scripts/discord-re/`. docs.discord.food stays the reference for endpoints and
payloads. These pages cover what the client itself does on top of that:
which opcodes it actually sends, the headers and properties it attaches, and
how its desktop shell and native modules fit together.

Snapshot: desktop host `1.0.9268` (stable, win-x64), web build `673764`
(`VERSION_HASH 1a91563b`), API v9.

| Page | Covers |
|---|---|
| [architecture.md](architecture.md) | The three layers: the Electron shell, the web client and the native modules |
| [gateway.md](gateway.md) | Main gateway: URL, compression, identify, capabilities, QoS heartbeat, resume, every command |
| [voice-gateway.md](voice-gateway.md) | RTC control socket v9/v10: every opcode both ways, DAVE/MLS, codecs, encryption, close codes |
| [http.md](http.md) | REST pipeline: headers, super properties, captcha, MFA, retries |
| [desktop.md](desktop.md) | `DiscordNative` bridge, IPC channels, native modules, local RPC |
| [gaps.md](gaps.md) | Where Discord Obsidian differs from the official client, and what it doesn't do yet |

Tables extracted from the bundles, in `data/`:

| File | Contents |
|---|---|
| `rest-endpoints.tsv` | All 916 entries of the client's `Endpoints` map (`NAME<TAB>/path/{arg0}`) |
| `gateway-events.txt` | The 201 gateway dispatches the client registers a handler for |
| `gateway-events-unhandled.txt` | Those that Discord Obsidian's `obsidian-discord` never names |
| `enums.txt` | All 519 TypeScript enums in the bundles: opcodes, flags, error codes, close codes, analytics |
| `protos.txt` | 294 protobuf messages (`discord_protos.*`): user settings, experiments, QoS token |

## Refreshing

```sh
scripts/discord-re/extract /tmp/discord-web
```

It downloads `https://discord.com/app`, the ~360 chunks that page names, and
the ~6,600 chunks named by the webpack runtime's map: 180 MB, with locale
string tables taking most of it. It then splits every chunk into one
beautified file per webpack module (`<work>/mods/<id>.js`, about 19,000
files) and regenerates `data/`. Look up a module with
`grep -l 'RTCControlSocket' <work>/mods/*.js`. Module ids and minified names
change with every build, but string literals such as log prefixes, event names
and endpoint keys stay the same, so search by those.

The desktop parts come from the install itself, in
`%LOCALAPPDATA%\Discord\app-<version>\`. The original shell is
`resources\_app.asar` when a client mod has replaced `app.asar` (Equicord
does this; that's the case on the development machine). The main process is
`modules\discord_desktop_core-1\discord_desktop_core\core.asar`. Extract
either with `npx @electron/asar extract`.

What reading the code shows and what it can't: the code says what the client
sends and how it reacts. Whether a given path runs depends on experiments
and on the server. When this doc depends on an experiment, it says so.
