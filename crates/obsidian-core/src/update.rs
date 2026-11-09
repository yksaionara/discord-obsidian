//! Updates from this build's GitHub releases. A check asks GitHub for the
//! latest release; one newer than this build has its installer downloaded
//! into the data folder's `update` folder, checked against the SHA-256
//! GitHub lists for it, and kept there. The next start, or the rail's
//! Update button, runs it silently: it closes the app, installs over it,
//! and starts it again.
//!
//! Nothing runs at startup but a look for a downloaded installer; the check
//! itself waits on a thread of its own, so starting stays as fast as before.

use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// The GitHub repository whose releases this build updates from.
pub const REPOSITORY: &str = "yksaionara/dc";
/// The installer's file name in each release.
pub const INSTALLER: &str = "Discord-Setup.exe";
/// This build's version: the app's, which it sets once at startup with
/// [`set_version`]; until then, none counts as newer than any release.
static VERSION: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// Gives the updater the app's version, as its Cargo.toml has it.
pub fn set_version(version: &'static str) {
    let _ = VERSION.set(version);
}

fn ours() -> &'static str {
    VERSION.get().copied().unwrap_or("999999.0.0")
}

/// An installer downloaded for a newer version.
#[derive(Clone, Debug)]
pub struct Pending {
    pub version: String,
    pub installer: PathBuf,
}

fn folder(root: &Path) -> PathBuf {
    root.join("update")
}

/// `1.2.3` as numbers to compare; a `v` before it and anything after a `-`
/// are left out.
fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let version = version.trim().trim_start_matches('v');
    let version = version.split('-').next()?;
    let mut parts = version.split('.').map(|part| part.parse::<u64>().ok());
    Some((parts.next()??, parts.next().flatten().unwrap_or(0), parts.next().flatten().unwrap_or(0)))
}

/// Whether `version` comes after this build's.
pub fn is_newer(version: &str) -> bool {
    matches!((parse(version), parse(ours())), (Some(new), Some(ours)) if new > ours)
}

/// A downloaded installer for a version newer than this build, if there is
/// one. Installers for this version or older are deleted.
pub fn pending(root: &Path) -> Option<Pending> {
    let entries = std::fs::read_dir(folder(root)).ok()?;
    let mut best: Option<Pending> = None;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "exe") {
            continue;
        }
        let Some(version) = path.file_stem().and_then(|stem| stem.to_str()).map(str::to_owned)
        else {
            continue;
        };
        if !is_newer(&version) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|best| parse(&version) > parse(&best.version))
        {
            best = Some(Pending {
                version,
                installer: path,
            });
        }
    }
    best
}

/// Runs a downloaded installer silently. It closes this app and starts the
/// new version, in the tray when `background`.
pub fn apply(pending: &Pending, background: bool) -> Result<()> {
    let mut command = std::process::Command::new(&pending.installer);
    command.args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]);
    if background {
        command.arg("/LAUNCH=--background");
    }
    command
        .spawn()
        .with_context(|| format!("could not start the {} installer", pending.version))?;
    tracing::info!(version = pending.version, "installing an update");
    Ok(())
}

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    /// `sha256:` and the file's digest, as GitHub lists it.
    #[serde(default)]
    digest: Option<String>,
}

/// Asks GitHub for the latest release and downloads its installer when
/// it's newer than this build: what's ready to install afterward.
pub fn check(root: &Path) -> Result<Option<Pending>> {
    if let Some(pending) = pending(root) {
        return Ok(Some(pending));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("no runtime for the update check")?;
    runtime.block_on(check_async(root))
}

async fn check_async(root: &Path) -> Result<Option<Pending>> {
    let client = reqwest::Client::builder()
        .user_agent(format!("dc-updater/{}", ours()))
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .context("no HTTP client")?;
    let answer = client
        .get(format!(
            "https://api.github.com/repos/{REPOSITORY}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("could not reach GitHub")?
        .error_for_status()
        .context("GitHub has no latest release")?
        .bytes()
        .await
        .context("GitHub's answer didn't arrive")?;
    let release: Release =
        serde_json::from_slice(&answer).context("GitHub's answer didn't read as a release")?;
    if release.draft || release.prerelease || !is_newer(&release.tag_name) {
        return Ok(None);
    }
    let Some(asset) = release.assets.iter().find(|asset| asset.name == INSTALLER) else {
        bail!("release {} has no {INSTALLER}", release.tag_name);
    };
    let version = release.tag_name.trim_start_matches('v').to_owned();
    let folder = folder(root);
    std::fs::create_dir_all(&folder).context("could not make the update folder")?;
    let partial = folder.join(format!("{version}.part"));
    let mut response = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .context("could not download the update")?
        .error_for_status()
        .context("the update's download failed")?;
    let mut file = std::fs::File::create(&partial).context("could not save the update")?;
    let mut hasher = Sha256::new();
    // Written as it arrives, so the installer never sits whole in memory.
    while let Some(chunk) = response.chunk().await.context("the download stopped")? {
        hasher.update(&chunk);
        file.write_all(&chunk).context("could not save the update")?;
    }
    file.flush()?;
    drop(file);
    if let Some(expected) = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
    {
        let actual: String = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = std::fs::remove_file(&partial);
            bail!("the downloaded update doesn't match GitHub's digest");
        }
    }
    let installer = folder.join(format!("{version}.exe"));
    std::fs::rename(&partial, &installer).context("could not keep the update")?;
    tracing::info!(version, "downloaded an update");
    Ok(Some(Pending { version, installer }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        set_version("0.4.0");
        assert_eq!(parse("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse("0.4"), Some((0, 4, 0)));
        assert_eq!(parse("0.4.1-nightly"), Some((0, 4, 1)));
        assert!(parse("latest").is_none());
        assert!(is_newer("v999.0.0"));
        assert!(!is_newer("0.4.0"));
        assert!(is_newer("v0.4.1"));
        assert!(!is_newer("v0.0.1"));
    }

    #[test]
    fn keeps_only_newer_installers() {
        set_version("0.4.0");
        let root = tempfile::tempdir().unwrap();
        let folder = folder(root.path());
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("0.0.1.exe"), b"old").unwrap();
        std::fs::write(folder.join("998.0.0.exe"), b"new").unwrap();
        std::fs::write(folder.join("999.0.0.exe"), b"newer").unwrap();
        std::fs::write(folder.join("999.1.0.part"), b"half").unwrap();
        let found = pending(root.path()).expect("a newer installer");
        assert_eq!(found.version, "999.0.0");
        assert!(!folder.join("0.0.1.exe").exists(), "the old one goes");
    }

    /// Needs the network: `cargo test -p obsidian-core update -- --ignored`.
    #[test]
    #[ignore = "reaches GitHub"]
    fn downloads_the_latest_release() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        set_version("0.0.1");
        let root = tempfile::tempdir().unwrap();
        let pending = check(root.path()).unwrap().expect("a newer release");
        assert!(pending.installer.metadata().unwrap().len() > 1_000_000);
        eprintln!("downloaded {} to {}", pending.version, pending.installer.display());
    }
}
