use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        link_clang_runtime();
    }
    tauri_build::build()
}

/// ONNX Runtime's CoreML objects (in ort's prebuilt) call `__isPlatformVersionAtLeast`,
/// which lives in clang's runtime library. rustc links with `-nodefaultlibs`, so that
/// library is not linked unless we ask for it; with release LTO the link fails without it.
fn link_clang_runtime() {
    let dir = Command::new("clang")
        .arg("--print-resource-dir")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok());
    match dir {
        Some(dir) => {
            println!("cargo:rustc-link-search=native={}/lib/darwin", dir.trim());
            println!("cargo:rustc-link-lib=static=clang_rt.osx");
        }
        None => {
            println!("cargo:warning=clang --print-resource-dir failed; not linking clang_rt.osx")
        }
    }
}
