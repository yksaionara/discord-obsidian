//! Settings › Devices: the Discord account's logged-in devices, as Discord's
//! page lists them, this one first, and logging them out.

use obsidian_discord::sessions::Device;

use super::*;

/// The Devices page while it's open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Devices {
    /// The devices, once Discord listed them, or why it didn't.
    pub listed: Option<Result<Vec<Device>, String>>,
    /// This login's session, which shows as Current Device.
    pub current: Option<String>,
    /// A logging out on its way, and why the last didn't go.
    pub busy: bool,
    pub error: Option<String>,
}

impl Devices {
    /// This device, and the others, the latest used first.
    pub fn split(&self) -> (Option<&Device>, Vec<&Device>) {
        let listed = match &self.listed {
            Some(Ok(listed)) => listed.as_slice(),
            _ => &[],
        };
        let current = self.current.as_deref();
        let this = listed
            .iter()
            .find(|device| Some(device.hash.as_str()) == current);
        let others = listed
            .iter()
            .filter(|device| Some(device.hash.as_str()) != current)
            .collect();
        (this, others)
    }
}

impl Application {
    /// Devices opens: Discord's list is asked for each time, as its page does.
    pub(super) fn open_devices(&mut self) -> Vec<Effect> {
        if self.state.preview || !self.state.discord_online {
            return vec![];
        }
        self.state.devices = Some(Devices {
            current: self
                .discord
                .as_ref()
                .and_then(|snapshot| snapshot.auth_session.clone()),
            ..Devices::default()
        });
        vec![Effect::Backend(Command::Discord(DiscordCommand::Devices))]
    }

    pub(super) fn devices_came(&mut self, result: Result<Vec<Device>, String>) {
        if let Some(devices) = &mut self.state.devices {
            devices.listed = Some(result);
        }
    }

    /// Logs one device out, by its hash, or with none every device but this.
    pub(super) fn log_out_devices(&mut self, hash: Option<String>) -> Vec<Effect> {
        let Some(devices) = &mut self.state.devices else {
            return vec![];
        };
        if devices.busy {
            return vec![];
        }
        let hashes: Vec<String> = match hash {
            Some(hash) => vec![hash],
            None => devices
                .split()
                .1
                .iter()
                .map(|device| device.hash.clone())
                .collect(),
        };
        if hashes.is_empty() {
            return vec![];
        }
        devices.busy = true;
        devices.error = None;
        vec![Effect::Backend(Command::Discord(
            DiscordCommand::LogOutDevices(hashes),
        ))]
    }

    /// Logged out: the list is asked for again; or it says why not.
    pub(super) fn devices_logged_out(&mut self, result: Result<(), String>) -> Vec<Effect> {
        let Some(devices) = &mut self.state.devices else {
            return vec![];
        };
        devices.busy = false;
        match result {
            Ok(()) => vec![Effect::Backend(Command::Discord(DiscordCommand::Devices))],
            Err(error) => {
                devices.error = Some(format!("Couldn't log out: {error}"));
                vec![]
            }
        }
    }
}
