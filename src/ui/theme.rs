//! The window's colours: a full set of what `Theme` in `ui/theme.slint` holds,
//! which Discord Obsidian sets on the window to change them all at once.

use std::{cell::Cell, rc::Rc};

use obsidian_ui::ObsidianWindow;
use slint::{Color, ComponentHandle, Model, ModelRc, VecModel};

/// Opaque `#rrggbb`.
const fn rgb(hex: u32) -> Color {
    Color::from_argb_encoded(0xff00_0000 | hex)
}

/// `#rrggbbaa`, as Slint writes a colour with alpha.
const fn rgba(hex: u32) -> Color {
    Color::from_argb_encoded(hex.rotate_right(8))
}

/// Declares `Theme` with one field per colour of the Slint global, and the
/// window's getter and setter for each.
macro_rules! theme {
    ($($name:ident ($get:ident, $set:ident),)*) => {
        /// A full set of the window's colours, and whether they make a dark
        /// theme. `ui/theme.slint` describes each.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Theme {
            pub dark: bool,
            $(pub $name: Color,)*
        }

        impl Theme {
            /// Sets only the colours that differ from those the window
            /// shows: nothing when it starts in Yap, its defaults.
            fn set(&self, tokens: &obsidian_ui::Theme<'_>) {
                if tokens.get_dark() != self.dark {
                    tokens.set_dark(self.dark);
                }
                $(if tokens.$get() != self.$name {
                    tokens.$set(self.$name);
                })*
            }

            /// Each colour's name, as `ui/theme.slint` has it but with `_`
            /// for `-`, in the order of [`Self::colour`].
            pub const NAMES: &[&str] = &[$(stringify!($name),)*];

            pub fn colour(&self, index: usize) -> Option<Color> {
                [$(self.$name,)*].get(index).copied()
            }

            pub fn colour_mut(&mut self, index: usize) -> Option<&mut Color> {
                [$(&mut self.$name,)*].into_iter().nth(index)
            }

            /// The colours the window shows now.
            #[cfg(test)]
            pub fn shown(window: &ObsidianWindow) -> Self {
                let tokens = window.global::<obsidian_ui::Theme>();
                Self { dark: tokens.get_dark(), $($name: tokens.$get(),)* }
            }
        }
    };
}

theme! {
    rail(get_rail, set_rail),
    sidebar(get_sidebar, set_sidebar),
    panel(get_panel, set_panel),
    chat(get_chat, set_chat),
    stage(get_stage, set_stage),
    raised(get_raised, set_raised),
    field(get_field, set_field),
    sunken(get_sunken, set_sunken),
    hover(get_hover, set_hover),
    active(get_active, set_active),
    row_hover(get_row_hover, set_row_hover),
    line(get_line, set_line),
    edge(get_edge, set_edge),
    outline(get_outline, set_outline),
    outline_chat(get_outline_chat, set_outline_chat),
    outline_field(get_outline_field, set_outline_field),
    text(get_text, set_text),
    secondary(get_secondary, set_secondary),
    muted(get_muted, set_muted),
    faint(get_faint, set_faint),
    accent(get_accent, set_accent),
    accent_hover(get_accent_hover, set_accent_hover),
    on_accent(get_on_accent, set_on_accent),
    accent_soft(get_accent_soft, set_accent_soft),
    danger(get_danger, set_danger),
    danger_hover(get_danger_hover, set_danger_hover),
    on_danger(get_on_danger, set_on_danger),
    live(get_live, set_live),
    loud(get_loud, set_loud),
    link(get_link, set_link),
    button_default(get_button_default, set_button_default),
    button_default_hover(get_button_default_hover, set_button_default_hover),
    overlay(get_overlay, set_overlay),
    shimmer(get_shimmer, set_shimmer),
    reply_spine(get_reply_spine, set_reply_spine),
    tooltip(get_tooltip, set_tooltip),
    avatar(get_avatar, set_avatar),
}

/// Discord Obsidian's own colours: layered graphite with a marigold accent. The defaults
/// in `ui/theme.slint` are these, so the first frame shows them.
pub const YAP: Theme = Theme {
    dark: true,
    rail: rgb(0x111215),
    sidebar: rgb(0x17181c),
    panel: rgb(0x121316),
    chat: rgb(0x1e1f24),
    stage: rgb(0x1e1f24),
    raised: rgb(0x26272e),
    field: rgb(0x2a2b32),
    sunken: rgb(0x0e0f12),
    hover: rgb(0x222329),
    active: rgb(0x2c2d35),
    row_hover: rgb(0x1a1b20),
    line: rgb(0x2c2d34),
    outline: rgb(0x2c2d34),
    outline_chat: rgb(0x2c2d34),
    outline_field: rgb(0x2c2d34),
    edge: rgb(0x0c0d0f),
    text: rgb(0xeef0f4),
    secondary: rgb(0xb5b8c1),
    muted: rgb(0x8d919c),
    faint: rgb(0x5c606a),
    accent: rgb(0xffbf47),
    accent_hover: rgb(0xffcd70),
    on_accent: rgb(0x1d1607),
    accent_soft: rgba(0xffbf4726),
    danger: rgb(0xf0545e),
    danger_hover: rgb(0xf46f77),
    on_danger: rgb(0xffffff),
    live: rgb(0x3ecf7e),
    loud: rgb(0xff8c42),
    link: rgb(0x7cc4ff),
    button_default: rgb(0x33343c),
    button_default_hover: rgb(0x3c3d46),
    overlay: rgb(0xffffff),
    shimmer: rgba(0xffffff10),
    reply_spine: rgb(0x4a4c56),
    tooltip: rgb(0x0b0c0e),
    avatar: rgb(0x6b7280),
};

/// Discord's Dark theme (`.theme-dark` with `.theme-darker` in its web
/// client's CSS) on Discord Obsidian's surfaces, with blurple for the accent;
/// once called Bonfire.
const BONFIRE: Theme = Theme {
    dark: true,
    rail: rgb(0x121214),
    sidebar: rgb(0x121214),
    panel: rgb(0x202024),
    chat: rgb(0x1a1a1e),
    stage: rgb(0x1a1a1e),
    raised: rgb(0x242429),
    field: rgb(0x202024),
    sunken: rgb(0x121214),
    hover: rgb(0x222225),
    active: rgb(0x2d2d30),
    row_hover: rgb(0x242428),
    line: rgb(0x38383f),
    outline: rgb(0x222225),
    outline_chat: rgb(0x29292d),
    outline_field: rgb(0x27282b),
    edge: rgb(0x303036),
    text: rgb(0xefeff1),
    secondary: rgb(0xabacb2),
    muted: rgb(0x96979e),
    faint: rgb(0x6c6f7c),
    accent: rgb(0x5865f2),
    accent_hover: rgb(0x4452bb),
    on_accent: rgb(0xffffff),
    accent_soft: rgba(0x5865f226),
    danger: rgb(0xd22d39),
    danger_hover: rgb(0xa9232e),
    on_danger: rgb(0xffffff),
    live: rgb(0x23a55a),
    loud: rgb(0xf0b232),
    link: rgb(0x4d96ee),
    button_default: rgb(0x323237),
    button_default_hover: rgb(0x36363c),
    overlay: rgb(0xffffff),
    shimmer: rgba(0xffffff10),
    reply_spine: rgb(0x474851),
    tooltip: rgb(0x242429),
    avatar: rgb(0x5c5e66),
};

/// Catppuccin's darkest flavour, with mauve for the accent.
const CATPPUCCIN_MOCHA: Theme = Theme {
    dark: true,
    rail: rgb(0x11111b),
    sidebar: rgb(0x181825),
    panel: rgb(0x11111b),
    chat: rgb(0x1e1e2e),
    stage: rgb(0x1e1e2e),
    raised: rgb(0x313244),
    field: rgb(0x313244),
    sunken: rgb(0x11111b),
    hover: rgb(0x282839),
    active: rgb(0x45475a),
    row_hover: rgb(0x1a1a29),
    line: rgb(0x313244),
    outline: rgb(0x313244),
    outline_chat: rgb(0x313244),
    outline_field: rgb(0x313244),
    edge: rgb(0x11111b),
    text: rgb(0xcdd6f4),
    secondary: rgb(0xbac2de),
    muted: rgb(0x9399b2),
    faint: rgb(0x6c7086),
    accent: rgb(0xcba6f7),
    accent_hover: rgb(0xccb4f6),
    on_accent: rgb(0x11111b),
    accent_soft: rgba(0xcba6f726),
    danger: rgb(0xf38ba8),
    danger_hover: rgb(0xeba0ac),
    on_danger: rgb(0x11111b),
    live: rgb(0xa6e3a1),
    loud: rgb(0xfab387),
    link: rgb(0x89b4fa),
    button_default: rgb(0x45475a),
    button_default_hover: rgb(0x585b70),
    overlay: rgb(0xffffff),
    shimmer: rgba(0xffffff10),
    reply_spine: rgb(0x585b70),
    tooltip: rgb(0x11111b),
    avatar: rgb(0x6c7086),
};

/// Catppuccin's light flavour, with mauve for the accent.
const CATPPUCCIN_LATTE: Theme = Theme {
    dark: false,
    rail: rgb(0xdce0e8),
    sidebar: rgb(0xe6e9ef),
    panel: rgb(0xdce0e8),
    chat: rgb(0xeff1f5),
    stage: rgb(0xeff1f5),
    raised: rgb(0xe6e9ef),
    field: rgb(0xdce0e8),
    sunken: rgb(0xdce0e8),
    hover: rgb(0xe0e4eb),
    active: rgb(0xccd0da),
    row_hover: rgb(0xeaedf2),
    line: rgb(0xccd0da),
    outline: rgb(0xccd0da),
    outline_chat: rgb(0xccd0da),
    outline_field: rgb(0xccd0da),
    edge: rgb(0xbcc0cc),
    text: rgb(0x4c4f69),
    secondary: rgb(0x5c5f77),
    muted: rgb(0x7c7f93),
    faint: rgb(0x8c8fa1),
    accent: rgb(0x8839ef),
    accent_hover: rgb(0x793ece),
    on_accent: rgb(0xeff1f5),
    accent_soft: rgba(0x8839ef26),
    danger: rgb(0xd20f39),
    danger_hover: rgb(0xb01f45),
    on_danger: rgb(0xeff1f5),
    live: rgb(0x40a02b),
    loud: rgb(0xfe640b),
    link: rgb(0x1e66f5),
    button_default: rgb(0xccd0da),
    button_default_hover: rgb(0xbcc0cc),
    overlay: rgb(0x000000),
    shimmer: rgba(0xffffff80),
    reply_spine: rgb(0xacb0be),
    tooltip: rgb(0xeff1f5),
    avatar: rgb(0x7c7f93),
};

/// Dracula, with purple for the accent.
const DRACULA: Theme = Theme {
    dark: true,
    rail: rgb(0x191a21),
    sidebar: rgb(0x21222c),
    panel: rgb(0x191a21),
    chat: rgb(0x282a36),
    stage: rgb(0x282a36),
    raised: rgb(0x343746),
    field: rgb(0x343746),
    sunken: rgb(0x191a21),
    hover: rgb(0x2e303e),
    active: rgb(0x44475a),
    row_hover: rgb(0x242631),
    line: rgb(0x343746),
    outline: rgb(0x343746),
    outline_chat: rgb(0x343746),
    outline_field: rgb(0x343746),
    edge: rgb(0x191a21),
    text: rgb(0xf8f8f2),
    secondary: rgb(0xcbd0db),
    muted: rgb(0x9ea8c3),
    faint: rgb(0x6272a4),
    accent: rgb(0xbd93f9),
    accent_hover: rgb(0xccacf7),
    on_accent: rgb(0x191a21),
    accent_soft: rgba(0xbd93f926),
    danger: rgb(0xff5555),
    danger_hover: rgb(0xfe7674),
    on_danger: rgb(0x191a21),
    live: rgb(0x50fa7b),
    loud: rgb(0xffb86c),
    link: rgb(0x8be9fd),
    button_default: rgb(0x44475a),
    button_default_hover: rgb(0x4e5674),
    overlay: rgb(0xffffff),
    shimmer: rgba(0xffffff10),
    reply_spine: rgb(0x44475a),
    tooltip: rgb(0x191a21),
    avatar: rgb(0x6272a4),
};

/// Nord's Polar Night and Snow Storm, with frost for the accent. Polar
/// Night has no shade below nord0, so the rail and sidebar darken it.
const NORD: Theme = Theme {
    dark: true,
    rail: rgb(0x242932),
    sidebar: rgb(0x292e39),
    panel: rgb(0x242932),
    chat: rgb(0x2e3440),
    stage: rgb(0x2e3440),
    raised: rgb(0x3b4252),
    field: rgb(0x3b4252),
    sunken: rgb(0x21252e),
    hover: rgb(0x343b49),
    active: rgb(0x434c5e),
    row_hover: rgb(0x2b313c),
    line: rgb(0x3b4252),
    outline: rgb(0x3b4252),
    outline_chat: rgb(0x3b4252),
    outline_field: rgb(0x3b4252),
    edge: rgb(0x1f232c),
    text: rgb(0xeceff4),
    secondary: rgb(0xd8dee9),
    muted: rgb(0x99a1b0),
    faint: rgb(0x616e88),
    accent: rgb(0x88c0d0),
    accent_hover: rgb(0xa6cedb),
    on_accent: rgb(0x2e3440),
    accent_soft: rgba(0x88c0d026),
    danger: rgb(0xbf616a),
    danger_hover: rgb(0xc6767f),
    on_danger: rgb(0xeceff4),
    live: rgb(0xa3be8c),
    loud: rgb(0xd08770),
    link: rgb(0x81a1c1),
    button_default: rgb(0x434c5e),
    button_default_hover: rgb(0x4c566a),
    overlay: rgb(0xffffff),
    shimmer: rgba(0xffffff10),
    reply_spine: rgb(0x4c566a),
    tooltip: rgb(0x1f232c),
    avatar: rgb(0x616e88),
};

/// Gruvbox's dark palette at medium contrast, with orange for the accent.
const GRUVBOX_DARK: Theme = Theme {
    dark: true,
    rail: rgb(0x1d2021),
    sidebar: rgb(0x222424),
    panel: rgb(0x1d2021),
    chat: rgb(0x282828),
    stage: rgb(0x282828),
    raised: rgb(0x3c3836),
    field: rgb(0x32302f),
    sunken: rgb(0x171a1a),
    hover: rgb(0x32302f),
    active: rgb(0x504945),
    row_hover: rgb(0x222424),
    line: rgb(0x3c3836),
    outline: rgb(0x3c3836),
    outline_chat: rgb(0x3c3836),
    outline_field: rgb(0x3c3836),
    edge: rgb(0x141617),
    text: rgb(0xebdbb2),
    secondary: rgb(0xd5c4a1),
    muted: rgb(0xa89984),
    faint: rgb(0x7c6f64),
    accent: rgb(0xfe8019),
    accent_hover: rgb(0xfc9822),
    on_accent: rgb(0x1d2021),
    accent_soft: rgba(0xfe801926),
    danger: rgb(0xfb4934),
    danger_hover: rgb(0xfb6b51),
    on_danger: rgb(0x1d2021),
    live: rgb(0xb8bb26),
    loud: rgb(0xfabd2f),
    link: rgb(0x83a598),
    button_default: rgb(0x504945),
    button_default_hover: rgb(0x665c54),
    overlay: rgb(0xffffff),
    shimmer: rgba(0xffffff10),
    reply_spine: rgb(0x665c54),
    tooltip: rgb(0x141617),
    avatar: rgb(0x7c6f64),
};

/// Tokyo Night, the Night style, with blue for the accent.
const TOKYO_NIGHT: Theme = Theme {
    dark: true,
    rail: rgb(0x13131a),
    sidebar: rgb(0x16161e),
    panel: rgb(0x13131a),
    chat: rgb(0x1a1b26),
    stage: rgb(0x1a1b26),
    raised: rgb(0x1f2335),
    field: rgb(0x1f2335),
    sunken: rgb(0x121218),
    hover: rgb(0x1c1f2e),
    active: rgb(0x292e42),
    row_hover: rgb(0x181822),
    line: rgb(0x292e42),
    outline: rgb(0x292e42),
    outline_chat: rgb(0x292e42),
    outline_field: rgb(0x292e42),
    edge: rgb(0x0f0f15),
    text: rgb(0xc0caf5),
    secondary: rgb(0xa9b1d6),
    muted: rgb(0x737aa2),
    faint: rgb(0x545c7e),
    accent: rgb(0x7aa2f7),
    accent_hover: rgb(0x80bafa),
    on_accent: rgb(0x16161e),
    accent_soft: rgba(0x7aa2f726),
    danger: rgb(0xf7768e),
    danger_hover: rgb(0xec87a3),
    on_danger: rgb(0x16161e),
    live: rgb(0x9ece6a),
    loud: rgb(0xff9e64),
    link: rgb(0x7dcfff),
    button_default: rgb(0x292e42),
    button_default_hover: rgb(0x3b4261),
    overlay: rgb(0xffffff),
    shimmer: rgba(0xffffff10),
    reply_spine: rgb(0x3b4261),
    tooltip: rgb(0x0f0f15),
    avatar: rgb(0x565f89),
};

/// Midnight: Discord Obsidian's own, near black with a deep navy cast and a
/// violet accent, for late nights and OLED screens.
const MIDNIGHT: Theme = Theme {
    dark: true,
    rail: rgb(0x040509),
    sidebar: rgb(0x080a11),
    panel: rgb(0x040509),
    chat: rgb(0x0b0d15),
    stage: rgb(0x0b0d15),
    raised: rgb(0x121522),
    field: rgb(0x111420),
    sunken: rgb(0x06070c),
    hover: rgb(0x141726),
    active: rgb(0x1d2134),
    row_hover: rgb(0x0f111b),
    line: rgb(0x191c2b),
    outline: rgb(0x1d2134),
    outline_chat: rgb(0x191c2b),
    outline_field: rgb(0x23283d),
    edge: rgb(0x020306),
    text: rgb(0xe6e8f3),
    secondary: rgb(0xb4b9d0),
    muted: rgb(0x7c829f),
    faint: rgb(0x4c5270),
    accent: rgb(0x8b7bff),
    accent_hover: rgb(0x9d90ff),
    on_accent: rgb(0xffffff),
    accent_soft: rgba(0x8b7bff26),
    danger: rgb(0xff5c7a),
    danger_hover: rgb(0xff7a92),
    on_danger: rgb(0xffffff),
    live: rgb(0x3ddc97),
    loud: rgb(0xffb454),
    link: rgb(0x82b8ff),
    button_default: rgb(0x1d2134),
    button_default_hover: rgb(0x292e47),
    overlay: rgb(0xffffff),
    shimmer: rgba(0xffffff0d),
    reply_spine: rgb(0x292e47),
    tooltip: rgb(0x020306),
    avatar: rgb(0x383e5c),
};

/// Rosé Pine's light variant, with rose for the accent.
const ROSE_PINE_DAWN: Theme = Theme {
    dark: false,
    rail: rgb(0xdfdad9),
    sidebar: rgb(0xf2e9e1),
    panel: rgb(0xdfdad9),
    chat: rgb(0xfaf4ed),
    stage: rgb(0xfaf4ed),
    raised: rgb(0xfffaf3),
    field: rgb(0xf2e9e1),
    sunken: rgb(0xf4ede8),
    hover: rgb(0xeae3de),
    active: rgb(0xdfdad9),
    row_hover: rgb(0xf4ede8),
    line: rgb(0xdfdad9),
    outline: rgb(0xdfdad9),
    outline_chat: rgb(0xdfdad9),
    outline_field: rgb(0xdfdad9),
    edge: rgb(0xcecacd),
    text: rgb(0x575279),
    secondary: rgb(0x797593),
    muted: rgb(0x87829b),
    faint: rgb(0x9893a5),
    accent: rgb(0xd7827e),
    accent_hover: rgb(0xc9767c),
    on_accent: rgb(0x39354f),
    accent_soft: rgba(0xd7827e2e),
    danger: rgb(0xb4637a),
    danger_hover: rgb(0x9d5f7a),
    on_danger: rgb(0xfffaf3),
    live: rgb(0x56949f),
    loud: rgb(0xea9d34),
    link: rgb(0x286983),
    button_default: rgb(0xdfdad9),
    button_default_hover: rgb(0xcecacd),
    overlay: rgb(0x575279),
    shimmer: rgba(0xffffff99),
    reply_spine: rgb(0xcecacd),
    tooltip: rgb(0xfffaf3),
    avatar: rgb(0x9893a5),
};

/// Discord Obsidian's own: cherry blossom on rice paper, pink on a pale pink ground.
const SAKURA: Theme = Theme {
    dark: false,
    rail: rgb(0xf6dbe5),
    sidebar: rgb(0xfcecf2),
    panel: rgb(0xf6dbe5),
    chat: rgb(0xfff8fb),
    stage: rgb(0xfff8fb),
    raised: rgb(0xfbe9f0),
    field: rgb(0xfbe5ee),
    sunken: rgb(0xf8e1ea),
    hover: rgb(0xf8e2eb),
    active: rgb(0xf2d1de),
    row_hover: rgb(0xfdf1f6),
    line: rgb(0xf2d6e1),
    outline: rgb(0xf2d6e1),
    outline_chat: rgb(0xf2d6e1),
    outline_field: rgb(0xf2d6e1),
    edge: rgb(0xe9c6d3),
    text: rgb(0x4a3541),
    secondary: rgb(0x6d5361),
    muted: rgb(0x957685),
    faint: rgb(0xa8899a),
    accent: rgb(0xd5588a),
    accent_hover: rgb(0xc44a7b),
    on_accent: rgb(0xffffff),
    accent_soft: rgba(0xd5588a24),
    danger: rgb(0xd2404f),
    danger_hover: rgb(0xbd3242),
    on_danger: rgb(0xffffff),
    live: rgb(0x2f9e6e),
    loud: rgb(0xee8a45),
    link: rgb(0x6a5ad0),
    button_default: rgb(0xf4d6e1),
    button_default_hover: rgb(0xedc6d4),
    overlay: rgb(0x6b2d4a),
    shimmer: rgba(0xffffffa0),
    reply_spine: rgb(0xe2bccb),
    tooltip: rgb(0xffffff),
    avatar: rgb(0xc08aa2),
};

/// `a` mixed towards `b` by `amount` thousandths, as `0xrrggbb`.
const fn mix_hex(a: u32, b: u32, amount: u32) -> u32 {
    let amount = if amount > 1000 { 1000 } else { amount };
    mix_channel(a, b, amount, 16) | mix_channel(a, b, amount, 8) | mix_channel(a, b, amount, 0)
}

const fn mix_channel(a: u32, b: u32, amount: u32, shift: u32) -> u32 {
    let (from, to) = ((a >> shift) & 0xff, (b >> shift) & 0xff);
    ((from * (1000 - amount) + to * amount + 500) / 1000) << shift
}

/// Discord's grey washes (`#97979f` at some opacity) laid over a ground.
const fn wash(ground: u32, amount: u32) -> u32 {
    mix_hex(ground, 0x97979f, amount)
}

/// A Discord theme's grounds, from its CSS variables: `--background-base-lowest`,
/// `-lower`, `-low` and `--background-surface-high`.
struct Grounds {
    lowest: u32,
    lower: u32,
    low: u32,
    high: u32,
}

/// A Discord theme's text and status colours, whether it's light, and how
/// much stronger than Dark's its borders are, in thousandths.
struct Ink {
    text: u32,
    subtle: u32,
    muted: u32,
    link: u32,
    positive: u32,
    warning: u32,
    light: bool,
    borders: u32,
}

/// One of Discord's themes on Discord Obsidian's surfaces, mapped as Bonfire
/// maps Dark (see docs/themes.md): grounds from its background variables,
/// hovers, selections and lines as the same washes over them, at the
/// opacities read off Dark, borders as much stronger as the theme's are.
const fn discord(grounds: Grounds, ink: Ink) -> Theme {
    let Grounds {
        lowest,
        lower,
        low,
        high,
    } = grounds;
    let borders = ink.borders;
    Theme {
        dark: !ink.light,
        rail: rgb(lowest),
        sidebar: rgb(lowest),
        panel: rgb(low),
        chat: rgb(lower),
        stage: rgb(lower),
        raised: rgb(high),
        field: rgb(low),
        sunken: rgb(lowest),
        hover: rgb(wash(lowest, 122)),
        active: rgb(wash(lowest, 200)),
        row_hover: rgb(wash(lower, 80)),
        line: rgb(wash(lower, 240 * borders / 1000)),
        outline: rgb(wash(lowest, 122 * borders / 1000)),
        outline_chat: rgb(wash(lower, 120 * borders / 1000)),
        outline_field: rgb(wash(low, 60 * borders / 1000)),
        edge: rgb(wash(lower, 176 * borders / 1000)),
        text: rgb(ink.text),
        secondary: rgb(ink.subtle),
        muted: rgb(ink.muted),
        faint: rgb(mix_hex(lower, ink.muted, 661)),
        accent: rgb(0x5865f2),
        accent_hover: rgb(0x4452bb),
        on_accent: rgb(0xffffff),
        accent_soft: rgba(0x5865f226),
        danger: rgb(0xd22d39),
        danger_hover: rgb(0xa9232e),
        on_danger: rgb(0xffffff),
        live: rgb(ink.positive),
        loud: rgb(ink.warning),
        link: rgb(ink.link),
        button_default: rgb(wash(low, 151)),
        button_default_hover: rgb(wash(low, 185)),
        overlay: if ink.light {
            rgb(ink.text)
        } else {
            rgb(0xffffff)
        },
        shimmer: if ink.light {
            rgba(0xffffff99)
        } else {
            rgba(0xffffff10)
        },
        reply_spine: rgb(wash(lower, 360 * borders / 1000)),
        tooltip: rgb(high),
        avatar: rgb(wash(lower, 528)),
    }
}

/// Discord's Light theme's text, `.theme-light`.
const LIGHT_INK: Ink = Ink {
    text: 0x2e2e34,
    subtle: 0x595a63,
    muted: 0x6c6d76,
    link: 0x006dd4,
    positive: 0x24934e,
    warning: 0xaf7615,
    light: true,
    borders: 2300,
};

/// Discord's Dark theme's text, `.theme-dark.theme-darker`.
const DARK_INK: Ink = Ink {
    text: 0xefeff1,
    subtle: 0xabacb2,
    muted: 0x96979e,
    link: 0x4d96ee,
    positive: 0x23a55a,
    warning: 0xf0b232,
    light: false,
    borders: 1000,
};

/// Discord's Light theme.
const LIGHT: Theme = discord(
    Grounds {
        lowest: 0xf3f3f4,
        lower: 0xfbfbfb,
        low: 0xfbfbfb,
        high: 0xffffff,
    },
    LIGHT_INK,
);

/// Discord's Ash theme, `.theme-dark` alone: a lighter grey.
const ASH: Theme = discord(
    Grounds {
        lowest: 0x2c2d32,
        lower: 0x323339,
        low: 0x36373e,
        high: 0x393a41,
    },
    Ink {
        text: 0xf3f3f4,
        subtle: 0xc5c6ca,
        muted: 0xabacb2,
        link: 0x76aff6,
        ..DARK_INK
    },
);

/// Discord's Onyx theme, `.theme-midnight`: black grounds, its own text,
/// and borders two thirds stronger than Dark's to set them apart.
const ONYX: Theme = discord(
    Grounds {
        lowest: 0x000000,
        lower: 0x000000,
        low: 0x000000,
        high: 0x0a0a0c,
    },
    Ink {
        text: 0xd4d5d8,
        subtle: 0x96979e,
        muted: 0x81828a,
        link: 0x2781e7,
        borders: 1650,
        ..DARK_INK
    },
);

/// The average of a gradient's colours.
const fn mean(stops: &[u32]) -> u32 {
    let (mut red, mut green, mut blue, mut index) = (0, 0, 0, 0);
    while index < stops.len() {
        red += (stops[index] >> 16) & 0xff;
        green += (stops[index] >> 8) & 0xff;
        blue += stops[index] & 0xff;
        index += 1;
    }
    let count = if stops.is_empty() {
        1
    } else {
        stops.len() as u32
    };
    ((red / count) << 16) | ((green / count) << 8) | (blue / count)
}

/// One of Discord's colour themes, flat: Discord lays its grounds over the
/// gradient, black at 70% on a dark one and white at 80% on a light one, and
/// Discord Obsidian lays them over the gradient's first colour for the
/// server list and sidebar and its average for the chat. Raised surfaces
/// keep Dark's or Light's, as Discord's do.
const fn gradient(stops: &[u32], light: bool) -> Theme {
    let (over, amount, high, ink) = if light {
        (0xffffff, 800, 0xffffff, LIGHT_INK)
    } else {
        (0x000000, 700, 0x242429, DARK_INK)
    };
    let side = mix_hex(stops[0], over, amount);
    let middle = mix_hex(mean(stops), over, amount);
    discord(
        Grounds {
            lowest: side,
            lower: middle,
            low: middle,
            high,
        },
        ink,
    )
}

/// Which row of Appearance a built-in theme shows in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// Discord's Default Themes: Light, Ash, Dark and Onyx.
    Default,
    /// Discord's Color Themes, its gradients.
    Colour,
    /// Discord Obsidian's own and the community palettes.
    More,
}

/// A built-in theme's place: its group, and the two colours its swatch runs
/// between, top to bottom.
const fn plain(theme: &Theme) -> [Color; 2] {
    [theme.chat, theme.chat]
}

/// A theme Discord Obsidian comes with: the id a layout stores, the name
/// Appearance shows, the row it shows in and its swatch's colours.
pub struct BuiltIn {
    pub id: &'static str,
    pub name: &'static str,
    pub theme: Theme,
    pub group: Group,
    pub swatch: [Color; 2],
}

/// The themes Discord Obsidian comes with, as Appearance lists them:
/// Discord's default themes and colour themes, then Discord Obsidian's own
/// and the community palettes, the dark ones before the light ones. The
/// palettes credited in docs/themes.md are MIT licensed.
pub const BUILT_IN: [BuiltIn; 35] = [
    BuiltIn {
        id: "light",
        name: "Light",
        theme: LIGHT,
        group: Group::Default,
        swatch: plain(&LIGHT),
    },
    BuiltIn {
        id: "ash",
        name: "Ash",
        theme: ASH,
        group: Group::Default,
        swatch: plain(&ASH),
    },
    BuiltIn {
        id: "dark",
        name: "Dark",
        theme: BONFIRE,
        group: Group::Default,
        swatch: plain(&BONFIRE),
    },
    BuiltIn {
        id: "onyx",
        name: "Onyx",
        theme: ONYX,
        group: Group::Default,
        swatch: plain(&ONYX),
    },
    BuiltIn {
        id: "mint-apple",
        name: "Mint Apple",
        theme: gradient(&[0x56b69f, 0x63bc61, 0x9eca67], true),
        group: Group::Colour,
        swatch: [rgb(0x56b69f), rgb(0x9eca67)],
    },
    BuiltIn {
        id: "citrus-sherbert",
        name: "Citrus Sherbert",
        theme: gradient(&[0xf3b336, 0xee8558], true),
        group: Group::Colour,
        swatch: [rgb(0xf3b336), rgb(0xee8558)],
    },
    BuiltIn {
        id: "retro-raincloud",
        name: "Retro Raincloud",
        theme: gradient(&[0x3a7ca1, 0x7f7eb9, 0x7f7eb9, 0x3a7ca1], true),
        group: Group::Colour,
        swatch: [rgb(0x3a7ca1), rgb(0x7f7eb9)],
    },
    BuiltIn {
        id: "hanami",
        name: "Hanami",
        theme: gradient(&[0xefaab3, 0xefd696, 0xa6daa2], true),
        group: Group::Colour,
        swatch: [rgb(0xefaab3), rgb(0xa6daa2)],
    },
    BuiltIn {
        id: "sunrise",
        name: "Sunrise",
        theme: gradient(&[0x9f4175, 0xc49064, 0xa6953d], true),
        group: Group::Colour,
        swatch: [rgb(0x9f4175), rgb(0xa6953d)],
    },
    BuiltIn {
        id: "cotton-candy",
        name: "Cotton Candy",
        theme: gradient(&[0xf4abb8, 0xb1c2fc], true),
        group: Group::Colour,
        swatch: [rgb(0xf4abb8), rgb(0xb1c2fc)],
    },
    BuiltIn {
        id: "lofi-vibes",
        name: "Lofi Vibes",
        theme: gradient(&[0xa4c0f7, 0xa9e4e8, 0xb0e2b8, 0xcfdfa2], true),
        group: Group::Colour,
        swatch: [rgb(0xa4c0f7), rgb(0xcfdfa2)],
    },
    BuiltIn {
        id: "desert-khaki",
        name: "Desert Khaki",
        theme: gradient(&[0xe7dbd0, 0xdfd0b2, 0xe0d6a3], true),
        group: Group::Colour,
        swatch: [rgb(0xe7dbd0), rgb(0xe0d6a3)],
    },
    BuiltIn {
        id: "sunset",
        name: "Sunset",
        theme: gradient(&[0x48288c, 0xdb7f4b], false),
        group: Group::Colour,
        swatch: [rgb(0x48288c), rgb(0xdb7f4b)],
    },
    BuiltIn {
        id: "chroma-glow",
        name: "Chroma Glow",
        theme: gradient(&[0x0eb5bf, 0x4c0ce0, 0xa308a7, 0x9a53ff, 0x218be0], false),
        group: Group::Colour,
        swatch: [rgb(0x0eb5bf), rgb(0x218be0)],
    },
    BuiltIn {
        id: "forest",
        name: "Forest",
        theme: gradient(&[0x142215, 0x2d4d39, 0x454c32, 0x5a7c58, 0xa98e4b], false),
        group: Group::Colour,
        swatch: [rgb(0x142215), rgb(0xa98e4b)],
    },
    BuiltIn {
        id: "crimson-moon",
        name: "Crimson Moon",
        theme: gradient(&[0x950909, 0x000000], false),
        group: Group::Colour,
        swatch: [rgb(0x950909), rgb(0x000000)],
    },
    BuiltIn {
        id: "midnight-blurple",
        name: "Midnight Blurple",
        theme: gradient(&[0x5348ca, 0x140730], false),
        group: Group::Colour,
        swatch: [rgb(0x5348ca), rgb(0x140730)],
    },
    BuiltIn {
        id: "mars",
        name: "Mars",
        theme: gradient(&[0x895240, 0x8f4343], false),
        group: Group::Colour,
        swatch: [rgb(0x895240), rgb(0x8f4343)],
    },
    BuiltIn {
        id: "dusk",
        name: "Dusk",
        theme: gradient(&[0x665069, 0x91a3d1], false),
        group: Group::Colour,
        swatch: [rgb(0x665069), rgb(0x91a3d1)],
    },
    BuiltIn {
        id: "under-the-sea",
        name: "Under the Sea",
        theme: gradient(&[0x647962, 0x588575, 0x6a8482], false),
        group: Group::Colour,
        swatch: [rgb(0x647962), rgb(0x6a8482)],
    },
    BuiltIn {
        id: "retro-storm",
        name: "Retro Storm",
        theme: gradient(&[0x3a7ca1, 0x58579a, 0x58579a, 0x3a7ca1], false),
        group: Group::Colour,
        swatch: [rgb(0x3a7ca1), rgb(0x58579a)],
    },
    BuiltIn {
        id: "neon-nights",
        name: "Neon Nights",
        theme: gradient(&[0x01a89e, 0x7d60ba, 0xb43898], false),
        group: Group::Colour,
        swatch: [rgb(0x01a89e), rgb(0xb43898)],
    },
    BuiltIn {
        id: "strawberry-lemonade",
        name: "Strawberry Lemonade",
        theme: gradient(&[0xaf1a6c, 0xc26b20, 0xe7a525], false),
        group: Group::Colour,
        swatch: [rgb(0xaf1a6c), rgb(0xe7a525)],
    },
    BuiltIn {
        id: "aurora",
        name: "Aurora",
        theme: gradient(&[0x062053, 0x191fbb, 0x13929a, 0x218573, 0x051a81], false),
        group: Group::Colour,
        swatch: [rgb(0x062053), rgb(0x051a81)],
    },
    BuiltIn {
        id: "sepia",
        name: "Sepia",
        theme: gradient(&[0x857664, 0x5b4421], false),
        group: Group::Colour,
        swatch: [rgb(0x857664), rgb(0x5b4421)],
    },
    BuiltIn {
        id: "yap",
        name: "Obsidian",
        theme: YAP,
        group: Group::More,
        swatch: plain(&YAP),
    },
    BuiltIn {
        id: "midnight",
        name: "Midnight",
        theme: MIDNIGHT,
        group: Group::More,
        swatch: plain(&MIDNIGHT),
    },
    BuiltIn {
        id: "catppuccin-mocha",
        name: "Catppuccin Mocha",
        theme: CATPPUCCIN_MOCHA,
        group: Group::More,
        swatch: plain(&CATPPUCCIN_MOCHA),
    },
    BuiltIn {
        id: "dracula",
        name: "Dracula",
        theme: DRACULA,
        group: Group::More,
        swatch: plain(&DRACULA),
    },
    BuiltIn {
        id: "nord",
        name: "Nord",
        theme: NORD,
        group: Group::More,
        swatch: plain(&NORD),
    },
    BuiltIn {
        id: "gruvbox-dark",
        name: "Gruvbox Dark",
        theme: GRUVBOX_DARK,
        group: Group::More,
        swatch: plain(&GRUVBOX_DARK),
    },
    BuiltIn {
        id: "tokyo-night",
        name: "Tokyo Night",
        theme: TOKYO_NIGHT,
        group: Group::More,
        swatch: plain(&TOKYO_NIGHT),
    },
    BuiltIn {
        id: "catppuccin-latte",
        name: "Catppuccin Latte",
        theme: CATPPUCCIN_LATTE,
        group: Group::More,
        swatch: plain(&CATPPUCCIN_LATTE),
    },
    BuiltIn {
        id: "rose-pine-dawn",
        name: "Rosé Pine Dawn",
        theme: ROSE_PINE_DAWN,
        group: Group::More,
        swatch: plain(&ROSE_PINE_DAWN),
    },
    BuiltIn {
        id: "sakura",
        name: "Sakura",
        theme: SAKURA,
        group: Group::More,
        swatch: plain(&SAKURA),
    },
];

/// Where Discord's Dark theme is in [`BUILT_IN`]: what shows when a layout
/// names no theme Discord Obsidian has.
pub const DARK: usize = 2;

/// The id Discord's Dark theme had before Appearance named it so.
const BONFIRE_ID: &str = "bonfire";

pub fn find(id: &str) -> Option<&'static BuiltIn> {
    let id = if id == BONFIRE_ID { "dark" } else { id };
    BUILT_IN.iter().find(|theme| theme.id == id)
}

/// A theme Appearance offers, built in or the person's own, and its id.
pub fn lookup<'a>(id: &str, custom: &'a [Custom]) -> Option<(&'a str, Theme)> {
    find(id)
        .map(|built_in| (built_in.id, built_in.theme))
        .or_else(|| {
            custom
                .iter()
                .find(|custom| custom.id == id)
                .map(|custom| (custom.id.as_str(), custom.theme))
        })
}

/// The theme a layout chose, and its id: Yap when it chose none, or one
/// this version doesn't have or the person deleted.
pub fn chosen<'a>(id: Option<&str>, custom: &'a [Custom]) -> (&'a str, Theme) {
    id.and_then(|id| lookup(id, custom))
        .unwrap_or((BUILT_IN[DARK].id, BUILT_IN[DARK].theme))
}

/// A colour's name as `ui/theme.slint` and `layout.json` have it.
fn token(name: &str) -> String {
    name.replace('_', "-")
}

/// Where a colour named as `layout.json` has it is in [`Theme::colour`].
fn token_index(token: &str) -> Option<usize> {
    Theme::NAMES.iter().position(|name| {
        name.bytes()
            .map(|byte| if byte == b'_' { b'-' } else { byte })
            .eq(token.bytes())
    })
}

/// A colour written `#rgb`, `#rrggbb` or `#rrggbbaa`, as Slint writes them.
pub fn parse(text: &str) -> Option<Color> {
    let digits = text.trim().strip_prefix('#')?;
    if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        3 => {
            let nibble = |shift: u32| ((value >> shift) & 0xf) * 0x11;
            Some(rgb((nibble(8) << 16) | (nibble(4) << 8) | nibble(0)))
        }
        6 => Some(rgb(value)),
        8 => Some(rgba(value)),
        _ => None,
    }
}

/// A colour as `#rrggbb`, or `#rrggbbaa` when it isn't opaque.
pub fn hex(colour: Color) -> String {
    let rgb = colour.as_argb_encoded() & 0xff_ffff;
    match colour.alpha() {
        0xff => format!("#{rgb:06x}"),
        alpha => format!("#{rgb:06x}{alpha:02x}"),
    }
}

/// One of the person's own themes, as a layout keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct Custom {
    /// `custom:` and a number: no built-in id has a colon. See [`tidy`].
    pub id: String,
    pub name: String,
    /// The built-in theme it began as a copy of, whose colours stand in for
    /// any it doesn't name, such as one added since it was saved.
    pub base: &'static str,
    pub theme: Theme,
}

/// A custom theme in `layout.json`: each colour as text, by its name in
/// `ui/theme.slint`.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Stored {
    id: String,
    name: String,
    base: String,
    dark: Option<bool>,
    colours: std::collections::BTreeMap<String, String>,
}

impl From<Stored> for Custom {
    fn from(stored: Stored) -> Self {
        let base = find(&stored.base).unwrap_or(&BUILT_IN[DARK]);
        let mut theme = base.theme;
        theme.dark = stored.dark.unwrap_or(theme.dark);
        for (token, text) in &stored.colours {
            if let (Some(slot), Some(colour)) = (
                token_index(token).and_then(|index| theme.colour_mut(index)),
                parse(text),
            ) {
                *slot = colour;
            }
        }
        Self {
            id: stored.id,
            name: stored.name,
            base: base.id,
            theme,
        }
    }
}

impl From<&Custom> for Stored {
    fn from(custom: &Custom) -> Self {
        let colours = Theme::NAMES
            .iter()
            .enumerate()
            .filter_map(|(index, name)| Some((token(name), hex(custom.theme.colour(index)?))))
            .collect();
        Self {
            id: custom.id.clone(),
            name: custom.name.clone(),
            base: custom.base.to_owned(),
            dark: Some(custom.theme.dark),
            colours,
        }
    }
}

impl serde::Serialize for Custom {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Stored::from(self).serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Custom {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Stored::deserialize(deserializer).map(Self::from)
    }
}

/// The number in a custom theme's id, if it's one Discord Obsidian gives.
fn custom_number(id: &str) -> Option<u32> {
    id.strip_prefix("custom:")?.parse().ok()
}

/// An id for a new custom theme, after those there are.
pub fn new_id(custom: &[Custom]) -> String {
    let last = custom
        .iter()
        .filter_map(|custom| custom_number(&custom.id))
        .max()
        .unwrap_or(0);
    format!("custom:{}", last + 1)
}

/// Gives a new id to each custom theme from `layout.json` whose id Discord Obsidian
/// wouldn't have given, or that another has, so that ids stay apart from
/// the built-in ones and from each other.
pub fn tidy(custom: &mut [Custom]) {
    let mut seen = std::collections::HashSet::new();
    let unfit: Vec<usize> = (0..custom.len())
        .filter(|&index| {
            custom_number(&custom[index].id).is_none() || !seen.insert(custom[index].id.clone())
        })
        .collect();
    for index in unfit {
        custom[index].id = new_id(custom);
    }
}

/// A theme a new one can start as.
pub struct Start<'a> {
    pub id: &'a str,
    /// The built-in theme it is or began as.
    pub base: &'static str,
    pub name: &'a str,
    pub theme: Theme,
}

/// What a new theme can start as: every theme Appearance offers, in its
/// order.
pub fn starts(custom: &[Custom]) -> impl Iterator<Item = Start<'_>> {
    BUILT_IN
        .iter()
        .map(|built_in| Start {
            id: built_in.id,
            base: built_in.id,
            name: built_in.name,
            theme: built_in.theme,
        })
        .chain(custom.iter().map(|custom| Start {
            id: &custom.id,
            base: custom.base,
            name: &custom.name,
            theme: custom.theme,
        }))
}

thread_local! {
    /// The colour of a mention without one of its own, as `0xrrggbb`: the
    /// theme's link colour. Rows build their markup with it, outside the
    /// window, on the window's thread.
    static MENTION: Cell<u32> = const { Cell::new(0x7cc4ff) };
    /// The theme's text and muted colours, the same way, for a notice's
    /// name, links and time.
    static TEXT: Cell<u32> = const { Cell::new(0xf2f3f5) };
    static MUTED: Cell<u32> = const { Cell::new(0x949ba4) };
}

pub fn mention_color() -> u32 {
    MENTION.get()
}

pub fn text_color() -> u32 {
    TEXT.get()
}

pub fn muted_color() -> u32 {
    MUTED.get()
}

impl Theme {
    /// Shows the window in these colours. Returns whether colours that rows
    /// carry in their markup change, as rows built before have the old ones.
    pub fn apply(&self, window: &ObsidianWindow) -> bool {
        self.set(&window.global::<obsidian_ui::Theme>());
        let rgb = |colour: Color| colour.as_argb_encoded() & 0xff_ffff;
        // Bonfire draws a mention's text as Discord's dark theme does, a light
        // blurple; the other themes use their link colour.
        let mention_colour = if self.dark && rgb(self.link) == rgb(BONFIRE.link) {
            0xc9cdfb
        } else {
            rgb(self.link)
        };
        let mention = MENTION.replace(mention_colour) != mention_colour;
        let text = TEXT.replace(rgb(self.text)) != rgb(self.text);
        let muted = MUTED.replace(rgb(self.muted)) != rgb(self.muted);
        mention || text || muted
    }
}

/// Appearance's cards in one of its rows: each theme's name and the colours
/// its preview and swatch draw with. The person's own come after the More
/// row's built-in ones.
pub fn cards(custom: &[Custom], group: Group) -> ModelRc<obsidian_ui::ThemeCard> {
    let card = |id: &str, name: &str, custom: bool, theme: &Theme, swatch: [Color; 2]| {
        obsidian_ui::ThemeCard {
            id: id.into(),
            name: name.into(),
            custom,
            rail: theme.rail,
            sidebar: theme.sidebar,
            chat: theme.chat,
            raised: theme.raised,
            accent: theme.accent,
            text: theme.text,
            muted: theme.muted,
            swatch_top: swatch[0],
            swatch_bottom: swatch[1],
        }
    };
    let own = custom
        .iter()
        .filter(|_| group == Group::More)
        .map(|custom| {
            card(
                &custom.id,
                &custom.name,
                true,
                &custom.theme,
                plain(&custom.theme),
            )
        });
    let cards: Vec<_> = BUILT_IN
        .iter()
        .filter(|built_in| built_in.group == group)
        .map(|built_in| {
            card(
                built_in.id,
                built_in.name,
                false,
                &built_in.theme,
                built_in.swatch,
            )
        })
        .chain(own)
        .collect();
    ModelRc::new(VecModel::from(cards))
}

/// Shows every row of Appearance's themes.
pub fn show_cards(window: &ObsidianWindow, custom: &[Custom]) {
    window.set_default_themes(cards(custom, Group::Default));
    window.set_colour_themes(cards(custom, Group::Colour));
    window.set_themes(cards(custom, Group::More));
}

/// Whether the system shows its apps dark, for "Sync with computer": on
/// Windows, Settings' app mode; elsewhere, what the window was told.
pub fn system_dark(window: &ObsidianWindow) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::{
            HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW,
        };
        let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (key, value) = (
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            wide("AppsUseLightTheme"),
        );
        let (mut light, mut size) = (1u32, 4u32);
        // SAFETY: the strings end in nul, and the value is a DWORD of `size` bytes.
        let read = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&raw mut light).cast(),
                &raw mut size,
            )
        };
        let _ = window;
        read != 0 || light == 0
    }
    #[cfg(not(windows))]
    {
        use slint::winit_030::{WinitWindowAccessor, winit::window::Theme as WinitTheme};
        window
            .window()
            .with_winit_window(|winit| winit.theme())
            .flatten()
            .is_none_or(|theme| theme == WinitTheme::Dark)
    }
}

/// The theme "Sync with computer" shows: Discord's Dark or Light.
pub fn synced(dark: bool) -> &'static str {
    if dark { "dark" } else { "light" }
}

/// Appearance's theme editor while it's open: the theme as edited so far,
/// and a row for each colour, the model it lends the window until it closes.
pub struct Editor {
    /// The custom theme being edited, by id; `None` for a new one.
    pub editing: Option<String>,
    pub base: &'static str,
    pub theme: Theme,
    rows: Rc<VecModel<obsidian_ui::ThemeColour>>,
}

impl Editor {
    pub fn new(editing: Option<String>, base: &'static str, theme: Theme) -> Self {
        let editor = Self {
            editing,
            base,
            theme,
            rows: Rc::default(),
        };
        editor.fill();
        editor
    }

    /// Writes every row again, which also gives each field its text anew.
    fn fill(&self) {
        let rows: Vec<_> = Theme::NAMES
            .iter()
            .enumerate()
            .filter_map(|(index, name)| {
                let colour = self.theme.colour(index)?;
                Some(obsidian_ui::ThemeColour {
                    name: token(name).into(),
                    label: label(name).into(),
                    colour,
                    hex: hex(colour).into(),
                    wrong: false,
                })
            })
            .collect();
        self.rows.set_vec(rows);
    }

    pub fn rows(&self) -> ModelRc<obsidian_ui::ThemeColour> {
        ModelRc::from(self.rows.clone())
    }

    /// Starts again from another theme's colours.
    pub fn restart(&mut self, base: &'static str, theme: Theme) {
        self.base = base;
        self.theme = theme;
        self.fill();
    }

    /// Takes a colour's text as typed: the theme takes the colour if it is
    /// one, and the row says so if not. Returns whether the theme changed.
    pub fn edit(&mut self, index: usize, text: &str) -> bool {
        let Some(mut row) = self.rows.row_data(index) else {
            return false;
        };
        let colour = parse(text);
        row.hex = text.into();
        row.wrong = colour.is_none();
        row.colour = colour.unwrap_or(row.colour);
        self.rows.set_row_data(index, row);
        match (colour, self.theme.colour_mut(index)) {
            (Some(colour), Some(slot)) if *slot != colour => {
                *slot = colour;
                true
            }
            _ => false,
        }
    }

    /// Takes a colour from the picker. Returns whether the theme changed.
    pub fn pick(&mut self, index: usize, colour: Color) -> bool {
        let Some(mut row) = self.rows.row_data(index) else {
            return false;
        };
        row.colour = colour;
        row.hex = hex(colour).into();
        row.wrong = false;
        self.rows.set_row_data(index, row);
        match self.theme.colour_mut(index) {
            Some(slot) if *slot != colour => {
                *slot = colour;
                true
            }
            _ => false,
        }
    }

    /// Makes every colour again from a background and an accent, as the
    /// theme maker's quick colours do.
    pub fn generate(&mut self, background: Color, accent: Color) {
        self.theme = generate(background, accent);
        self.fill();
    }

    /// Whether every field holds a colour.
    pub fn valid(&self) -> bool {
        !self.rows.iter().any(|row| row.wrong)
    }
}

/// What a colour paints, in words for the theme maker rather than its name
/// in `ui/theme.slint`.
pub fn label(name: &str) -> &'static str {
    match name {
        "rail" => "Server list",
        "sidebar" => "Channel list",
        "panel" => "Account panel",
        "chat" => "Chat background",
        "stage" => "Call stage",
        "raised" => "Cards and popups",
        "field" => "Message box",
        "sunken" => "Deep wells",
        "hover" => "Hover",
        "active" => "Selected",
        "row_hover" => "List hover",
        "line" => "Lines",
        "edge" => "Edges",
        "outline" => "Sidebar outline",
        "outline_chat" => "Chat outline",
        "outline_field" => "Message box outline",
        "text" => "Text",
        "secondary" => "Secondary text",
        "muted" => "Muted text",
        "faint" => "Faint text",
        "accent" => "Accent",
        "accent_hover" => "Accent on hover",
        "on_accent" => "Text on accent",
        "accent_soft" => "Accent glow",
        "danger" => "Danger",
        "danger_hover" => "Danger on hover",
        "on_danger" => "Text on danger",
        "live" => "Online and live",
        "loud" => "Loud microphone",
        "link" => "Links",
        "button_default" => "Buttons",
        "button_default_hover" => "Buttons on hover",
        "overlay" => "Hover wash",
        "shimmer" => "Loading shimmer",
        "reply_spine" => "Reply line",
        "tooltip" => "Tooltips",
        "avatar" => "Avatar fallback",
        _ => "Colour",
    }
}

/// `a` moved `amount` of the way to `b`, alpha included.
fn mix(a: Color, b: Color, amount: f32) -> Color {
    let step = |from: u8, to: u8| (from as f32 + (to as f32 - from as f32) * amount).round() as u8;
    Color::from_argb_u8(
        step(a.alpha(), b.alpha()),
        step(a.red(), b.red()),
        step(a.green(), b.green()),
        step(a.blue(), b.blue()),
    )
}

/// Relative luminance, 0 for black to 1 for white.
fn luminance(colour: Color) -> f32 {
    let channel = |value: u8| {
        let value = value as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(colour.red())
        + 0.7152 * channel(colour.green())
        + 0.0722 * channel(colour.blue())
}

/// A whole theme from two colours the theme maker asks for: the chat's
/// background and the accent. The rail and sidebar step darker than the chat
/// (lighter surfaces step up on a dark one), text reads against the ground,
/// and the theme is dark when the ground is.
pub fn generate(background: Color, accent: Color) -> Theme {
    let background = background.with_alpha(1.0);
    let accent = accent.with_alpha(1.0);
    let white = rgb(0xffffff);
    let black = rgb(0x000000);
    let dark = luminance(background) < 0.35;
    let on_accent = if luminance(accent) > 0.45 {
        mix(black, accent, 0.12)
    } else {
        white
    };
    let accent_soft = accent.with_alpha(0.15);
    if dark {
        let sidebar = mix(background, black, 0.22);
        Theme {
            dark: true,
            rail: mix(background, black, 0.4),
            sidebar,
            panel: mix(background, black, 0.32),
            chat: background,
            stage: background,
            raised: mix(background, white, 0.06),
            field: mix(background, white, 0.08),
            sunken: mix(background, black, 0.5),
            hover: mix(background, white, 0.04),
            active: mix(background, white, 0.09),
            row_hover: mix(sidebar, white, 0.04),
            line: mix(background, white, 0.09),
            edge: mix(background, black, 0.6),
            outline: mix(sidebar, white, 0.08),
            outline_chat: mix(background, white, 0.08),
            outline_field: mix(background, white, 0.09),
            text: mix(white, background, 0.06),
            secondary: mix(white, background, 0.3),
            muted: mix(white, background, 0.45),
            faint: mix(white, background, 0.64),
            accent,
            accent_hover: mix(accent, white, 0.18),
            on_accent,
            accent_soft,
            danger: YAP.danger,
            danger_hover: YAP.danger_hover,
            on_danger: YAP.on_danger,
            live: YAP.live,
            loud: YAP.loud,
            link: YAP.link,
            button_default: mix(background, white, 0.12),
            button_default_hover: mix(background, white, 0.17),
            overlay: white,
            shimmer: rgba(0xffffff10),
            reply_spine: mix(background, white, 0.2),
            tooltip: mix(background, black, 0.62),
            avatar: YAP.avatar,
        }
    } else {
        let sidebar = mix(background, black, 0.04);
        Theme {
            dark: false,
            rail: mix(background, black, 0.09),
            sidebar,
            panel: mix(background, black, 0.065),
            chat: background,
            stage: background,
            raised: mix(background, white, 0.6),
            field: mix(background, black, 0.05),
            sunken: mix(background, black, 0.08),
            hover: mix(background, black, 0.05),
            active: mix(background, black, 0.09),
            row_hover: mix(sidebar, black, 0.04),
            line: mix(background, black, 0.12),
            edge: mix(background, black, 0.18),
            outline: mix(sidebar, black, 0.1),
            outline_chat: mix(background, black, 0.1),
            outline_field: mix(background, black, 0.12),
            text: mix(black, background, 0.1),
            secondary: mix(black, background, 0.34),
            muted: mix(black, background, 0.48),
            faint: mix(black, background, 0.62),
            accent,
            accent_hover: mix(accent, black, 0.12),
            on_accent,
            accent_soft,
            danger: rgb(0xd83a42),
            danger_hover: rgb(0xc4313a),
            on_danger: white,
            live: rgb(0x1f9d55),
            loud: rgb(0xe8730c),
            link: rgb(0x0b6bcb),
            button_default: mix(background, black, 0.08),
            button_default_hover: mix(background, black, 0.12),
            overlay: black,
            shimmer: rgba(0xffffff66),
            reply_spine: mix(background, black, 0.25),
            tooltip: mix(background, white, 0.75),
            avatar: YAP.avatar,
        }
    }
}

#[cfg(test)]
mod generate_tests {
    use super::*;

    #[test]
    fn a_dark_ground_makes_a_dark_theme_with_light_text() {
        let theme = generate(rgb(0x1b1026), rgb(0xeb459e));
        assert!(theme.dark);
        assert!(luminance(theme.text) > 0.7);
        assert!(luminance(theme.rail) < luminance(theme.chat));
        assert_eq!(theme.accent, rgb(0xeb459e));
    }

    #[test]
    fn a_light_ground_makes_a_light_theme_with_dark_text() {
        let theme = generate(rgb(0xf5f5f7), rgb(0x5865f2));
        assert!(!theme.dark);
        assert!(luminance(theme.text) < 0.1);
        assert_eq!(theme.on_accent, rgb(0xffffff));
    }

    #[test]
    fn a_pale_accent_takes_dark_text() {
        let theme = generate(rgb(0x111111), rgb(0xffe066));
        assert!(luminance(theme.on_accent) < 0.2);
    }

    #[test]
    fn discords_themes_come_from_its_colours() {
        // Dark keeps Bonfire's mapping, and Bonfire's id finds it.
        assert_eq!(find("bonfire").map(|built_in| built_in.id), Some("dark"));
        assert_eq!(BUILT_IN[DARK].id, "dark");
        assert_eq!(BUILT_IN[DARK].theme, BONFIRE);
        // Onyx is black, Light light, Ash between.
        assert_eq!(ONYX.chat, rgb(0x000000));
        assert_eq!(ONYX.rail, rgb(0x000000));
        const { assert!(ONYX.dark && ASH.dark && !LIGHT.dark) };
        assert_eq!(LIGHT.rail, rgb(0xf3f3f4));
        assert_eq!(ASH.chat, rgb(0x323339));
        // The washes read off Dark give Dark's own colours back.
        let dark = discord(
            Grounds {
                lowest: 0x121214,
                lower: 0x1a1a1e,
                low: 0x202024,
                high: 0x242429,
            },
            DARK_INK,
        );
        assert_eq!((dark.hover, dark.active), (BONFIRE.hover, BONFIRE.active));
        // A colour theme lays black over its gradient, and keeps blurple.
        let sunset = find("sunset").unwrap();
        assert_eq!(sunset.group, Group::Colour);
        assert!(sunset.theme.dark);
        assert_eq!(sunset.theme.rail, rgb(mix_hex(0x48288c, 0, 700)));
        assert_eq!(sunset.theme.accent, rgb(0x5865f2));
        assert!(!find("mint-apple").unwrap().theme.dark);
        // Sync with computer shows Dark or Light.
        assert_eq!((synced(true), synced(false)), ("dark", "light"));
        assert_eq!(
            BUILT_IN
                .iter()
                .filter(|built_in| built_in.group == Group::Default)
                .count(),
            4
        );
    }

    #[test]
    fn every_colour_has_words() {
        for name in Theme::NAMES {
            assert_ne!(label(name), "Colour", "{name} has no label");
        }
    }
}
