use anyhow::{Context, Result, bail};
use std::path::Path;

#[cfg(windows)]
pub fn protect(bytes: &[u8]) -> Result<Vec<u8>> {
    crypt(bytes, true)
}
#[cfg(windows)]
pub fn unprotect(bytes: &[u8]) -> Result<Vec<u8>> {
    crypt(bytes, false)
}
#[cfg(windows)]
fn crypt(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
    if bytes.len() > 1024 * 1024 {
        bail!("Private invitation is too large");
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        bail!("Windows could not protect/open this invitation for your user account");
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(result)
}
#[cfg(not(windows))]
pub fn protect(bytes: &[u8]) -> Result<Vec<u8>> {
    Ok(bytes.to_vec())
}
#[cfg(not(windows))]
pub fn unprotect(bytes: &[u8]) -> Result<Vec<u8>> {
    Ok(bytes.to_vec())
}

#[cfg(windows)]
fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
pub fn normalize_version(s: &str) -> Option<String> {
    let re = regex::Regex::new(r"(\d+)\.(\d+)\.(\d+)\.(\d+)").ok()?;
    re.captures(s)
        .map(|m| format!("{}.{}.{}.{}", &m[1], &m[2], &m[3], &m[4]))
}
#[cfg(windows)]
pub fn exe_version(path: &Path) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::*;
    let name = wide(path);
    let mut ignored = 0;
    let size = unsafe { GetFileVersionInfoSizeW(name.as_ptr(), &mut ignored) };
    if size == 0 || size > 4 * 1024 * 1024 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    if unsafe { GetFileVersionInfoW(name.as_ptr(), 0, size, data.as_mut_ptr().cast()) } == 0 {
        return None;
    }
    let mut ptr = std::ptr::null_mut();
    let mut length = 0;
    let translation = wide("\\VarFileInfo\\Translation");
    let mut languages = vec![(0x0409u16, 0x04b0u16), (0x0409, 0x04e4)];
    if unsafe {
        VerQueryValueW(
            data.as_ptr().cast(),
            translation.as_ptr(),
            &mut ptr,
            &mut length,
        )
    } != 0
        && length >= 4
    {
        let words = unsafe { std::slice::from_raw_parts(ptr.cast::<u16>(), length as usize / 2) };
        for p in words.chunks_exact(2) {
            languages.insert(0, (p[0], p[1]));
        }
    }
    for (lang, charset) in languages {
        for field in ["FileVersion", "ProductVersion"] {
            let q = wide(format!(
                "\\StringFileInfo\\{lang:04x}{charset:04x}\\{field}"
            ));
            if unsafe { VerQueryValueW(data.as_ptr().cast(), q.as_ptr(), &mut ptr, &mut length) }
                != 0
                && length > 1
            {
                let s =
                    unsafe { std::slice::from_raw_parts(ptr.cast::<u16>(), length as usize - 1) };
                if let Some(v) = normalize_version(&String::from_utf16_lossy(s)) {
                    return Some(v);
                }
            }
        }
    }
    None
}
#[cfg(not(windows))]
pub fn exe_version(_: &Path) -> Option<String> {
    None
}

fn cert_der(pem: &str) -> Result<Vec<u8>> {
    use base64::Engine;
    let s = pem
        .split("-----BEGIN CERTIFICATE-----")
        .nth(1)
        .and_then(|s| s.split("-----END CERTIFICATE-----").next())
        .context("Invitation has no public game certificate")?;
    let der = base64::engine::general_purpose::STANDARD
        .decode(s.split_whitespace().collect::<String>())?;
    if der.is_empty() || der.len() > 1024 * 1024 {
        bail!("Invalid public certificate");
    }
    Ok(der)
}
fn hosts_file() -> Result<std::path::PathBuf> {
    Ok(std::path::PathBuf::from(
        std::env::var_os("SystemRoot").context("Windows folder unavailable")?,
    )
    .join("System32/drivers/etc/hosts"))
}
pub fn mapping_state(text: &str) -> Result<bool> {
    let mut found = false;
    for line in text.lines() {
        let mut fields = line.split('#').next().unwrap_or("").split_whitespace();
        let Some(address) = fields.next() else {
            continue;
        };
        if fields.any(|f| f.eq_ignore_ascii_case("trinity.actual.battle.net")) {
            if address != "127.0.0.1" {
                bail!(
                    "Another server uses the game hostname. Existing hosts entries were preserved."
                );
            }
            found = true;
        }
    }
    Ok(found)
}
#[cfg(windows)]
pub fn setup_ready(pem: &str) -> bool {
    let Ok(der) = cert_der(pem) else {
        return false;
    };
    let mapped = hosts_file()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| mapping_state(&s).ok())
        .unwrap_or(false);
    mapped && certificate_installed(&der)
}
#[cfg(windows)]
fn certificate_installed(der: &[u8]) -> bool {
    use windows_sys::Win32::Security::Cryptography::*;
    let root = wide("ROOT");
    let store = unsafe {
        CertOpenStore(
            CERT_STORE_PROV_SYSTEM_W,
            0,
            0,
            CERT_SYSTEM_STORE_LOCAL_MACHINE | CERT_STORE_READONLY_FLAG,
            root.as_ptr().cast(),
        )
    };
    if store.is_null() {
        return false;
    }
    let mut context = std::ptr::null_mut();
    let mut found = false;
    loop {
        context = unsafe { CertEnumCertificatesInStore(store, context) };
        if context.is_null() {
            break;
        }
        let bytes = unsafe {
            std::slice::from_raw_parts((*context).pbCertEncoded, (*context).cbCertEncoded as usize)
        };
        if bytes == der {
            found = true;
            unsafe {
                CertFreeCertificateContext(context);
            }
            break;
        }
    }
    unsafe {
        CertCloseStore(store, 0);
    }
    found
}
#[cfg(not(windows))]
pub fn setup_ready(_: &str) -> bool {
    false
}

#[cfg(windows)]
pub fn apply_setup(public_cert: &Path) -> Result<()> {
    use windows_sys::Win32::Security::Cryptography::*;
    let der = cert_der(&std::fs::read_to_string(public_cert)?)?;
    let hosts = hosts_file()?;
    let old = std::fs::read_to_string(&hosts)?;
    let mapped = mapping_state(&old)?;
    let root = wide("ROOT");
    let store = unsafe {
        CertOpenStore(
            CERT_STORE_PROV_SYSTEM_W,
            0,
            0,
            CERT_SYSTEM_STORE_LOCAL_MACHINE,
            root.as_ptr().cast(),
        )
    };
    if store.is_null() {
        bail!("Administrator approval is required for one-time Windows setup");
    }
    let ok = unsafe {
        CertAddEncodedCertificateToStore(
            store,
            X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
            der.as_ptr(),
            der.len() as u32,
            CERT_STORE_ADD_REPLACE_EXISTING,
            std::ptr::null_mut(),
        )
    };
    unsafe {
        CertCloseStore(store, 0);
    }
    if ok == 0 {
        bail!("Windows could not install the private server's public certificate");
    }
    if !mapped {
        use std::io::Write;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        std::fs::copy(
            &hosts,
            hosts.with_file_name(format!("hosts.wow-friends-before-{stamp}")),
        )?;
        std::fs::OpenOptions::new()
            .append(true)
            .open(hosts)?
            .write_all(b"\r\n127.0.0.1 trinity.actual.battle.net # WoW Friends\r\n")?;
    }
    let system_root = std::env::var_os("SystemRoot").context("Windows folder unavailable")?;
    let mut flush = std::process::Command::new(
        std::path::PathBuf::from(system_root).join("System32/ipconfig.exe"),
    );
    flush.arg("/flushdns");
    hidden(&mut flush);
    if !flush.output()?.status.success() {
        bail!("Windows setup was saved but DNS cache refresh failed; retry Windows setup");
    }
    Ok(())
}
#[cfg(not(windows))]
pub fn apply_setup(_: &Path) -> Result<()> {
    bail!("One-time setup currently supports Windows")
}
#[cfg(windows)]
pub fn request_setup(pem: &str, root: &Path) -> Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    cert_der(pem)?;
    let ca = root.join("game-ca.pem");
    crate::config::save_atomic(&ca, pem.as_bytes())?;
    let exe = wide(std::env::current_exe()?);
    let verb = wide("runas");
    let args = wide(format!("--setup \"{}\"", ca.display()));
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            exe.as_ptr(),
            args.as_ptr(),
            std::ptr::null(),
            1,
        )
    } as isize;
    if code <= 32 {
        bail!("Windows setup was not approved");
    }
    Ok(())
}
#[cfg(not(windows))]
pub fn request_setup(_: &str, _: &Path) -> Result<()> {
    bail!("This release targets Windows")
}

pub fn hidden(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}
#[cfg(windows)]
pub fn game_process_status(helper: Option<u32>) -> (bool, bool, u64) {
    use windows_sys::Win32::{
        Foundation::*,
        System::{Diagnostics::ToolHelp::*, ProcessStatus::*, Threading::*},
        UI::WindowsAndMessaging::*,
    };
    let Some(helper) = helper else {
        return (false, false, 0);
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return (false, false, 0);
    }
    let mut row: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    row.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut ok = unsafe { Process32FirstW(snapshot, &mut row) };
    let mut pid = 0;
    while ok != 0 {
        let end = row
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(row.szExeFile.len());
        if row.th32ParentProcessID == helper
            && String::from_utf16_lossy(&row.szExeFile[..end]).eq_ignore_ascii_case("Wow.exe")
        {
            pid = row.th32ProcessID;
            break;
        }
        ok = unsafe { Process32NextW(snapshot, &mut row) };
    }
    unsafe {
        CloseHandle(snapshot);
    }
    if pid == 0 {
        return (false, false, 0);
    }
    struct WindowState {
        pid: u32,
        responsive: bool,
    }
    unsafe extern "system" fn inspect(hwnd: HWND, data: LPARAM) -> i32 {
        let state = unsafe { &mut *(data as *mut WindowState) };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if pid == state.pid && unsafe { IsWindowVisible(hwnd) } != 0 {
            state.responsive = unsafe { IsHungAppWindow(hwnd) } == 0;
            return 0;
        }
        1
    }
    let mut state = WindowState {
        pid,
        responsive: false,
    };
    unsafe {
        EnumWindows(Some(inspect), &mut state as *mut _ as LPARAM);
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
    let mut mib = 0;
    if !handle.is_null() {
        let mut mem: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        mem.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if unsafe { GetProcessMemoryInfo(handle, &mut mem, mem.cb) } != 0 {
            mib = mem.WorkingSetSize as u64 / 1024 / 1024;
        }
        unsafe {
            CloseHandle(handle);
        }
    }
    (true, state.responsive, mib)
}
#[cfg(not(windows))]
pub fn game_process_status(_: Option<u32>) -> (bool, bool, u64) {
    (false, false, 0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_version_strings() {
        assert_eq!(
            normalize_version("Version 1.60.1.70938").unwrap(),
            "1.60.1.70938"
        );
    }
    #[test]
    fn does_not_override_other_hosts() {
        assert!(mapping_state("10.0.0.2 trinity.actual.battle.net").is_err());
        assert!(mapping_state("127.0.0.1 trinity.actual.battle.net # own server").unwrap());
        assert!(!mapping_state("# 127.0.0.1 trinity.actual.battle.net").unwrap());
    }
}
