//! Exercise Windows' running-executable replacement using disposable copies only.
#[cfg(windows)]
#[test]
fn windows_can_replace_its_running_binary() {
    use std::{fs, io::Write, process::Command};
    const MARKER: &[u8] = b"WOW_FRIENDS_DISPOSABLE_UPDATE_FIXTURE";
    if let Some(candidate) = std::env::var_os("WOW_FRIENDS_UPDATE_FIXTURE") {
        self_replace::self_replace(candidate).expect("replace only disposable running fixture");
        assert!(
            fs::read(std::env::current_exe().unwrap())
                .unwrap()
                .ends_with(MARKER)
        );
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let old = temp.path().join("old-fixture.exe");
    let new = temp.path().join("new-fixture.exe");
    let test = std::env::current_exe().unwrap();
    fs::copy(&test, &old).unwrap();
    fs::copy(test, &new).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&new)
        .unwrap()
        .write_all(MARKER)
        .unwrap();
    let status = Command::new(&old)
        .args([
            "--exact",
            "windows_can_replace_its_running_binary",
            "--nocapture",
        ])
        .env("WOW_FRIENDS_UPDATE_FIXTURE", &new)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(fs::read(&old).unwrap().ends_with(MARKER));
}
