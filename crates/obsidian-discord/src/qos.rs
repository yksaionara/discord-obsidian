//! What Discord's client tells the gateway about whether someone is using
//! it: the `qos` of every heartbeat (op 40, `QOS_HEARTBEAT`) and the
//! `qos_token` of IDENTIFY. See `docs/discord/official-client/gateway.md`.

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

/// The version of the `qos` payload the web client sends.
const VERSION: u64 = 31;

/// Why the client counts as in use, as the web client reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    /// The window is in front.
    pub foregrounded: bool,
    /// This session is connected to a call.
    pub rtc_connected: bool,
}

impl Activity {
    pub fn active(self) -> bool {
        self.foregrounded || self.rtc_connected
    }

    fn reasons(self) -> Vec<&'static str> {
        let mut reasons = Vec::new();
        if self.foregrounded {
            reasons.push("foregrounded");
        }
        if self.rtc_connected {
            reasons.push("rtc_connected");
        }
        reasons
    }
}

/// What the next heartbeats carry, kept as the web client's socket keeps
/// it: a heartbeat reports the state since the last one, so a moment of
/// activity between two beats still counts once.
#[derive(Clone, Debug, Default)]
pub struct Heartbeats {
    /// What the next heartbeat sends; `None` until the first report.
    current: Option<Payload>,
    /// What the beat after it sends.
    upcoming: Option<Activity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Payload {
    active: bool,
    reasons: Vec<&'static str>,
}

impl From<Activity> for Payload {
    fn from(activity: Activity) -> Self {
        Self {
            active: activity.active(),
            reasons: activity.reasons(),
        }
    }
}

impl Heartbeats {
    /// Takes a change of activity. Returns whether a heartbeat should go
    /// now, as the web client beats at once when someone starts using it.
    pub fn report(&mut self, activity: Activity) -> bool {
        let was_active = self.current.as_ref().is_some_and(|payload| payload.active);
        let current = self.current.get_or_insert_with(|| activity.into());
        if activity.active() {
            current.active = true;
            for reason in activity.reasons() {
                if !current.reasons.contains(&reason) {
                    current.reasons.push(reason);
                }
            }
            current.reasons.sort_unstable();
        }
        self.upcoming = Some(activity);
        activity.active() && !was_active
    }

    /// Whether the session counts as in use, for IDENTIFY's token.
    pub fn active(&self) -> bool {
        match (&self.upcoming, &self.current) {
            (Some(upcoming), _) => upcoming.active(),
            (None, Some(current)) => current.active,
            (None, None) => false,
        }
    }

    /// The `qos` of the next heartbeat, moving on to the state after it.
    pub fn take_next(&mut self) -> Value {
        let sent = self.current.take();
        self.current = match self.upcoming.take() {
            Some(upcoming) => Some(upcoming.into()),
            None => sent.clone(),
        };
        match sent {
            Some(payload) => json!({
                "active": payload.active,
                "ver": VERSION,
                "reasons": payload.reasons,
            }),
            None => Value::Null,
        }
    }
}

/// IDENTIFY's `qos_token`: a `discord_protos.qos_token.v1.QosToken` with
/// `client_provided { is_active }` and, once Discord has handed one out in
/// READY's `qos_token`, the signed `derived` data it carries, in base64.
pub fn token(active: bool, derived: Option<&str>) -> String {
    // field 1, client_provided: { field 1, is_active } (false is left out).
    let mut bytes = if active {
        vec![0x0a, 0x02, 0x08, 0x01]
    } else {
        vec![0x0a, 0x00]
    };
    if let Some(derived) = derived.and_then(|derived| STANDARD.decode(derived).ok()) {
        // field 2, derived: the DerivedQosData as Discord sent it.
        bytes.push(0x12);
        varint(derived.len() as u64, &mut bytes);
        bytes.extend_from_slice(&derived);
    }
    STANDARD.encode(bytes)
}

/// The account a token is of: its first part is the user ID in base64.
pub fn user_of(token: &str) -> Option<String> {
    use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
    let first = token.split('.').next()?.trim_end_matches('=');
    let bytes = STANDARD_NO_PAD
        .decode(first)
        .or_else(|_| URL_SAFE_NO_PAD.decode(first))
        .ok()?;
    let id = String::from_utf8(bytes).ok()?;
    (!id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())).then_some(id)
}

fn varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRONT: Activity = Activity {
        foregrounded: true,
        rtc_connected: false,
    };
    const AWAY: Activity = Activity {
        foregrounded: false,
        rtc_connected: false,
    };
    const CALL: Activity = Activity {
        foregrounded: false,
        rtc_connected: true,
    };

    #[test]
    fn beats_without_qos_until_the_first_report() {
        let mut beats = Heartbeats::default();
        assert_eq!(beats.take_next(), Value::Null);
        assert!(!beats.active());
    }

    #[test]
    fn reports_activity_since_the_last_beat() {
        let mut beats = Heartbeats::default();
        assert!(beats.report(FRONT), "starting to use it beats at once");
        // Away again before the beat: the beat still says it was used.
        assert!(!beats.report(AWAY));
        assert_eq!(
            beats.take_next(),
            json!({"active": true, "ver": 31, "reasons": ["foregrounded"]})
        );
        assert_eq!(
            beats.take_next(),
            json!({"active": false, "ver": 31, "reasons": []})
        );
        assert!(!beats.active());
    }

    #[test]
    fn joins_the_reasons_in_order() {
        let mut beats = Heartbeats::default();
        beats.report(FRONT);
        assert!(!beats.report(CALL), "already active");
        assert_eq!(
            beats.take_next()["reasons"],
            json!(["foregrounded", "rtc_connected"])
        );
        assert_eq!(beats.take_next()["reasons"], json!(["rtc_connected"]));
        assert!(beats.active());
    }

    #[test]
    fn encodes_the_token_as_the_web_client_does() {
        assert_eq!(token(false, None), "CgA=");
        assert_eq!(token(true, None), "CgIIAQ==");
        // The signed part goes after, as field 2, byte for byte.
        let derived = STANDARD.encode([1, 2, 3]);
        assert_eq!(
            STANDARD.decode(token(true, Some(&derived))).unwrap(),
            [0x0a, 0x02, 0x08, 0x01, 0x12, 0x03, 1, 2, 3]
        );
        // Something that isn't base64 is left out rather than sent broken.
        assert_eq!(token(false, Some("not base64!")), "CgA=");
    }

    #[test]
    fn reads_whose_a_token_is() {
        let first = STANDARD.encode("80351110224678912");
        assert_eq!(
            user_of(&format!("{}.GxYz.abc", first.trim_end_matches('='))).as_deref(),
            Some("80351110224678912")
        );
        assert_eq!(user_of("token"), None);
    }
}
