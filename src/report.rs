//! An error report to paste to whoever looks after this build: its version,
//! the system, and the end of the log, with anything that looks like a token
//! or a password left out. Nothing is sent anywhere; the rail's version
//! label copies it.
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
};

/// How much of the log's end goes in, in bytes.
const LOG_TAIL: u64 = 24 * 1024;

/// The report for the data folder at `root`.
pub fn build(root: &Path) -> String {
    let mut report = format!(
        "Discord {} error report\nSystem: {} {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    match tail(&root.join("obsidian.log")) {
        Some(log) => {
            report.push_str("\nLog (last part):\n");
            for line in log.lines() {
                report.push_str(&scrub(line));
                report.push('\n');
            }
        }
        None => report.push_str("\nNo log was found.\n"),
    }
    report
}

/// The log's last [`LOG_TAIL`] bytes, from the first whole line in them.
fn tail(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(LOG_TAIL);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::with_capacity((length - start) as usize);
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let text = if start > 0 {
        text.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        &text
    };
    Some(text.to_owned())
}

/// A log line with secrets left out: values after words such as "token" or
/// "password", and long runs that look like a token or a key.
fn scrub(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut hide_next = false;
    for (index, word) in line.split(' ').enumerate() {
        if index > 0 {
            out.push(' ');
        }
        let lower = word.to_ascii_lowercase();
        let secret_name = [
            "token",
            "password",
            "authorization",
            "secret",
            "cookie",
            "ticket",
        ]
        .iter()
        .any(|name| lower.contains(name));
        if hide_next || looks_secret(word) {
            out.push_str("[hidden]");
            hide_next = false;
        } else if secret_name && let Some((name, value)) = word.split_once('=') {
            out.push_str(name);
            out.push_str(if value.is_empty() { "=" } else { "=[hidden]" });
        } else {
            out.push_str(word);
            hide_next = secret_name && word.ends_with(':');
        }
    }
    out
}

/// Long and made only of token characters, with letters and digits mixed.
fn looks_secret(word: &str) -> bool {
    let word = word.trim_matches(|c: char| "\"',;()[]{}".contains(c));
    word.len() >= 32
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-+/=".contains(c))
        && word.chars().any(|c| c.is_ascii_digit())
        && word.chars().any(|c| c.is_ascii_alphabetic())
        && !word.contains("::")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_out_tokens_and_passwords() {
        assert_eq!(
            scrub("signed in token=MTA4.abc password: hunter2 ok"),
            "signed in token=[hidden] password: [hidden] ok"
        );
        // Made up, and put together here so it doesn't read as a real one.
        let token = ["x1Y2", &"z3".repeat(10), ".", &"Q4w".repeat(8)].concat();
        assert_eq!(scrub(&format!("auth {token} done")), "auth [hidden] done");
        assert_eq!(scrub("stream at 1440p 360 fps"), "stream at 1440p 360 fps");
    }

    #[test]
    fn reads_the_logs_last_whole_lines() {
        let folder = tempfile::tempdir().unwrap();
        let line = "x".repeat(100);
        let log: String = (0..1000).map(|n| format!("{n} {line}\n")).collect();
        std::fs::write(folder.path().join("obsidian.log"), &log).unwrap();
        let report = build(folder.path());
        assert!(report.contains("999 xxx"));
        assert!(!report.contains("\n0 xxx"));
        assert!(report.len() < 26 * 1024);
    }
}
