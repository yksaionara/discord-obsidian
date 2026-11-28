//! Settings › Devices: the Discord account's logged-in devices; see the
//! core's `application/devices.rs`.

use super::*;

impl Adapter {
    /// The Devices page's lists, while it's open.
    pub(super) fn show_devices(&self, window: &ObsidianWindow) {
        let Some(devices) = &self.app.state.devices else {
            return;
        };
        if !window.get_settings_open() || window.get_settings_section() != 11 {
            return;
        }
        let now = obsidian_core::clock::now_ms();
        let row = |device: &obsidian_core::obsidian_discord::sessions::Device, current: bool| {
            let mut title = [device.os.as_str(), device.platform.as_str()]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            if title.is_empty() {
                title = "Unknown device".into();
            }
            let when = if current || device.last_used_ms == 0 {
                String::new()
            } else {
                super::refresh::ago(device.last_used_ms, now)
            };
            let detail = [device.location.as_str(), when.as_str()]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let os = device.os.to_lowercase();
            DeviceRow {
                hash: device.hash.as_str().into(),
                title: title.into(),
                detail: detail.into(),
                phone: os.contains("android") || os.contains("ios"),
            }
        };
        let (this, others) = devices.split();
        window.set_devices_loading(devices.listed.is_none());
        window.set_devices_error(
            match (&devices.listed, &devices.error) {
                (_, Some(error)) => error.clone(),
                (Some(Err(error)), None) => error.clone(),
                _ => String::new(),
            }
            .into(),
        );
        window.set_devices_busy(devices.busy);
        window.set_devices_current(ModelRc::new(VecModel::from(
            this.map(|device| row(device, true))
                .into_iter()
                .collect::<Vec<_>>(),
        )));
        window.set_devices_others(ModelRc::new(VecModel::from(
            others
                .into_iter()
                .map(|device| row(device, false))
                .collect::<Vec<_>>(),
        )));
    }
}

pub(super) fn bind(window: &ObsidianWindow, adapter: &Rc<RefCell<Adapter>>) {
    macro_rules! act {
        ($callback:ident($($arg:ident),*) => $action:expr) => {{
            let state = adapter.clone();
            let weak = window.as_weak();
            window.$callback(move |$($arg),*| {
                if let Some(window) = weak.upgrade() {
                    state.borrow_mut().action(&window, $action);
                }
            });
        }};
    }
    act!(on_open_account_devices() => Action::OpenDevices(true));
    act!(on_log_out_device(hash) => Action::LogOutDevices(Some(hash.into())));
    act!(on_log_out_all_devices() => Action::LogOutDevices(None));
}
