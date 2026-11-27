use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=packaging/collector.ico");
    println!("cargo:rerun-if-changed=packaging/collector.rc");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let target = env::var("TARGET").expect("TARGET");
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let resource = root.join("packaging");
    let mut defines = vec![
        format!(
            "VER_MAJOR={}",
            env::var("CARGO_PKG_VERSION_MAJOR").expect("major")
        ),
        format!(
            "VER_MINOR={}",
            env::var("CARGO_PKG_VERSION_MINOR").expect("minor")
        ),
        format!(
            "VER_PATCH={}",
            env::var("CARGO_PKG_VERSION_PATCH").expect("patch")
        ),
    ];
    if env::var_os("CARGO_FEATURE_INTERNAL_TEST").is_some() {
        defines.push("INTERNAL_TEST=1".to_owned());
    }

    if target.ends_with("-gnu") {
        let object = out.join("collector-icon.o");
        let tool = if target.starts_with("x86_64") {
            "x86_64-w64-mingw32-windres"
        } else {
            "i686-w64-mingw32-windres"
        };
        let status = Command::new(tool)
            .current_dir(&resource)
            .args(defines.iter().map(|d| format!("-D{d}")))
            .args(["-i", "collector.rc", "-O", "coff", "-o"])
            .arg(&object)
            .status()
            .expect("windres");
        assert!(status.success(), "icon resource compilation failed");
        println!(
            "cargo:rustc-link-arg-bin=live-match-collector={}",
            object.display()
        );
    } else {
        let output = out.join("collector-icon.res");
        let status = Command::new("rc.exe")
            .current_dir(&resource)
            .arg("/nologo")
            .args(defines.iter().map(|d| format!("/d{}", d)))
            .arg(format!("/fo{}", output.display()))
            .arg("collector.rc")
            .status()
            .expect("rc.exe");
        assert!(status.success(), "icon resource compilation failed");
        println!(
            "cargo:rustc-link-arg-bin=live-match-collector={}",
            output.display()
        );
    }
    let _ = fs::metadata(resource.join("collector.ico")).expect("icon");
}
