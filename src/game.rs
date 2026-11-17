use crate::{
    config::{Invitation, Settings},
    relay::Relay,
};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

pub struct Running {
    pub child: Child,
    pub _relay: Relay,
}
#[cfg(windows)]
const CONNECTOR: &[u8] = include_bytes!(env!("CONNECTOR_BINARY"));
#[cfg(not(windows))]
const CONNECTOR: &[u8] = &[];
const LUA: &str = include_str!("../assets/JansInputDiagnostics.lua");
const TOC: &str = include_str!("../assets/JansInputDiagnostics.toc");
pub fn runtime_connector(root: &Path) -> Result<PathBuf> {
    if CONNECTOR.is_empty() {
        bail!("The game connector is available in the Windows build");
    }
    let path = root.join("runtime/wow-connector.exe");
    let expected = Sha256::digest(CONNECTOR);
    if fs::read(&path)
        .ok()
        .is_none_or(|bytes| Sha256::digest(&bytes) != expected)
    {
        crate::config::save_atomic(&path, CONNECTOR)?;
    }
    Ok(path)
}
pub fn configure_wtf(text: &str, account: &str) -> Result<String> {
    if account.contains(['\r', '\n', '"']) {
        bail!("Invalid game account");
    }
    let mut lines: Vec<_> = text
        .lines()
        .filter(|l| {
            let fields: Vec<_> = l.split_whitespace().collect();
            !(fields.first() == Some(&"SET")
                && matches!(fields.get(1), Some(&"portal" | &"accountName")))
        })
        .map(str::to_owned)
        .collect();
    lines.push("SET portal \"trinity.actual.battle.net\"".into());
    lines.push(format!("SET accountName \"{account}\""));
    Ok(lines.join("\r\n") + "\r\n")
}
pub fn prepare_files(game: &Path, settings: &Settings, invite: &Invitation) -> Result<()> {
    let folder = game.parent().context("Missing game folder")?;
    let wtf = folder.join("WTF");
    fs::create_dir_all(&wtf)?;
    let own = wtf.join("ForeverLauncher.wtf");
    let original = if own.exists() {
        fs::read_to_string(&own)?
    } else {
        fs::read_to_string(wtf.join("Config.wtf")).unwrap_or_default()
    };
    crate::config::save_atomic(&own, configure_wtf(&original, &invite.account)?.as_bytes())?;
    let addon = folder.join("Interface/AddOns/JansInputDiagnostics");
    if settings.support_diagnostics {
        fs::create_dir_all(&addon)?;
        crate::config::save_atomic(&addon.join("JansInputDiagnostics.lua"), LUA.as_bytes())?;
        crate::config::save_atomic(&addon.join("JansInputDiagnostics.toc"), TOC.as_bytes())?;
    } else if addon.exists() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        fs::rename(
            &addon,
            wtf.join(format!("JansInputDiagnostics-disabled-{stamp}")),
        )?;
    }
    Ok(())
}
pub async fn launch(
    root: PathBuf,
    settings: Settings,
    invite: Invitation,
    accepted: Vec<String>,
) -> Result<Running> {
    let game = settings
        .game
        .as_ref()
        .context("Choose the game before playing")?;
    let install = crate::builds::inspect(game)?;
    crate::builds::compatibility(&install, &accepted)?;
    if !crate::platform::setup_ready(&invite.ca_pem) {
        bail!(
            "One-time Windows setup is needed. Click Windows setup and approve the certificate/localhost setup, then Play again."
        );
    }
    prepare_files(game, &settings, &invite)?;
    let connector = runtime_connector(&root)?;
    let relay = Relay::start(invite).await?;
    let logs = root.join("logs");
    fs::create_dir_all(&logs)?;
    let out = fs::File::create(logs.join("connector.log"))?;
    let mut command = Command::new(connector);
    command
        .args(["--launch"])
        .arg(game)
        .current_dir(game.parent().unwrap())
        .stdout(Stdio::from(out.try_clone()?))
        .stderr(Stdio::from(out));
    crate::platform::hidden(&mut command);
    let child = command
        .spawn()
        .context("Could not start the game connector")?;
    Ok(Running {
        child,
        _relay: relay,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_preserves_settings_without_copying_passwords() {
        let source = "SET gxApi \"D3D11\"\nSET portal \"other\"\nSET accountName \"old\"\n";
        let s = configure_wtf(source, "fixture@local.invalid").unwrap();
        assert!(s.contains("SET gxApi \"D3D11\""));
        assert_eq!(s.matches("SET portal").count(), 1);
        assert!(!s.contains("\"old\""));
        assert!(!s.to_lowercase().contains("password"));
    }
    #[test]
    fn prevents_config_injection() {
        assert!(configure_wtf("", "x\"\nSET scriptErrors 0").is_err());
    }
}
