use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=vendor/connector.cpp");
    println!("cargo:rerun-if-env-changed=CONNECTOR_EXE_PATH");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let exe = out.join("wow-connector.exe");
    if let Some(source) = env::var_os("CONNECTOR_EXE_PATH") {
        std::fs::copy(source, &exe).expect("copy supplied connector");
    } else {
        let compiler = cc::Build::new().cpp(true).get_compiler();
        assert!(compiler.is_like_msvc(), "Windows MSVC is the tested target");
        let mut cmd: Command = compiler.to_command();
        cmd.args([
            "/nologo",
            "/std:c++17",
            "/EHsc",
            "/O2",
            "/DUNICODE",
            "/D_UNICODE",
            "vendor/connector.cpp",
        ])
        .arg(format!("/Fe{}", exe.display()))
        .arg(format!("/Fo{}", out.join("connector.obj").display()))
        .args(["/link", "crypt32.lib", "ws2_32.lib"]);
        assert!(
            cmd.status().expect("run connector compiler").success(),
            "connector build failed"
        );
    }
    println!("cargo:rustc-env=CONNECTOR_BINARY={}", exe.display());
}
