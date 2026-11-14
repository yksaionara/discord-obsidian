# Discord Obsidian UI

The default presentation follows Discord's desktop client more closely. It is a
presentation layer over the same Slint component tree, adapter and core: no
Discord protocol, session or media code changes. `OBSIDIAN_UI=classic` selects
the earlier metrics, typeface and widgets, for regression checks and
comparison. Saved appearance choices and themes always win over these defaults.

## Where it lives

`ui/obsidian/style.slint` owns the metrics (`ClientStyle`): 304px navigation
beside the 72px rail, the 58px inset account card, text size, font family and
settings frame. Small bindings in the shared components read those values.
`src/ui/obsidian.rs` picks the presentation when the window is created. Newer
widgets reach the new look through the same exports, so a merge rarely needs
more than a metric.

Components, one file each in `ui/obsidian/`: `window-bar` (the
32px integrated Windows caption bar, with the same minimize, maximize, close,
drag and double-click actions), `dm-heading` (the Direct Messages heading with
add-friend and new-message actions), `account-popover` (status, activity, Edit
Profile, Copy User ID, User Settings), `settings-nav` (the user-settings
sidebar with a filter) and `settings-toggle` (switch faces on the right).

## Typeface

The UI embeds DM Sans, an OFL-licensed face close to Discord's gg sans (not an
exact copy; no Discord font is bundled). Default text is 15px; the explicit
small (14px) and large (18px) message sizes remain. Provenance and license:
`ui/fonts/README.md`. Binary packages carry the license (`scripts/release_package.py`,
`testing/test_font_notices.py`).

## Layout

The rail uses 40px tiles with a pinned Home and one separator. DM rows are 42px.
Message groups have a 16px leading gap, with a smaller timestamp. The docked
profile is a 320px column with a View Full Profile button. User settings open as
a modal with a 266px sidebar and a content column up to 696px.

## Motion

Transitions take their time from `Theme` in `ui/theme.slint`: `quick`
(120 ms) for a hover or press, `smooth` (200 ms) for something moving into
place, `slow` (320 ms) for larger pieces, and `Popup.duration` (160 ms, easing
out like Discord's modals from 90% of the card's size). Buttons give a little
under the pointer, the Home logo lifts on hover, and the rail's Read all pill
grows in and folds away. Appearance's "Reduce motion" (`reduce_motion` in
`layout.json`) makes every one of them take no time, so nothing redraws for an
animation. Animations only cost while they run: nothing is drawn between them,
and no picture is kept for one.

## Vice City

Typing "vice city" into Search settings shows a hidden Vice City page: a
sunset counting down to GTA VI (`src/ui/gta.rs`). It counts to midnight local
time on November 19, 2026, as Rockstar releases it, except in US time zones
(UTC-5 to UTC-10), whose stores unlock together at midnight Eastern. It ticks
once a second while it's on screen.

## Not yet Discord

Only implemented destinations appear. Missing: banner, badges and decorations
in the profile and account popover, clan tags, per-channel unread counts, the
and the quick switcher. Nitro, Shop and Quests are left out on purpose. Several need data the client
does not fetch yet.

## Tests

`src/ui/tests.rs` covers the new components' actions (caption buttons, DM
heading, account popover, settings navigation, pinned Home, typography) and has
a server-rail scroll benchmark (`obsidian_server_rail_scroll_snapshot_benchmark`).
