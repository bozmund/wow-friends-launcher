use crate::{
    config::{Invitation, Settings},
    relay::Relay,
};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{ErrorKind, Write},
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
const FRIENDS_PORTAL: &str = "SET portal \"trinity.actual.battle.net\"";
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
fn sets_variable(line: &str, name: &str) -> bool {
    let mut fields = line.split_whitespace();
    fields
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("SET"))
        && fields
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case(name))
}

pub fn configure_wtf(text: &str, account: &str) -> Result<String> {
    if account.contains(['\r', '\n', '"']) {
        bail!("Invalid game account");
    }
    let mut lines: Vec<_> = text
        .lines()
        .filter(|line| {
            !sets_variable(line.trim_start_matches('\u{feff}'), "portal")
                && !sets_variable(line.trim_start_matches('\u{feff}'), "accountName")
        })
        .map(str::to_owned)
        .collect();
    lines.push(FRIENDS_PORTAL.into());
    lines.push(format!("SET accountName \"{account}\""));
    Ok(lines.join("\r\n") + "\r\n")
}

fn prepare_default_portal(wtf: &Path) -> Result<()> {
    let path = wtf.join("Config.wtf");
    let existing = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("Cannot read WoW's default login settings"),
    };
    let original = existing.as_deref().unwrap_or("");
    let body = original.strip_prefix('\u{feff}').unwrap_or(original);
    let mut portals = body.lines().filter(|line| sets_variable(line, "portal"));
    if portals
        .next()
        .is_some_and(|line| line.trim() == FRIENDS_PORTAL)
        && portals.next().is_none()
    {
        return Ok(());
    }

    if let Some(original) = existing.as_deref() {
        let mut backup = tempfile::NamedTempFile::new_in(wtf)?;
        backup.write_all(original.as_bytes())?;
        match backup.persist_noclobber(wtf.join("Config.wtf.before-friends")) {
            Ok(_) => {}
            Err(error) if error.error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.error).context("Cannot back up WoW's login settings"),
        }
    }

    let newline = if body.contains('\n') && !body.contains("\r\n") {
        "\n"
    } else {
        "\r\n"
    };
    let bom = &original[..original.len() - body.len()];
    let mut updated = bom.to_owned();
    for line in body
        .split_inclusive('\n')
        .filter(|line| !sets_variable(line, "portal"))
    {
        updated.push_str(line);
    }
    if updated.len() > bom.len() && !updated.ends_with('\n') {
        updated.push_str(newline);
    }
    updated.push_str(FRIENDS_PORTAL);
    updated.push_str(newline);
    crate::config::save_atomic(&path, updated.as_bytes())
        .context("Cannot set WoW's default login address to the Friends realm")
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
        let observer = if wtf.join(".wow-friends-controls-test").is_file() {
            format!("JANS_DIAGNOSTICS_SHOW_PANEL=true\n{LUA}")
        } else {
            LUA.to_owned()
        };
        crate::config::save_atomic(&addon.join("JansInputDiagnostics.lua"), observer.as_bytes())?;
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
    if crate::recovery::wow_running()? {
        bail!("Close World of Warcraft before starting a Friends session.");
    }
    let connector = runtime_connector(&root)?;
    let relay = Relay::start(invite.clone()).await?;
    prepare_files(game, &settings, &invite)?;
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
    // Newer clients can load Config.wtf for sign-in even with a private config on the command line.
    prepare_default_portal(&game.parent().context("Missing game folder")?.join("WTF"))?;
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

    #[test]
    fn replaces_saved_eu_portal_without_touching_other_settings() {
        let wtf = tempfile::tempdir().unwrap();
        let path = wtf.path().join("Config.wtf");
        let backup = wtf.path().join("Config.wtf.before-friends");
        let original = "\u{feff}set Portal \"EU\"\r\nSET gxApi \"D3D11\"\r\nSET portal \"US\"\r\nSET accountName \"existing\"";
        fs::write(&path, original).unwrap();

        prepare_default_portal(wtf.path()).unwrap();
        let expected = format!(
            "\u{feff}SET gxApi \"D3D11\"\r\nSET accountName \"existing\"\r\n{FRIENDS_PORTAL}\r\n"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        assert_eq!(fs::read_to_string(&backup).unwrap(), original);

        fs::write(&path, "SET portal \"EU\"\nSET gxApi \"D3D12\"\n").unwrap();
        prepare_default_portal(wtf.path()).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("SET gxApi \"D3D12\"\n{FRIENDS_PORTAL}\n")
        );
        assert_eq!(fs::read_to_string(&backup).unwrap(), original);
    }

    #[test]
    fn sets_portal_for_a_fresh_game_without_creating_a_backup_on_retry() {
        let wtf = tempfile::tempdir().unwrap();
        prepare_default_portal(wtf.path()).unwrap();
        prepare_default_portal(wtf.path()).unwrap();
        assert_eq!(
            fs::read_to_string(wtf.path().join("Config.wtf")).unwrap(),
            format!("{FRIENDS_PORTAL}\r\n")
        );
        assert_eq!(fs::read_dir(wtf.path()).unwrap().count(), 1);
    }
}
