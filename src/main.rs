#![cfg_attr(windows, windows_subsystem = "windows")]
mod builds;
mod config;
mod diagnostics;
mod game;
mod platform;
mod recovery;
mod relay;
mod service;
mod ui;
mod updates;
use anyhow::{Context, Result};
use std::{fs::OpenOptions, path::PathBuf, sync::Arc};

fn main() {
    if let Err(e) = run() {
        eprintln!("{e:#}");
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let text: Vec<u16> = std::ffi::OsStr::new(&e.to_string())
                .encode_wide()
                .chain(Some(0))
                .collect();
            let title: Vec<u16> = "WoW Friends Launcher"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    std::ptr::null_mut(),
                    text.as_ptr(),
                    title.as_ptr(),
                    0,
                );
            }
        }
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let root = config::data_dir()?;
    if args.first().is_some_and(|s| s == "--setup") {
        platform::apply_setup(&PathBuf::from(
            args.get(1).context("Missing public certificate path")?,
        ))?;
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "--import") {
        let path = PathBuf::from(args.get(1).context("Missing private invitation")?);
        let (i, s) = config::import_invite(&path)?;
        config::store_invite(&root, &i)?;
        if let Some(s) = s {
            config::save_settings(&root, &s)?;
        }
        println!("Private invitation imported; credentials were not printed.");
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let rt = Arc::new(tokio::runtime::Runtime::new()?);
    if args.first().is_some_and(|s| s == "--support-once") {
        let settings = config::load_settings(&root);
        anyhow::ensure!(
            settings.support_diagnostics && settings.support_consent_version == 1,
            "Support sharing is not enabled"
        );
        let invite = config::load_invite(&root)?;
        let preflight = settings
            .game
            .as_ref()
            .map(|p| {
                builds::inspect(p)
                    .and_then(|i| builds::compatibility(&i, &invite.expected_builds()))
                    .map_err(|e| e.to_string())
            })
            .map(|r| format!("Compatibility check: {r:?}"))
            .unwrap_or_else(|| "No game selected".into());
        let report = diagnostics::collect(
            &settings,
            &root,
            "rust-manual-support-check",
            None,
            &preflight,
        );
        rt.block_on(async { diagnostics::upload(&service::client()?, &invite, &report).await })?;
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "--doctor") {
        let s = config::load_settings(&root);
        let invite = config::load_invite(&root).ok();
        let install = s
            .game
            .as_ref()
            .map(|p| builds::inspect(p).map_err(|e| e.to_string()));
        let status = invite.as_ref().map(|i| {
            rt.block_on(async { service::status(&service::client()?, i).await })
                .map_err(|e: anyhow::Error| e.to_string())
        });
        let accepted = status
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|s| s.accepted_builds.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| invite.as_ref().map(|i| i.expected_builds()))
            .unwrap_or_default();
        let compatible = install
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|i| builds::compatibility(i, &accepted).map_err(|e| e.to_string()));
        let data = serde_json::json!({"launcher":env!("CARGO_PKG_VERSION"),"invitationConfigured":invite.is_some(),"supportOptIn":s.support_diagnostics,"installation":install,"server":status,"compatibility":compatible,"windowsSetupReady":invite.as_ref().is_some_and(|i|platform::setup_ready(&i.ca_pem))});
        let out = serde_json::to_vec_pretty(&data)?;
        if let Some(index) = args.iter().position(|s| s == "--output") {
            std::fs::write(args.get(index + 1).context("Missing output file")?, &out)?;
        } else {
            println!("{}", String::from_utf8_lossy(&out));
        }
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "--transport-test") {
        let i = config::load_invite(&root)?;
        rt.block_on(async {
            use tokio::io::AsyncReadExt;
            let relay = relay::Relay::start(i).await?;
            let mut socket = tokio::net::TcpStream::connect("127.0.0.1:18085").await?;
            let mut b = [0u8; 256];
            let n = tokio::time::timeout(std::time::Duration::from_secs(15), socket.read(&mut b))
                .await??;
            anyhow::ensure!(n > 0, "No world handshake received");
            drop(socket);
            drop(relay);
            Ok::<_, anyhow::Error>(())
        })?;
        let data = b"Rust HTTPS/WebSocket game transport received the native world handshake.";
        if let Some(index) = args.iter().position(|s| s == "--output") {
            std::fs::write(args.get(index + 1).context("Missing output file")?, data)?;
        } else {
            println!("{}", String::from_utf8_lossy(data));
        }
        return Ok(());
    }
    let instance = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("launcher.lock"))?;
    let mut locked = false;
    for _ in 0..100 {
        if fs2::FileExt::try_lock_exclusive(&instance).is_ok() {
            locked = true;
            break;
        }
        if !args.iter().any(|s| s == "--updated") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    anyhow::ensure!(
        locked,
        "WoW Friends Launcher is already open. Use its existing window."
    );
    ui::run(root, rt).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(())
}
