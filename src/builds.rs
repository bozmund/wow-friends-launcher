use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize)]
pub struct Installation {
    pub game: PathBuf,
    pub builds: Vec<String>,
    pub executable_build: Option<String>,
}
pub fn parse_build_info(text: &str) -> Result<Vec<String>> {
    let mut lines = text
        .trim_start_matches('\u{feff}')
        .lines()
        .filter(|s| !s.trim().is_empty());
    let keys: Vec<_> = lines
        .next()
        .context("Empty Battle.net build file")?
        .split('|')
        .map(|s| s.split('!').next().unwrap_or(s))
        .collect();
    let index = |name: &str| {
        keys.iter()
            .position(|s| *s == name)
            .with_context(|| format!("Build file has no {name} column"))
    };
    let active = index("Active")?;
    let product = index("Product")?;
    let version = index("Version")?;
    let mut found = BTreeSet::new();
    for line in lines {
        let cols: Vec<_> = line.trim_end_matches('\r').split('|').collect();
        if cols.get(active) == Some(&"1")
            && cols.get(product) == Some(&"wowf")
            && let Some(v) = cols.get(version)
            && valid_build(v)
        {
            found.insert((*v).to_string());
        }
    }
    if found.is_empty() {
        bail!(
            "No active WoW Forever build found. Select Wow.exe inside the installed _forever_ folder."
        );
    }
    Ok(found.into_iter().collect())
}
pub fn valid_build(s: &str) -> bool {
    let a: Vec<_> = s.split('.').collect();
    a.len() == 4
        && a.iter()
            .all(|p| !p.is_empty() && p.len() <= 8 && p.bytes().all(|x| x.is_ascii_digit()))
}
pub fn inspect(game: &Path) -> Result<Installation> {
    if !game
        .file_name()
        .is_some_and(|s| s.eq_ignore_ascii_case("Wow.exe"))
        || !game.is_file()
    {
        bail!("Choose the game's Wow.exe, not Start WoW.exe.");
    }
    let parent = game.parent().context("Game folder missing")?;
    if !parent
        .file_name()
        .is_some_and(|s| s.eq_ignore_ascii_case("_forever_"))
    {
        bail!("Select Wow.exe inside _forever_, not Retail or another WoW edition.");
    }
    let root = parent.parent().context("WoW root missing")?;
    let builds = parse_build_info(
        &fs::read_to_string(root.join(".build.info"))
            .context("Cannot read .build.info beside Data and _forever_")?,
    )?;
    Ok(Installation {
        game: game.to_owned(),
        builds,
        executable_build: crate::platform::exe_version(game),
    })
}
pub fn compatibility(i: &Installation, accepted: &[String]) -> Result<String> {
    let installed = if let Some(actual) = &i.executable_build {
        actual.clone()
    } else if i.builds.len() == 1 {
        i.builds[0].clone()
    } else {
        bail!(
            "Multiple distinct active builds were found: {}. The game executable version could not be resolved; no update or downgrade was performed.",
            i.builds.join(", ")
        );
    };
    if !i.builds.contains(&installed) {
        bail!(
            "Installation is inconsistent or still updating: executable {installed}, Battle.net metadata {}. Finish or verify this installation in Battle.net.",
            i.builds.join(", ")
        );
    }
    if !accepted.contains(&installed) {
        bail!(
            "Installed: {installed}. Server accepts: {}. Battle.net may have updated this installation. Your game files were not changed.",
            accepted.join(", ")
        );
    }
    Ok(installed)
}
pub fn discover() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for env in ["ProgramFiles(x86)", "ProgramFiles", "USERPROFILE"] {
        if let Some(p) = std::env::var_os(env) {
            roots.push(PathBuf::from(p));
        }
    }
    let mut paths = vec![];
    for root in roots {
        for name in [
            "World of Warcraft",
            "Games/World of Warcraft Forever",
            "Games/World of Warcraft",
        ] {
            let p = root.join(name).join("_forever_/Wow.exe");
            if p.is_file() {
                paths.push(p);
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_regions_same_version_are_one_build() {
        let s = "Branch!STRING:0|Active!DEC:1|Version!STRING:0|Product!STRING:0\neu|1|1.60.1.70938|wowf\nus|1|1.60.1.70938|wowf\neu|1|12.0.0.99999|wow\n";
        assert_eq!(parse_build_info(s).unwrap(), vec!["1.60.1.70938"]);
    }
    #[test]
    fn mismatch_reports_installed_and_expected() {
        let i = Installation {
            game: PathBuf::new(),
            builds: vec!["1.60.1.70959".into()],
            executable_build: None,
        };
        let e = compatibility(&i, &["1.60.1.70938".into()])
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("Installed: 1.60.1.70959") && e.contains("Server accepts: 1.60.1.70938")
        );
    }
    #[test]
    fn excludes_inactive_foreign_and_empty_rows() {
        assert!(
            parse_build_info("Active|Product|Version\n0|wowf|1.60.1.70938\n1|wow|12.0.0.99999")
                .is_err()
        );
    }
    #[test]
    fn actual_executable_disambiguates_regions() {
        let i = Installation {
            game: PathBuf::new(),
            builds: vec!["1.60.1.70938".into(), "1.60.1.70959".into()],
            executable_build: Some("1.60.1.70938".into()),
        };
        assert!(compatibility(&i, &["1.60.1.70938".into()]).is_ok());
    }
    #[test]
    fn refuses_half_updated_installation() {
        let i = Installation {
            game: PathBuf::new(),
            builds: vec!["1.60.1.70959".into()],
            executable_build: Some("1.60.1.70938".into()),
        };
        assert!(
            compatibility(&i, &["1.60.1.70938".into()])
                .unwrap_err()
                .to_string()
                .contains("inconsistent")
        );
    }
}
