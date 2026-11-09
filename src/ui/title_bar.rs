//! The title bar Discord Obsidian draws in place of the desktop's on Linux: whether it
//! does, and moving, sizing, minimizing, maximizing and closing the window
//! from it through winit. Discord Obsidian also uses these actions for its Windows bar.
//!
//! A desktop that gives its windows title bars, as KDE and GNOME do, draws
//! one on Discord Obsidian's too unless Discord Obsidian draws its own. A tiling compositor such
//! as Hyprland or sway draws none, and there Discord Obsidian's would be in the way.
//! The desktop doesn't say which it does: KWin and Hyprland both answer
//! xdg-decoration with server-side. So `XDG_CURRENT_DESKTOP` decides, until
//! the person chooses in Appearance.
//!
//! Each window is made with the frame or without, rather than given it or
//! losing it once made. Where the desktop draws no frame, as GNOME and
//! Weston, winit draws one, and putting it on a maximized window or taking it
//! off makes the window larger or smaller than maximized, for which Weston
//! ends the connection.

use slint::winit_030::winit::window::{ResizeDirection, WindowAttributes};

thread_local! {
    /// Windows are made without the desktop's frame, for Discord Obsidian's title bar.
    static OWN: Cell<bool> = const { Cell::new(false) };
}

use super::*;

/// Desktops whose windows have title bars, as `XDG_CURRENT_DESKTOP` names them.
const FRAMED: [&str; 14] = [
    "KDE",
    "GNOME",
    "Unity",
    "Cinnamon",
    "X-Cinnamon",
    "XFCE",
    "MATE",
    "LXQt",
    "LXDE",
    "Budgie",
    "Pantheon",
    "Deepin",
    "COSMIC",
    "Enlightenment",
];

/// The edges and corners ResizeEdges numbers, in its order.
const DIRECTIONS: [ResizeDirection; 8] = [
    ResizeDirection::East,
    ResizeDirection::North,
    ResizeDirection::NorthEast,
    ResizeDirection::NorthWest,
    ResizeDirection::South,
    ResizeDirection::SouthEast,
    ResizeDirection::SouthWest,
    ResizeDirection::West,
];

/// Discord Obsidian also offers the existing window actions on Windows. Upstream
/// presentation and other platforms keep their inherited frame policy.
fn offered() -> bool {
    cfg!(target_os = "linux") || (cfg!(windows) && obsidian::enabled())
}

/// Whether the desktop draws the title bar unless the person chose: on a
/// desktop not known to give windows one, it's the desktop's, which may be
/// none at all.
pub(super) fn system_by_default() -> bool {
    if cfg!(windows) && obsidian::enabled() && !cfg!(test) {
        false
    } else {
        cfg!(test) || !std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktops| framed(&desktops))
    }
}

/// Whether any of the desktops in a colon-separated `XDG_CURRENT_DESKTOP`,
/// such as "ubuntu:GNOME", gives its windows title bars.
fn framed(desktops: &str) -> bool {
    desktops.split(':').any(|desktop| {
        FRAMED
            .iter()
            .any(|framed| desktop.eq_ignore_ascii_case(framed))
    })
}

/// Makes the windows from now on with Discord Obsidian's title bar or the desktop's,
/// as the layout has it, and returns whether Discord Obsidian's.
pub(super) fn choose(system: Option<bool>) -> bool {
    let own = offered() && !system.unwrap_or_else(system_by_default);
    OWN.set(own);
    own
}

/// Each window as Slint makes it: with the desktop's frame unless Discord Obsidian
/// draws its own title bar.
pub(super) fn attributes(attributes: WindowAttributes) -> WindowAttributes {
    attributes.with_decorations(!OWN.get())
}

/// Shows the title bar as the layout has it: the desktop's, or Discord Obsidian's. A
/// window made since the choice changed shows it from before it's shown.
pub(super) fn show(window: &ObsidianWindow, system: Option<bool>) {
    let own = choose(system);
    window.set_title_bar_choice(offered());
    window.set_system_title_bar(!own);
    window.set_own_title_bar(own);
}

/// What the title bar and the window's edges do, and the setting.
pub(super) fn bind(window: &ObsidianWindow, adapter: &Rc<RefCell<Adapter>>) {
    let weak = window.as_weak();
    window.on_move_window(move || {
        winit(&weak, |winit| {
            if let Err(error) = winit.drag_window() {
                tracing::warn!(%error, "could not move the window");
            }
        });
    });
    let weak = window.as_weak();
    window.on_resize_window(move |direction| {
        let Some(&direction) = usize::try_from(direction)
            .ok()
            .and_then(|index| DIRECTIONS.get(index))
        else {
            return;
        };
        winit(&weak, |winit| {
            if let Err(error) = winit.drag_resize_window(direction) {
                tracing::warn!(%error, "could not resize the window");
            }
        });
    });
    let weak = window.as_weak();
    // Straight to winit: Slint keeps whether the window is minimized from
    // winit, which on Wayland never says, so Slint's own may be stale.
    window.on_minimize_window(move || winit(&weak, |winit| winit.set_minimized(true)));
    let weak = window.as_weak();
    window.on_full_screen_changed(move |on| full_screen_changed(&weak, on));
    let weak = window.as_weak();
    window.on_maximize_window(move || {
        if let Some(window) = weak.upgrade() {
            let native = window.window();
            native.set_maximized(!native.is_maximized());
        }
    });
    let weak = window.as_weak();
    window.on_close_window(move || {
        // As the desktop's close button: to the tray, or quit; see Shell::close.
        if let Some(window) = weak.upgrade() {
            window
                .window()
                .dispatch_event(slint::platform::WindowEvent::CloseRequested);
        }
    });
    let adapter = Rc::downgrade(adapter);
    let weak = window.as_weak();
    window.on_title_bar_toggled(move || {
        let (Some(adapter), Some(window)) = (adapter.upgrade(), weak.upgrade()) else {
            return;
        };
        let mut adapter = adapter.borrow_mut();
        let system = window.get_system_title_bar();
        adapter.layout.system_title_bar = Some(system);
        adapter.layout.write(&adapter.root);
        drop(adapter);
        // A maximized window is made again with the frame or without; see above.
        if window.window().is_maximized() {
            choose(Some(system));
            // Not from inside the window's own callback, which dropping it ends.
            slint::Timer::single_shot(Duration::ZERO, shell::remake_window);
        } else {
            show(&window, Some(system));
        }
    });
}

/// Windows keeps the taskbar over a borderless fullscreen window that still
/// has a resizable frame, and draws its border around it; winit leaves the
/// frame, and a maximized window's place clear of the taskbar, on a window
/// without decorations. So once winit has gone full
/// screen, the window loses its frame and its border, and gets them back
/// on leaving.
#[cfg(windows)]
mod full_screen {
    use std::cell::Cell;
    use windows_sys::Win32::{
        Foundation::{HWND, RECT},
        Graphics::{
            Dwm::{DWMWA_BORDER_COLOR, DwmSetWindowAttribute},
            Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
        },
        UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GWL_STYLE, GetWindowLongPtrW, HWND_TOP, SWP_FRAMECHANGED, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
            SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, WS_BORDER, WS_CAPTION,
            WS_DLGFRAME, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_WINDOWEDGE, WS_MAXIMIZE,
            WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
        },
    };

    /// DWM's colours for "no border" and "the usual border".
    const COLOR_NONE: u32 = 0xFFFF_FFFE;
    const COLOR_DEFAULT: u32 = 0xFFFF_FFFF;

    thread_local! {
        /// The styles the window had before losing its frame.
        static SAVED: Cell<Option<(isize, isize)>> = const { Cell::new(None) };
    }

    pub(super) fn hwnd(window: &slint::winit_030::winit::window::Window) -> Option<HWND> {
        use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
            _ => None,
        }
    }

    fn border(hwnd: HWND, color: u32) {
        // SAFETY: a u32 the size given, for a window this process owns.
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR as u32,
                (&raw const color).cast(),
                size_of::<u32>() as u32,
            );
        }
    }

    /// Takes the frame and border off, and covers the whole monitor.
    pub(super) fn enter(hwnd: HWND) {
        // SAFETY: Win32 calls on this process's own window, with structures
        // sized as they ask.
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let extended = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SAVED.set(Some((style, extended)));
            // WS_MAXIMIZE too: winit keeps a maximized window without
            // decorations off the taskbar, so a window maximized before going
            // full screen drew 48 px short, with the taskbar showing through.
            let frame = (WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX
                | WS_BORDER | WS_DLGFRAME | WS_MAXIMIZE) as isize;
            let edges = (WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME) as isize;
            SetWindowLongPtrW(hwnd, GWL_STYLE, (style & !frame) | WS_POPUP as isize);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, extended & !edges);
            border(hwnd, COLOR_NONE);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..std::mem::zeroed()
            };
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            if GetMonitorInfoW(monitor, &mut info) != 0 {
                let RECT { left, top, right, bottom } = info.rcMonitor;
                SetWindowPos(
                    hwnd,
                    HWND_TOP,
                    left,
                    top,
                    right - left,
                    bottom - top,
                    SWP_FRAMECHANGED | SWP_SHOWWINDOW | SWP_NOOWNERZORDER,
                );
            }
            SetForegroundWindow(hwnd);
        }
    }

    /// Puts the frame and border back, before winit leaves full screen and
    /// returns the window to its place.
    pub(super) fn leave(hwnd: HWND) {
        let Some((style, extended)) = SAVED.take() else {
            return;
        };
        // SAFETY: as in `enter`.
        unsafe {
            SetWindowLongPtrW(hwnd, GWL_STYLE, style);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, extended);
            border(hwnd, COLOR_DEFAULT);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

/// The window went full screen, as the stage asks, or left it.
pub(super) fn full_screen_changed(weak: &slint::Weak<ObsidianWindow>, on: bool) {
    #[cfg(windows)]
    {
        if on {
            // After winit has applied full screen, which happens as Slint
            // next syncs the window's properties.
            let weak = weak.clone();
            slint::Timer::single_shot(Duration::from_millis(80), move || {
                winit(&weak, |winit| {
                    if winit.fullscreen().is_some()
                        && let Some(hwnd) = full_screen::hwnd(winit)
                    {
                        full_screen::enter(hwnd);
                    }
                });
            });
        } else {
            winit(weak, |winit| {
                if let Some(hwnd) = full_screen::hwnd(winit) {
                    full_screen::leave(hwnd);
                }
            });
        }
    }
    #[cfg(not(windows))]
    let _ = (weak, on);
}

/// Acts on the window's winit window, once it has one.
fn winit(
    weak: &slint::Weak<ObsidianWindow>,
    act: impl FnOnce(&slint::winit_030::winit::window::Window),
) {
    if let Some(window) = weak.upgrade() {
        window.window().with_winit_window(act);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framed_desktops_by_name() {
        assert!(framed("KDE"));
        assert!(framed("ubuntu:GNOME"));
        assert!(framed("X-Cinnamon"));
        assert!(!framed("Hyprland"));
        assert!(!framed("sway"));
        assert!(!framed("niri"));
        assert!(!framed(""));
    }
}
