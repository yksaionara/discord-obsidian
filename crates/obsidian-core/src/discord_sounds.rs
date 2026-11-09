//! Discord's own sounds, as its client plays them: fetched once from
//! Discord's asset server, the same files its web client loads, and kept in
//! the data folder's `sounds/discord`. Nothing of them is bundled. A sound
//! plays from its file, decoded as it plays and dropped after; until its
//! file has arrived, the caller plays Discord Obsidian's own instead.

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

use anyhow::{Context, Result};

use crate::sounds::Sound;

/// Where the files are kept, set once at startup.
static FOLDER: OnceLock<PathBuf> = OnceLock::new();

/// Keeps the files under the data folder at `root`.
pub fn set_folder(root: &Path) {
    let _ = FOLDER.set(root.join("sounds").join("discord"));
}

/// The ring of an incoming DM call, and the tone while ringing someone.
pub const RINGING: (&str, &str) = ("call_ringing", "c2a7111bb44b8da0");
pub const CALLING: (&str, &str) = ("call_calling", "5146737af413d88e");

/// Each sound's name in Discord's client, and its file on Discord's asset
/// server, as its web client's build lists them.
fn file(sound: Sound) -> (&'static str, &'static str) {
    match sound {
        Sound::Join => ("user_join", "b135ff6c8e091b43"),
        Sound::Leave => ("user_leave", "7b9a183742515fc2"),
        Sound::Disconnect => ("disconnect", "dd0911025fe00b2e"),
        Sound::Mute => ("mute", "2d3b4ba32c34c862"),
        Sound::Unmute => ("unmute", "e74c4a06134a20e4"),
        Sound::Deafen => ("deafen", "529ff198eac567af"),
        Sound::Undeafen => ("undeafen", "b150f03c89944403"),
        // Discord's client plays its message sound for mentions too.
        Sound::Message | Sound::Mention => ("message1", "3ed22d14f3c30bc4"),
        Sound::ViewerJoin => ("stream_user_joined", "3d2af2764581bb2e"),
        Sound::ViewerLeave => ("stream_user_left", "24ae9dc0184bd570"),
        Sound::StreamStart => ("stream_started", "abe52a3c92953edb"),
        Sound::StreamStop => ("stream_ended", "cbff07b5a83e0321"),
    }
}

/// How loud Discord's files play against Discord Obsidian's own sounds, which peak at
/// a quarter of full scale at 100%.
const GAIN: f32 = 0.3;

/// `sound` as 48 kHz interleaved stereo at [`GAIN`], if its file is here.
pub fn samples(sound: Sound) -> Option<Vec<f32>> {
    named(file(sound))
}

/// A named file's sound as 48 kHz interleaved stereo at [`GAIN`], if it's here.
pub fn named((name, _): (&str, &str)) -> Option<Vec<f32>> {
    let path = FOLDER.get()?.join(format!("{name}.mp3"));
    let data = std::fs::read(path).ok()?;
    match crate::soundboard::decode(&data) {
        Ok(pcm) => Some(
            pcm.into_iter()
                .map(|sample| f32::from(sample) / f32::from(i16::MAX) * GAIN)
                .collect(),
        ),
        Err(error) => {
            tracing::warn!(error = format!("{error:#}"), name, "a Discord sound didn't decode");
            None
        }
    }
}

/// Whether a fetch is under way, so choosing the set again doesn't start
/// another.
static FETCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A while after it's asked for, so starting is as quick as before.
const FETCH_DELAY: Duration = Duration::from_secs(5);

/// Fetches every file not yet here, on a thread of its own.
pub fn fetch_missing() {
    use std::sync::atomic::Ordering;
    let Some(folder) = FOLDER.get().cloned() else {
        return;
    };
    let missing: Vec<(&str, &str)> = Sound::ALL
        .iter()
        .map(|sound| file(*sound))
        .chain([RINGING, CALLING])
        .filter(|(name, _)| !folder.join(format!("{name}.mp3")).exists())
        .collect();
    if missing.is_empty() || FETCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    let started = std::thread::Builder::new()
        .name("discord-sounds".into())
        .spawn(move || {
            std::thread::sleep(FETCH_DELAY);
            if let Err(error) = fetch(&folder, &missing) {
                tracing::info!(error = format!("{error:#}"), "Discord's sounds didn't all arrive");
            }
            FETCHING.store(false, Ordering::Relaxed);
        });
    if started.is_err() {
        FETCHING.store(false, Ordering::Relaxed);
    }
}

fn fetch(folder: &Path, files: &[(&str, &str)]) -> Result<()> {
    std::fs::create_dir_all(folder).context("no folder for Discord's sounds")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("no runtime to fetch Discord's sounds")?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("no HTTP client")?;
        for (name, hash) in files {
            let data = client
                .get(format!("https://discord.com/assets/{hash}.mp3"))
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .with_context(|| format!("couldn't fetch {name}"))?
                .bytes()
                .await
                .with_context(|| format!("{name} didn't arrive"))?;
            // Written whole under another name first, so a cut-off file is
            // never taken for the sound.
            let part = folder.join(format!("{name}.part"));
            std::fs::write(&part, &data).with_context(|| format!("couldn't save {name}"))?;
            std::fs::rename(&part, folder.join(format!("{name}.mp3")))
                .with_context(|| format!("couldn't save {name}"))?;
        }
        tracing::info!(count = files.len(), "fetched Discord's sounds");
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every sound's file is on Discord's asset server and plays. Needs the
    /// network.
    #[test]
    #[ignore = "fetches from discord.com"]
    fn discords_files_arrive_and_decode() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let folder = tempfile::tempdir().unwrap();
        set_folder(folder.path());
        let all: Vec<_> = Sound::ALL.iter().map(|sound| file(*sound)).chain([RINGING, CALLING]).collect();
        fetch(FOLDER.get().unwrap(), &all).unwrap();
        for sound in Sound::ALL {
            let samples = samples(sound).expect("decodes");
            assert!(samples.len() > 4800, "{sound:?}");
        }
        assert!(named(RINGING).is_some());
    }
}
