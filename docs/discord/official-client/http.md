# REST pipeline

Source: the module that calls `IA({prepareRequest, interceptResponse})`
(search for `"X-Discord-Locale"`) and the super-properties builder (search for
`"Discord Client"`).

## Every request to a relative path

`prepareRequest` prefixes `https://discord.com/api/v9` and sets:

| header | value |
|---|---|
| `Authorization` | the token, unless the request already set one |
| `X-Super-Properties` | base64 JSON, see below |
| `X-Fingerprint` | while one is stored: the fingerprint `/experiments` hands out before login |
| `X-Installation-ID` | when one is known, for tracking |
| `Accept-Language` | desktop only: `navigator.languages` with weights `en-US,en;q=0.9,…` |
| `X-Discord-Locale` | the UI locale |
| `X-Discord-Timezone` | IANA zone, e.g. `America/Puerto_Rico` |
| `X-Debug-Options` | only when a developer option is set |
| `X-Routing-Key` | only when one is set (internal routing) |
| `x-client-trace-id` | only while staff request tracing is on |

Other headers some calls send: `X-Context-Properties` (base64 JSON with
the UI location, for invites, joins and friend requests),
`X-Audit-Log-Reason`, `X-Discord-MFA-Authorization` (the token from an
MFA prompt), `X-Captcha-Key`, `X-Captcha-Rqtoken`, `X-Captcha-Session-Id`.

A sampled `HTTP_REQUEST` analytics event records the method, the status, the
duration and the URL with digits replaced by `#`.

## Super properties (desktop)

```json
{
  "os": "Windows", "browser": "Discord Client", "release_channel": "stable",
  "client_version": "1.0.9268", "os_version": "10.0.26200", "os_arch": "x64", "app_arch": "x64",
  "os_sdk_version": "26200", "os_build_revision": "...",
  "system_locale": "en-US", "has_client_mods": false,
  "browser_user_agent": "Mozilla/5.0 ... discord/1.0.9268 Chrome/... Electron/... Safari/537.36",
  "browser_version": "<electron version>",
  "client_launch_id": "<uuid per launch>", "launch_signature": "<uuid-like>",
  "client_heartbeat_session_id": "<uuid>",
  "client_build_number": 673764, "native_build_number": "<DiscordNative.app.getBuildNumber()>",
  "client_event_source": null
}
```

On Linux it adds `window_manager`, `distro`, `runtime_environment` and
`display_server`. The web client sends `os` and `browser` from the user agent, with
`device`, `referrer`, `referring_domain`, `referrer_current`,
`referring_domain_current` and `release_channel`.

`has_client_mods` is true when any of `window.jQuery`, `$`, `BetterDiscord`,
`BdApi`, `rambox`, `Vencord` or `VencordNative` exists. The same check also
feeds `uses_client_mods` in the analytics heartbeat, and `modded_client`
tags on socket-crash metrics.

`client_build_number` is baked into the bundle as a literal, the same as
`GLOBAL_ENV.BUILD_NUMBER` in `app.html`.

## Responses

- **400 with `captcha_key`**: opens the captcha (hCaptcha) with the
  response's sitekey, `captcha_rqdata` and `captcha_rqtoken`, then retries the
  request with the three captcha headers.
- **401 with code 60003 `MFA_REQUIRED` and an `mfa` object**: opens the MFA
  modal with the methods the `mfa` object lists. The modal finishes at
  `/mfa/finish` (`FINISH_MFA_CHECK`), and the request is retried with the
  headers it returns (`X-Discord-MFA-Authorization`).
- **403 with a code in the guild-limited-access range**: shows the
  limited-access notice for `guild_id`.
- **403 `RESTRICTED_HOURS_ACTIVE`**: shows the restricted-hours modal
  (parental controls).
- When a request fails at the network, the client waits until it is online
  and retries.

JSON error codes are the enum that contains `MFA_REQUIRED = 60003` in
`data/enums.txt`.

## Login by QR code (remote auth)

`wss://remote-auth-gateway.discord.gg/?v=2`, JSON frames with `op` strings:

1. → `init {encoded_public_key}`: an RSA key pair made per session
2. ← `hello {heartbeat_interval, timeout_ms}`. Heartbeat `{op: heartbeat}`
   after a random `0..interval`, ack `heartbeat_ack`
3. ← `nonce_proof {encrypted_nonce}` → `nonce_proof {nonce}` (the decrypted nonce)
4. ← `pending_remote_init {fingerprint}`: the QR shows
   `https://discord.com/ra/<fingerprint>`, and the client checks that the
   fingerprint is that of its own key
5. ← `pending_ticket {encrypted_user_payload}` decrypts to `id:discrim:avatar:username`
6. ← `pending_login {ticket}` → `POST /users/@me/remote-auth/login {ticket}`
   returns `encrypted_token`, which the private key decrypts
7. ← `cancel` starts over. The client reconnects with backoff from 1.5 s to 30 s

## Endpoints

`data/rest-endpoints.tsv` is the client's whole `Endpoints` map, in source
order. Arguments appear as `{arg0}`, `{arg1}`. Where the source computes a
default, the raw expression is kept.
