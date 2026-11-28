//! Shortcuts that toggle mute and deafen from any app, a game included: a
//! low-level keyboard hook on Windows, which only runs while one is set and
//! leaves keys to the window while Discord Obsidian is in front. The window
//! hears them itself whenever it has the focus, on every system. Each
//! shortcut is a key with the Ctrl, Shift and Alt it needs held; the key
//! passes on to the app in front as it came.
use serde::{Deserialize, Serialize};
use slint::winit_030::winit::{
    event::{ElementState, KeyEvent},
    keyboard::{Key, NamedKey},
};

use crate::push_to_talk::Keybind;

/// A recorded shortcut.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcut {
    pub key: Keybind,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ctrl: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shift: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub alt: bool,
}

/// Held modifiers, as bits.
const CTRL: u32 = 1;
const SHIFT: u32 = 2;
const ALT: u32 = 4;

impl Shortcut {
    /// The shortcut a key press makes with the modifiers held, unless it's a
    /// modifier itself, which waits for the key it goes with.
    pub fn recorded(event: &KeyEvent, ctrl: bool, shift: bool, alt: bool) -> Option<Self> {
        if event.state != ElementState::Pressed
            || matches!(
                event.logical_key,
                Key::Named(
                    NamedKey::Control
                        | NamedKey::Shift
                        | NamedKey::Alt
                        | NamedKey::AltGraph
                        | NamedKey::Super
                        | NamedKey::Meta
                )
            )
        {
            return None;
        }
        Some(Self {
            key: Keybind::recorded(event)?,
            ctrl,
            shift,
            alt,
        })
    }

    /// As settings show it, such as "Ctrl+Shift+M".
    pub fn name(&self) -> String {
        let mut name = String::new();
        for (held, part) in [
            (self.ctrl, "Ctrl+"),
            (self.shift, "Shift+"),
            (self.alt, "Alt+"),
        ] {
            if held {
                name.push_str(part);
            }
        }
        name + self.key.name.as_str()
    }

    fn modifiers(&self) -> u32 {
        (u32::from(self.ctrl) * CTRL)
            | (u32::from(self.shift) * SHIFT)
            | (u32::from(self.alt) * ALT)
    }

    /// Whether a key press in the window is this shortcut.
    pub fn matches(&self, event: &KeyEvent, ctrl: bool, shift: bool, alt: bool) -> bool {
        use slint::winit_030::winit::platform::scancode::PhysicalKeyExtScancode;
        event.physical_key.to_scancode() == Some(self.key.scancode)
            && (ctrl, shift, alt) == (self.ctrl, self.shift, self.alt)
    }
}

/// What a shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Mute,
    Deafen,
}

/// Watches the shortcuts in every app, until dropped.
pub struct Watcher {
    shortcuts: [Option<Shortcut>; 2],
    #[cfg(windows)]
    _hook: Option<windows::Hook>,
}

impl Watcher {
    /// Starts watching the shortcuts set, if any are; `pressed` hears each
    /// one pressed, on the hook's thread.
    pub fn start(
        mute: Option<Shortcut>,
        deafen: Option<Shortcut>,
        pressed: impl Fn(Toggle) + Send + Sync + 'static,
    ) -> Option<Self> {
        if mute.is_none() && deafen.is_none() {
            return None;
        }
        let shortcuts = [mute, deafen];
        #[cfg(windows)]
        let hook = windows::Hook::start(&shortcuts, Box::new(pressed));
        #[cfg(not(windows))]
        let _ = pressed;
        Some(Self {
            shortcuts,
            #[cfg(windows)]
            _hook: hook,
        })
    }

    pub fn shortcuts(&self) -> &[Option<Shortcut>; 2] {
        &self.shortcuts
    }

    /// What a key press in the focused window toggles; the hook leaves keys
    /// pressed there to the window.
    pub fn focused(&self, event: &KeyEvent, ctrl: bool, shift: bool, alt: bool) -> Option<Toggle> {
        if event.repeat || event.state != ElementState::Pressed {
            return None;
        }
        [Toggle::Mute, Toggle::Deafen]
            .into_iter()
            .zip(&self.shortcuts)
            .find(|(_, shortcut)| {
                shortcut
                    .as_ref()
                    .is_some_and(|shortcut| shortcut.matches(event, ctrl, shift, alt))
            })
            .map(|(toggle, _)| toggle)
    }

    /// Where the shortcuts work, for settings.
    pub fn note() -> &'static str {
        if cfg!(windows) {
            "Works in every app, games included, and in this one."
        } else {
            "Works while the Discord window is in front."
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::{
        ptr,
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicU32, Ordering},
            mpsc,
        },
    };

    use windows_sys::Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        System::{
            LibraryLoader::GetModuleHandleW,
            Threading::{GetCurrentProcessId, GetCurrentThreadId},
        },
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT},
            WindowsAndMessaging::{
                CallNextHookEx, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
                HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, MSG, PostThreadMessageW,
                SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
                WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
            },
        },
    };

    use super::{ALT, CTRL, SHIFT, Shortcut, Toggle};

    const NONE: u32 = u32::MAX;
    /// Each shortcut's scan code and modifiers, mute's then deafen's, and
    /// whether its key is down, so holding it toggles once.
    static KEYS: [AtomicU32; 2] = [AtomicU32::new(NONE), AtomicU32::new(NONE)];
    static MODIFIERS: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];
    static DOWN: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];
    type Pressed = Box<dyn Fn(Toggle) + Send + Sync>;
    static PRESSED: Mutex<Option<Pressed>> = Mutex::new(None);

    /// A low-level keyboard hook on a thread of its own. Removed when dropped.
    pub struct Hook {
        thread: u32,
    }

    impl Hook {
        pub fn start(shortcuts: &[Option<Shortcut>; 2], pressed: Pressed) -> Option<Self> {
            for (index, shortcut) in shortcuts.iter().enumerate() {
                let (key, modifiers) = shortcut.as_ref().map_or((NONE, 0), |shortcut| {
                    (shortcut.key.scancode, shortcut.modifiers())
                });
                KEYS[index].store(key, Ordering::Relaxed);
                MODIFIERS[index].store(modifiers, Ordering::Relaxed);
                DOWN[index].store(false, Ordering::Relaxed);
            }
            *PRESSED
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(pressed);
            let (started, thread) = mpsc::sync_channel(1);
            let spawned = std::thread::Builder::new()
                .name("obsidian-shortcuts".into())
                .spawn(move || {
                    // SAFETY: the hook procedure reads only the event Windows
                    // hands it; the message loop runs on this thread, which
                    // the hook needs, until WM_QUIT.
                    unsafe {
                        let hook = SetWindowsHookExW(
                            WH_KEYBOARD_LL,
                            Some(proc),
                            GetModuleHandleW(ptr::null()),
                            0,
                        );
                        if hook.is_null() {
                            let _ = started.send(None);
                            return;
                        }
                        let _ = started.send(Some(GetCurrentThreadId()));
                        let mut message: MSG = std::mem::zeroed();
                        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {}
                        UnhookWindowsHookEx(hook);
                    }
                });
            let thread = spawned.ok().and_then(|_| thread.recv().ok().flatten());
            if thread.is_none() {
                tracing::warn!("Windows didn't let the mute and deafen shortcuts watch keys");
            }
            thread.map(|thread| Self { thread })
        }
    }

    impl Drop for Hook {
        fn drop(&mut self) {
            for key in &KEYS {
                key.store(NONE, Ordering::Relaxed);
            }
            *PRESSED
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
            // SAFETY: posting to a thread id is harmless if it has ended.
            unsafe {
                PostThreadMessageW(self.thread, WM_QUIT, 0, 0);
            }
        }
    }

    fn held(key: u16) -> bool {
        // SAFETY: reads the key's state; no pointers.
        unsafe { GetAsyncKeyState(i32::from(key)) as u16 & 0x8000 != 0 }
    }

    /// Whether one of Discord Obsidian's own windows is in front, which hears
    /// the shortcuts through its key events.
    fn ours_in_front() -> bool {
        let mut process = 0;
        // SAFETY: GetForegroundWindow may return null, which
        // GetWindowThreadProcessId answers with 0; `process` outlives it.
        unsafe {
            GetWindowThreadProcessId(GetForegroundWindow(), &mut process);
            process == GetCurrentProcessId()
        }
    }

    /// Toggles on a shortcut's key going down with its modifiers held, unless
    /// Discord Obsidian's window is in front, and passes every key on.
    unsafe extern "system" fn proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            // SAFETY: for HC_ACTION, lparam points at the key's details.
            let key = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            let extended = if key.flags & LLKHF_EXTENDED != 0 {
                0xE000
            } else {
                0
            };
            let scancode = key.scanCode | extended;
            for (index, toggle) in [Toggle::Mute, Toggle::Deafen].into_iter().enumerate() {
                if KEYS[index].load(Ordering::Relaxed) != scancode {
                    continue;
                }
                match wparam as u32 {
                    WM_KEYDOWN | WM_SYSKEYDOWN => {
                        let modifiers = (u32::from(held(VK_CONTROL)) * CTRL)
                            | (u32::from(held(VK_SHIFT)) * SHIFT)
                            | (u32::from(held(VK_MENU)) * ALT);
                        if modifiers == MODIFIERS[index].load(Ordering::Relaxed)
                            && !DOWN[index].swap(true, Ordering::Relaxed)
                            && !ours_in_front()
                            && let Some(pressed) = PRESSED
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .as_ref()
                        {
                            pressed(toggle);
                        }
                    }
                    WM_KEYUP | WM_SYSKEYUP => DOWN[index].store(false, Ordering::Relaxed),
                    _ => {}
                }
            }
        }
        // SAFETY: the next hook gets the event as it came.
        unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcut(ctrl: bool, shift: bool, alt: bool) -> Shortcut {
        Shortcut {
            key: Keybind {
                scancode: 0x32,
                name: "M".into(),
                trigger: None,
            },
            ctrl,
            shift,
            alt,
        }
    }

    #[test]
    fn names_the_modifiers_before_the_key() {
        assert_eq!(shortcut(true, true, false).name(), "Ctrl+Shift+M");
        assert_eq!(shortcut(false, false, true).name(), "Alt+M");
        assert_eq!(shortcut(false, false, false).name(), "M");
    }

    #[test]
    fn saves_only_the_modifiers_held() {
        let saved = serde_json::to_value(shortcut(true, false, false)).unwrap();
        assert_eq!(saved["ctrl"], true);
        assert!(saved.get("shift").is_none());
        let read: Shortcut = serde_json::from_value(saved).unwrap();
        assert_eq!(read, shortcut(true, false, false));
    }

    #[test]
    fn nothing_set_watches_nothing() {
        assert!(Watcher::start(None, None, |_| {}).is_none());
    }
}
