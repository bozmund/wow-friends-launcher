//! A reversible, user-requested isolation profile; never a claimed root-cause fix.
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub fn wow_running() -> Result<bool> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{Foundation::*, System::Diagnostics::ToolHelp::*};
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            bail!("Cannot check whether WoW is running; no files changed.");
        }
        let mut row: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        row.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = unsafe { Process32FirstW(snapshot, &mut row) };
        if ok == 0 {
            unsafe {
                CloseHandle(snapshot);
            }
            bail!("Cannot enumerate game processes; no files changed.");
        }
        let mut found = false;
        while ok != 0 {
            let end = row
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(row.szExeFile.len());
            if String::from_utf16_lossy(&row.szExeFile[..end]).eq_ignore_ascii_case("Wow.exe") {
                found = true;
                break;
            }
            ok = unsafe { Process32NextW(snapshot, &mut row) };
        }
        unsafe {
            CloseHandle(snapshot);
        }
        Ok(found)
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

fn reject_link(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) => {
            let mut linked = m.file_type().is_symlink();
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                linked |= m.file_attributes() & 0x400 != 0;
            }
            anyhow::ensure!(
                !linked,
                "Refusing to move linked game folders; no profile reset performed."
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

pub fn prepare(game: &Path) -> Result<PathBuf> {
    anyhow::ensure!(
        !wow_running()?,
        "Close all WoW windows before repairing controls. Nothing was changed."
    );
    prepare_closed(game)
}

pub fn restore(game: &Path) -> Result<PathBuf> {
    anyhow::ensure!(
        !wow_running()?,
        "Close all WoW windows before restoring settings."
    );
    restore_closed(game)
}

fn restore_closed(game: &Path) -> Result<PathBuf> {
    let folder = game.parent().context("Missing game folder")?;
    anyhow::ensure!(
        folder.file_name().is_some_and(|n| n == "_forever_"),
        "Select the _forever_ game first."
    );
    let backups = folder.join("WoWFriendsRecovery");
    for path in [
        folder.to_owned(),
        backups.clone(),
        folder.join("Interface"),
        folder.join("Interface/AddOns"),
        folder.join("WTF"),
    ] {
        reject_link(&path)?;
    }
    let mut candidates = fs::read_dir(&backups)?
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(|e| e.path())
        .collect::<Vec<_>>();
    candidates.sort();
    let backup = candidates
        .into_iter()
        .rev()
        .find(|p| p.join("original-WTF").is_dir() || p.join("original-AddOns").is_dir())
        .context("No unrestored controls backup found.")?;
    reject_link(&backup)?;
    let mut completed: Vec<(PathBuf, PathBuf)> = Vec::new();
    let result: Result<()> = (|| {
        for (relative, original, replaced) in [
            ("WTF", "original-WTF", "replaced-clean-WTF"),
            (
                "Interface/AddOns",
                "original-AddOns",
                "replaced-clean-AddOns",
            ),
        ] {
            let target = folder.join(relative);
            let source = backup.join(original);
            reject_link(&source)?;
            if !source.exists() {
                continue;
            }
            anyhow::ensure!(source.is_dir(), "Original backup must be a directory.");
            if target.exists() {
                let aside = backup.join(replaced);
                anyhow::ensure!(
                    !aside.exists(),
                    "A prior restore needs review; no data will be overwritten."
                );
                fs::rename(&target, &aside)?;
                completed.push((target.clone(), aside));
            }
            fs::rename(&source, &target)?;
            completed.push((source, target));
        }
        Ok(())
    })();
    if let Err(error) = result {
        for (source, destination) in completed.iter().rev() {
            fs::rename(destination, source).context(
                "Restore rollback needs attention; preserved files remain in recovery folder",
            )?;
        }
        return Err(error);
    }
    Ok(backup)
}

fn prepare_closed(game: &Path) -> Result<PathBuf> {
    anyhow::ensure!(
        game.is_file()
            && game
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("Wow.exe")),
        "Select the game's Wow.exe first."
    );
    let folder = game.parent().context("Missing game folder")?;
    anyhow::ensure!(
        folder.file_name().is_some_and(|n| n == "_forever_"),
        "Recovery is restricted to the selected _forever_ installation."
    );
    anyhow::ensure!(
        !folder.join("WTF/.wow-friends-controls-test").exists(),
        "The clean controls profile is already active. Test it or restore the backup first; nothing was changed."
    );
    for p in [
        folder.to_owned(),
        folder.join("WTF"),
        folder.join("Interface"),
        folder.join("Interface/AddOns"),
        folder.join("WoWFriendsRecovery"),
    ] {
        reject_link(&p)?;
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let backup = folder.join("WoWFriendsRecovery").join(stamp.to_string());
    fs::create_dir_all(&backup)?;
    let staged = backup.join("prepared-WTF");
    fs::create_dir(&staged)?;
    // Stop old account bindings/settings from immediately re-synchronizing into
    // the isolation profile. Characters, spells, quests and saves stay on server.
    fs::write(
        staged.join("ForeverLauncher.wtf"),
        concat!(
            "SET GamePadEnable \"0\"\r\n",
            "SET autoInteract \"0\"\r\n",
            "SET showTutorials \"0\"\r\n",
            "SET showNPETutorials \"0\"\r\n",
            "SET synchronizeBindings \"0\"\r\n",
            "SET synchronizeConfig \"0\"\r\n",
            "SET gxApi \"D3D11\"\r\n",
            "SET maxFPS \"60\"\r\n"
        ),
    )?;
    fs::write(
        staged.join(".wow-friends-controls-test"),
        b"Show the local control diagnostics panel for this reversible test.\n",
    )?;
    let old_wtf = folder.join("WTF");
    let old_addons = folder.join("Interface/AddOns");
    let mut moved = Vec::new();
    for (source, name) in [(&old_wtf, "original-WTF"), (&old_addons, "original-AddOns")] {
        if source.exists() {
            let dest = backup.join(name);
            if let Err(e) = fs::rename(source, &dest) {
                for (src, dst) in moved.iter().rev() {
                    fs::rename(dst, src).context(
                        "Recovery rollback needs attention; original files remain in backup",
                    )?;
                }
                return Err(e).context("Cannot preserve the old profile; repair cancelled");
            }
            moved.push((source.clone(), dest));
        }
    }
    if let Err(e) = fs::rename(&staged, &old_wtf) {
        for (src, dst) in moved.iter().rev() {
            fs::rename(dst, src)
                .context("Recovery rollback needs attention; original files remain in backup")?;
        }
        return Err(e).context("Cannot activate clean settings; original profile restored");
    }
    fs::write(
        backup.join("README.txt"),
        "Original settings, key bindings, macros and third-party addons are preserved here. No game content or server characters were deleted. Test the clean profile before reintroducing addons. To restore, close WoW, move the new WTF/AddOns aside, then move original-WTF and original-AddOns back to their original locations.\r\n",
    )?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean_profile_preserves_all_original_files() {
        let d = tempfile::tempdir().unwrap();
        let game = d.path().join("_forever_/Wow.exe");
        fs::create_dir_all(game.parent().unwrap().join("WTF/Account/fixture")).unwrap();
        fs::create_dir_all(game.parent().unwrap().join("Interface/AddOns/Broken")).unwrap();
        fs::write(&game, b"fixture game asset").unwrap();
        fs::write(
            game.parent()
                .unwrap()
                .join("WTF/Account/fixture/bindings-cache.wtf"),
            b"old bindings",
        )
        .unwrap();
        fs::write(
            game.parent()
                .unwrap()
                .join("Interface/AddOns/Broken/code.lua"),
            b"old addon",
        )
        .unwrap();
        let backup = prepare_closed(&game).unwrap();
        assert_eq!(
            fs::read(backup.join("original-WTF/Account/fixture/bindings-cache.wtf")).unwrap(),
            b"old bindings"
        );
        assert_eq!(
            fs::read(backup.join("original-AddOns/Broken/code.lua")).unwrap(),
            b"old addon"
        );
        assert_eq!(fs::read(&game).unwrap(), b"fixture game asset");
        let fresh =
            fs::read_to_string(game.parent().unwrap().join("WTF/ForeverLauncher.wtf")).unwrap();
        assert!(fresh.contains("SET GamePadEnable \"0\""));
        assert!(fresh.contains("SET synchronizeBindings \"0\""));
        assert!(
            !game
                .parent()
                .unwrap()
                .join("Interface/AddOns/Broken")
                .exists()
        );
    }
    #[test]
    fn rejects_other_game_directories() {
        let d = tempfile::tempdir().unwrap();
        let game = d.path().join("Wow.exe");
        fs::write(&game, b"fixture").unwrap();
        assert!(prepare_closed(&game).is_err());
        assert!(!d.path().join("WoWFriendsRecovery").exists());
    }
    #[test]
    fn diagnostic_inventory_does_not_reveal_other_settings() {
        // Recovery never needs invitation credentials or any account database.
        let d = tempfile::tempdir().unwrap();
        let folder = d.path().join("_forever_");
        fs::create_dir_all(folder.join("WTF")).unwrap();
        fs::write(folder.join("Wow.exe"), b"asset").unwrap();
        fs::write(folder.join("WTF/Config.wtf"), b"original settings").unwrap();
        let backup = prepare_closed(&folder.join("Wow.exe")).unwrap();
        assert_eq!(
            fs::read(backup.join("original-WTF/Config.wtf")).unwrap(),
            b"original settings"
        );
        assert!(
            !fs::read_to_string(folder.join("WTF/ForeverLauncher.wtf"))
                .unwrap()
                .contains("original settings")
        );
    }
    #[test]
    fn restores_original_profile_and_keeps_new_data() {
        let d = tempfile::tempdir().unwrap();
        let folder = d.path().join("_forever_");
        fs::create_dir_all(folder.join("WTF")).unwrap();
        fs::create_dir_all(folder.join("Interface/AddOns/Original")).unwrap();
        fs::write(folder.join("Wow.exe"), b"asset").unwrap();
        fs::write(folder.join("WTF/Config.wtf"), b"original").unwrap();
        let backup = prepare_closed(&folder.join("Wow.exe")).unwrap();
        assert!(prepare_closed(&folder.join("Wow.exe")).is_err());
        assert_eq!(
            fs::read_dir(folder.join("WoWFriendsRecovery"))
                .unwrap()
                .count(),
            1
        );
        fs::write(folder.join("WTF/new-progress.lua"), b"new local data").unwrap();
        fs::create_dir_all(folder.join("Interface/AddOns/JansInputDiagnostics")).unwrap();
        restore_closed(&folder.join("Wow.exe")).unwrap();
        assert_eq!(
            fs::read(folder.join("WTF/Config.wtf")).unwrap(),
            b"original"
        );
        assert_eq!(
            fs::read(backup.join("replaced-clean-WTF/new-progress.lua")).unwrap(),
            b"new local data"
        );
        assert!(folder.join("Interface/AddOns/Original").is_dir());
        assert!(
            backup
                .join("replaced-clean-AddOns/JansInputDiagnostics")
                .is_dir()
        );
        assert!(restore_closed(&folder.join("Wow.exe")).is_err());
    }
}
