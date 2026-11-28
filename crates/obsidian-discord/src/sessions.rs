//! The account's logged-in devices, as Discord's Settings › Devices lists
//! them (`GET /auth/sessions`), each by its session's hash, which logging it
//! out names (`POST /auth/sessions/logout`); see
//! <https://docs.discord.food/authentication#auth-session-object>.

use serde_json::Value;

/// A device the account is logged in on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    pub hash: String,
    /// Its system and client, such as "Windows" and "Discord Client", and
    /// roughly where it was.
    pub os: String,
    pub platform: String,
    pub location: String,
    /// When it was last used, in Unix milliseconds; 0 when Discord doesn't
    /// say.
    pub last_used_ms: u64,
}

/// The devices in `GET /auth/sessions`'s answer, the latest used first.
pub fn devices(value: &Value) -> Vec<Device> {
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    let mut devices: Vec<Device> = value["user_sessions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|session| {
            let info = &session["client_info"];
            Some(Device {
                hash: session["id_hash"].as_str()?.to_owned(),
                os: text(&info["os"]),
                platform: text(&info["platform"]),
                location: text(&info["location"]),
                last_used_ms: session["approx_last_used_time"]
                    .as_str()
                    .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                    .and_then(|at| u64::try_from(at.timestamp_millis()).ok())
                    .unwrap_or_default(),
            })
        })
        .collect();
    devices.sort_by(|a, b| b.last_used_ms.cmp(&a.last_used_ms));
    devices
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lists_the_devices_the_latest_used_first() {
        let listed = devices(&json!({"user_sessions": [
            {"id_hash": "a=", "approx_last_used_time": "2024-03-30T21:30:58.100231+00:00",
             "client_info": {"os": "Windows", "platform": "Discord Client", "location": "Paris, France"}},
            {"id_hash": "b=", "approx_last_used_time": "2024-04-01T10:00:00+00:00",
             "client_info": {"os": "Android", "platform": "Discord Android", "location": null}},
        ]}));
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].hash, "b=");
        assert_eq!(listed[0].location, "");
        assert_eq!(listed[1].os, "Windows");
        assert_eq!(listed[1].last_used_ms, 1_711_834_258_100);
    }
}
