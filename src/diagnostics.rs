use crate::config::{Invitation, Settings};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

pub fn redact(text: &str) -> String {
    let secret = regex::Regex::new(
        "(?i)password|ticket|session.?key|authorization|bearer |accountname|token",
    )
    .unwrap();
    let email = regex::Regex::new(r"[\w.+-]+@[\w.-]+").unwrap();
    let user = regex::Regex::new(r"(?i)[a-z]:[\\/]Users[\\/][^\\/\s]+").unwrap();
    let digest = regex::Regex::new(r"[a-fA-F0-9]{32,}").unwrap();
    let mut result = text
        .lines()
        .filter(|l| !secret.is_match(l))
        .rev()
        .take(80)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|l| {
            let limited: String = l.chars().take(500).collect();
            let v = email.replace_all(&limited, "[email]");
            let v = user.replace_all(&v, "[user]");
            digest.replace_all(&v, "[digest]").to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    if result.len() > 8000 {
        let mut end = 8000;
        while !result.is_char_boundary(end) {
            end -= 1;
        }
        result.truncate(end);
    }
    result
}
fn tail(path: &Path) -> String {
    let Ok(mut f) = fs::File::open(path) else {
        return String::new();
    };
    let Ok(meta) = f.metadata() else {
        return String::new();
    };
    if f.seek(SeekFrom::Start(meta.len().saturating_sub(16000)))
        .is_err()
    {
        return String::new();
    }
    let mut b = Vec::new();
    let _ = f.take(16000).read_to_end(&mut b);
    redact(&String::from_utf8_lossy(&b))
}
fn control_profile(folder: &Path) -> String {
    let addon = folder.join("Interface/AddOns/JansInputDiagnostics");
    let mut lines = vec![format!(
        "Control observer installed: lua={} toc={} cleanProfile={}",
        addon.join("JansInputDiagnostics.lua").is_file(),
        addon.join("JansInputDiagnostics.toc").is_file(),
        folder.join("WTF/.wow-friends-controls-test").is_file()
    )];
    if let Ok(file) = fs::File::open(folder.join("WTF/ForeverLauncher.wtf")) {
        let mut text = String::new();
        let _ = file.take(65536).read_to_string(&mut text);
        for line in text.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() == 3
                && fields[0] == "SET"
                && [
                    "gamepadenable",
                    "autointeract",
                    "shownpetutorials",
                    "showtutorials",
                    "synchronizebindings",
                    "synchronizeconfig",
                ]
                .contains(&fields[1].to_ascii_lowercase().as_str())
            {
                let value = fields[2].trim_matches('"');
                if value.len() <= 8 && value.bytes().all(|b| b.is_ascii_digit()) {
                    lines.push(format!("Control setting {}={value}", fields[1]));
                }
            }
        }
    }
    // Inspect only our addon's enabled flag, never other addon names, macros,
    // bindings or account-folder names. WoW writes saved variables at logout.
    for entry in walkdir::WalkDir::new(folder.join("WTF"))
        .max_depth(8)
        .follow_links(false)
        .into_iter()
        .take(15000)
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file()
            && entry.file_name() == "AddOns.txt"
            && let Ok(file) = fs::File::open(entry.path())
        {
            let mut text = String::new();
            let _ = file.take(65536).read_to_string(&mut text);
            for line in text.lines() {
                if let Some((name, value)) = line.split_once(':')
                    && name.trim() == "JansInputDiagnostics"
                {
                    let value = value.trim();
                    if ["enabled", "disabled", "0", "1"].contains(&value) {
                        lines.push(format!("Observer saved enabled flag: {value}"));
                    }
                }
            }
        }
        if lines.len() >= 20 {
            break;
        }
    }
    lines.join("\n")
}
#[derive(Serialize)]
pub struct Report {
    version: u8,
    session: String,
    running: bool,
    responding: bool,
    #[serde(rename = "memoryMiB")]
    memory_mib: u64,
    build: String,
    logs: BTreeMap<String, String>,
}
pub fn collect(
    settings: &Settings,
    root: &Path,
    session: &str,
    helper: Option<u32>,
    preflight: &str,
) -> Report {
    let (running, responding, memory_mib) = crate::platform::game_process_status(helper);
    let mut logs = BTreeMap::new();
    let mut build = "not-selected".to_string();
    logs.insert(
        "client".into(),
        redact(&format!(
            "Rust launcher {}\n{preflight}",
            env!("CARGO_PKG_VERSION")
        )),
    );
    if let Some(game) = &settings.game
        && let Some(folder) = game.parent()
    {
        logs.entry("client".into()).and_modify(|text| {
            text.push('\n');
            text.push_str(&control_profile(folder));
            *text = redact(text);
        });
        for (kind, name) in [("gx", "gx.log"), ("general", "General.log")] {
            logs.insert(kind.into(), tail(&folder.join("Logs").join(name)));
        }
        // Even broken/half-updated Battle.net metadata must not hide the actual
        // executable build from support when Play is blocked.
        build = crate::platform::exe_version(game).unwrap_or_else(|| "unknown".into());
        match crate::builds::inspect(game) {
            Ok(i) => {
                build = i.executable_build.unwrap_or_else(|| i.builds.join(","));
            }
            Err(error) => {
                logs.entry("client".into()).and_modify(|s| {
                    s.push_str(&format!("\nVersion metadata error: {error}"));
                    *s = redact(s);
                });
            }
        }
        if let Ok(entries) = fs::read_dir(folder.join("Errors")) {
            let latest = entries
                .filter_map(Result::ok)
                .filter(|e| {
                    e.file_name().to_string_lossy().contains("Crash")
                        && e.path().extension().is_some_and(|s| s == "txt")
                })
                .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
            if let Some(e) = latest
                && let Ok(mut file) = fs::File::open(e.path())
            {
                let mut bytes = vec![];
                let _ = (&mut file).take(20000).read_to_end(&mut bytes);
                let head = String::from_utf8_lossy(&bytes)
                    .lines()
                    .take(65)
                    .take_while(|s| {
                        !s.contains("DBG-DATA")
                            && !s.contains("CVar Settings")
                            && !s.contains("Loaded Modules")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                logs.insert("crash".into(), redact(&head));
            }
        }
        // Read only our observer, not any other addon's data. Bound traversal.
        let latest = walkdir::WalkDir::new(folder.join("WTF"))
            .max_depth(8)
            .follow_links(false)
            .into_iter()
            .take(15000)
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file() && e.file_name() == "JansInputDiagnostics.lua")
            .max_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()));
        if let Some(e) = latest {
            logs.insert("observer".into(), tail(e.path()));
        }
    }
    let connector = tail(&root.join("logs/connector.log"));
    if !connector.is_empty() {
        logs.entry("client".into()).and_modify(|s| {
            s.push('\n');
            s.push_str(&connector);
            *s = redact(s);
        });
    }
    Report {
        version: 1,
        session: session.chars().take(64).collect(),
        running,
        responding,
        memory_mib,
        build,
        logs,
    }
}
pub async fn upload(
    client: &reqwest::Client,
    invite: &Invitation,
    report: &Report,
) -> anyhow::Result<()> {
    let response = client
        .post(format!("{}/diagnostics", invite.origin()))
        .bearer_auth(&invite.token)
        .json(report)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!(network_problem(&e)))?;
    let code = response.status().as_u16();
    if !response.status().is_success() {
        anyhow::bail!(upload_problem(code));
    }
    Ok(())
}
/// Explains a failed connection without the URL or any header (no credentials).
pub fn network_problem(e: &reqwest::Error) -> String {
    let kind = if e.is_timeout() {
        "the server did not answer within 10 seconds"
    } else if e.is_connect() {
        "this PC could not connect to the server (offline, firewall or antivirus?)"
    } else {
        "the connection failed"
    };
    format!("Cannot reach the private server: {kind}.")
}
pub fn upload_problem(code: u16) -> String {
    match code {
        401 => "the server rejected this invitation (revoked or replaced). Ask the server owner for a new one.".into(),
        403 => "the server refused reports for this account.".into(),
        404 => "this server does not accept support reports yet.".into(),
        429 => "reports were sent too often; the next one will go through.".into(),
        400 | 413 => format!("the server rejected the report format (HTTP {code}). Tell the server owner."),
        _ => format!("the server answered HTTP {code}."),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removes_credentials_and_personal_paths() {
        let s = redact(
            "render error\npassword SECRET\nBearer ABC\nC:\\Users\\PrivateName\\WoW\\gx.log someone@example.com\n0123456789012345678901234567890123",
        );
        assert!(s.contains("render error"));
        for bad in [
            "SECRET",
            "ABC",
            "PrivateName",
            "someone@example.com",
            "0123456789012345678901234567890123",
        ] {
            assert!(!s.contains(bad));
        }
    }
    #[test]
    fn upload_errors_are_explained() {
        assert!(upload_problem(401).contains("rejected this invitation"));
        assert!(upload_problem(404).contains("does not accept"));
        assert!(upload_problem(500).contains("HTTP 500"));
    }
    #[test]
    fn bounds_large_lines() {
        assert!(redact(&"x".repeat(100000)).len() <= 8000);
    }
    #[test]
    fn bounds_unicode_by_bytes() {
        assert!(redact(&("界".repeat(500) + "\n").repeat(80)).len() <= 8000);
    }
    #[test]
    fn blocked_launch_still_has_a_version_report() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("_forever_");
        std::fs::create_dir(&folder).unwrap();
        let game = folder.join("Wow.exe");
        std::fs::write(&game, b"non-executable test fixture").unwrap();
        std::fs::write(
            root.path().join(".build.info"),
            "Active|Product|Version\n1|wowf|1.60.1.70959\n",
        )
        .unwrap();
        let settings = Settings {
            game: Some(game),
            ..Default::default()
        };
        let r = collect(
            &settings,
            root.path(),
            "fixture",
            None,
            "Installed 70959; server accepts 70938",
        );
        assert_eq!(r.build, "1.60.1.70959");
        assert!(r.logs["client"].contains("server accepts 70938"));
        std::fs::write(root.path().join(".build.info"), "broken metadata").unwrap();
        let r = collect(&settings, root.path(), "fixture", None, "Play blocked");
        assert!(r.logs["client"].contains("Version metadata error"));
    }
    #[test]
    fn control_inventory_is_allowlisted() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("WTF/Account/private-name")).unwrap();
        fs::write(d.path().join("WTF/ForeverLauncher.wtf"), "SET GamePadEnable \"1\"\nSET autoInteract \"1\"\nSET accountName \"private@example.invalid\"\nSET showTutorials \"123456789-bad\"\n").unwrap();
        fs::write(
            d.path().join("WTF/Account/private-name/AddOns.txt"),
            "OtherPrivateAddon: enabled\nJansInputDiagnostics: disabled\n",
        )
        .unwrap();
        let text = control_profile(d.path());
        assert!(text.contains("GamePadEnable=1"));
        assert!(text.contains("autoInteract=1"));
        assert!(text.contains("flag: disabled"));
        for secret in [
            "private-name",
            "private@example.invalid",
            "OtherPrivateAddon",
            "123456789-bad",
        ] {
            assert!(!text.contains(secret));
        }
    }
}
