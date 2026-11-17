use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const REPO: &str = "bozmund/wow-friends-launcher";
pub const ASSET: &str = "wow-friends-launcher-windows-x86_64.exe";
#[derive(Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub size: u64,
}
#[derive(Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    pub assets: Vec<Asset>,
}
#[derive(Clone)]
pub struct Update {
    pub version: String,
    pub binary: Asset,
    pub sums: Asset,
}
pub fn select(r: Release, current: &str) -> Result<Option<Update>> {
    if r.draft || r.prerelease {
        return Ok(None);
    }
    let candidate = semver::Version::parse(r.tag_name.trim_start_matches('v'))?;
    if candidate <= semver::Version::parse(current)? {
        return Ok(None);
    }
    let asset = |name: &str| {
        r.assets
            .iter()
            .find(|a| a.name == name)
            .cloned()
            .with_context(|| format!("Release publication incomplete: missing {name}"))
    };
    let binary = asset(ASSET)?;
    let sums = asset("SHA256SUMS")?;
    for a in [&binary, &sums] {
        validate_url(&a.browser_download_url)?;
    }
    if binary.size > 128 * 1024 * 1024 {
        bail!("Update exceeds the maximum launcher size");
    }
    Ok(Some(Update {
        version: candidate.to_string(),
        binary,
        sums,
    }))
}
fn validate_url(value: &str) -> Result<()> {
    let u = url::Url::parse(value)?;
    let prefix = format!("/{REPO}/releases/download/");
    if u.scheme() != "https"
        || u.host_str() != Some("github.com")
        || !u.path().starts_with(&prefix)
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || u.port_or_known_default() != Some(443)
    {
        bail!("Update URL is outside the configured release repository");
    }
    Ok(())
}
pub async fn check(client: &reqwest::Client) -> Result<Option<Update>> {
    let response = client
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .send()
        .await?;
    if response.status() == 404 {
        return Ok(None);
    }
    let release: Release = response.error_for_status()?.json().await?;
    select(release, env!("CARGO_PKG_VERSION"))
}
pub fn verify(data: &[u8], sums: &str, digest: Option<&str>) -> Result<()> {
    let expected = sums
        .lines()
        .find_map(|l| {
            let (hash, name) = l.split_once(char::is_whitespace)?;
            (name.trim().trim_start_matches('*') == ASSET).then_some(hash.to_lowercase())
        })
        .context("Launcher missing from SHA256SUMS")?;
    let actual = hex::encode(Sha256::digest(data));
    if expected.len() != 64 || actual != expected {
        bail!("Update checksum mismatch; installed launcher was preserved");
    }
    if let Some(d) = digest
        && d != format!("sha256:{actual}")
    {
        bail!("GitHub asset digest does not match the update");
    }
    if !data.starts_with(b"MZ") {
        bail!("Release is not a Windows executable");
    }
    Ok(())
}
pub async fn install(update: &Update) -> Result<()> {
    // Separate client: never reuse a server-authenticated request or invitation header.
    let client = reqwest::Client::builder()
        .user_agent("WoW-Friends-Updater")
        .timeout(std::time::Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;
    let sums = client
        .get(&update.sums.browser_download_url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    if sums.len() > 64000 {
        bail!("Checksum file too large");
    }
    let mut response = client
        .get(&update.binary.browser_download_url)
        .send()
        .await?
        .error_for_status()?;
    let mut data = Vec::new();
    while let Some(bytes) = response.chunk().await? {
        if data.len() + bytes.len() > 128 * 1024 * 1024 {
            bail!("Update too large");
        }
        data.extend_from_slice(&bytes);
    }
    verify(&data, &sums, update.binary.digest.as_deref())?;
    let dir = crate::config::data_dir()?.join("updates");
    std::fs::create_dir_all(&dir)?;
    let temp = tempfile::NamedTempFile::new_in(dir)?;
    std::fs::write(temp.path(), data)?;
    self_replace::self_replace(temp.path())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_wrong_checksum_and_release_origin() {
        let data = b"MZ disposable executable fixture";
        let good = format!("{}  {ASSET}", hex::encode(Sha256::digest(data)));
        assert!(verify(data, &good, None).is_ok());
        assert!(verify(b"MZ corrupt", &good, None).is_err());
        assert!(validate_url("https://example.invalid/payload").is_err());
    }
    #[test]
    fn semver_and_incomplete_release_are_not_hidden() {
        let r = Release {
            tag_name: "v0.2.0".into(),
            draft: false,
            prerelease: false,
            assets: vec![],
        };
        assert!(select(r, "0.1.0").is_err());
    }
}
