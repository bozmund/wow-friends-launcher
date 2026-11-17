use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

// Deliberately no Debug implementation: tokens/passwords must never enter logs.
#[derive(Clone, Serialize, Deserialize)]
pub struct Invitation {
    pub name: String,
    pub account: String,
    pub token: String,
    pub relay: String,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default, rename = "caPem")]
    pub ca_pem: String,
    #[serde(default, rename = "allowedBuilds")]
    pub allowed_builds: Vec<String>,
}
impl Invitation {
    pub fn validate(&self) -> Result<()> {
        let u = url::Url::parse(&self.relay).context("Invalid invitation server URL")?;
        if u.scheme() != "https"
            || u.host_str().is_none()
            || !u.username().is_empty()
            || u.password().is_some()
            || u.query().is_some()
            || u.fragment().is_some()
            || u.path() != "/"
        {
            bail!("Invitation requires an HTTPS server origin");
        }
        if self.token.is_empty()
            || self.token.len() > 180
            || !self.token.bytes().all(|b| b.is_ascii_graphic())
        {
            bail!("Invalid invitation credential");
        }
        if self.account.is_empty()
            || self.account.len() > 256
            || self.account.contains(['\r', '\n', '"'])
            || self.name.len() > 100
        {
            bail!("Invalid invitation account");
        }
        if self.password.as_ref().is_some_and(|p| p.len() > 512) {
            bail!("Invalid invitation password");
        }
        if self.allowed_builds.len() > 16
            || !self
                .allowed_builds
                .iter()
                .all(|s| crate::builds::valid_build(s))
        {
            bail!("Invalid accepted builds in the private invitation");
        }
        Ok(())
    }
    pub fn origin(&self) -> String {
        self.relay.trim_end_matches('/').to_owned()
    }
    pub fn expected_builds(&self) -> Vec<String> {
        if self.allowed_builds.is_empty() {
            vec!["1.60.1.70938".into()]
        } else {
            self.allowed_builds.clone()
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub game: Option<PathBuf>,
    #[serde(default)]
    pub support_diagnostics: bool,
    #[serde(default = "yes")]
    pub auto_update: bool,
    #[serde(default)]
    pub support_consent_version: u32,
}
fn yes() -> bool {
    true
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            game: None,
            support_diagnostics: false,
            auto_update: true,
            support_consent_version: 0,
        }
    }
}

pub fn data_dir() -> Result<PathBuf> {
    // Isolated profile for testing a build beside a real installation.
    if let Some(p) = std::env::var_os("WOW_FRIENDS_LAUNCHER_DATA").filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        fs::create_dir_all(&p)?;
        return Ok(p);
    }
    let p = directories::BaseDirs::new()
        .context("Cannot find user profile")?
        .data_local_dir()
        .join("WoWFriendsLauncher");
    fs::create_dir_all(&p)?;
    Ok(p)
}
pub fn save_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing settings folder")?;
    fs::create_dir_all(parent)?;
    let temp = tempfile::NamedTempFile::new_in(parent)?;
    fs::write(temp.path(), bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o600))?;
    }
    for _ in 0..30 {
        if fs::rename(temp.path(), path).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    bail!("Windows has locked the settings file. Please retry.")
}
pub fn load_settings(root: &Path) -> Settings {
    fs::read(root.join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_slice(&s).ok())
        .unwrap_or_default()
}
pub fn save_settings(root: &Path, s: &Settings) -> Result<()> {
    save_atomic(&root.join("settings.json"), &serde_json::to_vec_pretty(s)?)
}
pub fn store_invite(root: &Path, i: &Invitation) -> Result<()> {
    i.validate()?;
    let encrypted = crate::platform::protect(&serde_json::to_vec(i)?)?;
    save_atomic(&root.join("invitation.dpapi"), &encrypted)
}
pub fn load_invite(root: &Path) -> Result<Invitation> {
    let bytes = fs::read(root.join("invitation.dpapi"))?;
    let i: Invitation = serde_json::from_slice(&crate::platform::unprotect(&bytes)?)?;
    i.validate()?;
    Ok(i)
}
pub fn import_invite(path: &Path) -> Result<(Invitation, Option<Settings>)> {
    if fs::metadata(path)?.len() > 1024 * 1024 {
        bail!("Invitation file is too large");
    }
    let mut invite: Invitation =
        serde_json::from_slice(&fs::read(path).context("Cannot read invitation")?)
            .context("This is not a valid private invitation")?;
    let parent = path.parent().context("Invitation has no folder")?;
    if invite.ca_pem.is_empty() {
        if fs::metadata(parent.join("ca.crt.pem")).is_ok_and(|m| m.len() > 1024 * 1024) {
            bail!("Public certificate file is too large");
        }
        invite.ca_pem = fs::read_to_string(parent.join("ca.crt.pem")).unwrap_or_default();
    }
    invite.validate()?;
    let legacy = fs::read(parent.join("player-settings.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .map(|v| Settings {
            game: v.get("game").and_then(|s| s.as_str()).map(PathBuf::from),
            support_diagnostics: v
                .get("supportDiagnostics")
                .and_then(|s| s.as_bool())
                .unwrap_or(false),
            support_consent_version: v
                .get("supportConsentVersion")
                .and_then(|s| s.as_u64())
                .unwrap_or(0) as u32,
            auto_update: true,
        });
    Ok((invite, legacy))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn invite() -> Invitation {
        Invitation {
            name: "Fixture".into(),
            account: "fixture@local.invalid".into(),
            token: "fixture-only".into(),
            relay: "https://example.invalid".into(),
            password: None,
            ca_pem: String::new(),
            allowed_builds: vec![],
        }
    }
    #[test]
    fn rejects_insecure_and_credential_urls() {
        for url in [
            "http://example.invalid",
            "https://user:pass@example.invalid",
            "https://example.invalid/path",
            "https://example.invalid/?token=x",
        ] {
            let mut i = invite();
            i.relay = url.into();
            assert!(i.validate().is_err());
        }
    }
    #[test]
    fn protected_invite_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let i = invite();
        store_invite(d.path(), &i).unwrap();
        assert_eq!(load_invite(d.path()).unwrap().token, i.token);
        #[cfg(windows)]
        assert!(
            !String::from_utf8_lossy(&fs::read(d.path().join("invitation.dpapi")).unwrap())
                .contains("fixture-only")
        );
    }
    #[test]
    fn defaults_do_not_enable_support() {
        assert!(!Settings::default().support_diagnostics);
    }
    #[test]
    fn rejects_header_injection_and_invalid_builds() {
        let mut i = invite();
        i.token = "bad\r\nInjected: value".into();
        assert!(i.validate().is_err());
        let mut i = invite();
        i.allowed_builds = vec!["anything".into()];
        assert!(i.validate().is_err());
    }
    #[test]
    fn replacing_settings_keeps_valid_private_data() {
        let d = tempfile::tempdir().unwrap();
        let i = invite();
        store_invite(d.path(), &i).unwrap();
        store_invite(d.path(), &i).unwrap();
        save_settings(d.path(), &Settings::default()).unwrap();
        let mut s = load_settings(d.path());
        s.auto_update = false;
        save_settings(d.path(), &s).unwrap();
        assert!(!load_settings(d.path()).auto_update);
        assert_eq!(load_invite(d.path()).unwrap().token, i.token);
    }
}
