use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Su macOS il codice Objective-C di ggml (backend Metal) usa `@available`, che clang traduce
    // in chiamate a `__isPlatformVersionAtLeast` del proprio runtime (libclang_rt.osx.a). Rust
    // collega gli eseguibili con -nodefaultlibs, quindi la libreria va indicata esplicitamente;
    // in una libreria la direttiva vale per tutti gli eseguibili e i test che la usano.
    let macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    if macos && std::env::var_os("CARGO_FEATURE_WHISPER").is_some() {
        if let Some(dir) = clang_darwin_runtime_dir() {
            println!("cargo:rustc-link-search=native={}", dir.display());
            println!("cargo:rustc-link-lib=static=clang_rt.osx");
        } else {
            println!(
                "cargo:warning=libclang_rt.osx.a non trovata: il collegamento con Metal può fallire"
            );
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CC");
}

/// Cartella con le librerie di runtime di clang per macOS (`<resource-dir>/lib/darwin`).
fn clang_darwin_runtime_dir() -> Option<PathBuf> {
    let cc = std::env::var("CC").unwrap_or_else(|_| "clang".to_string());
    let output = Command::new(cc).arg("--print-resource-dir").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let resource_dir = String::from_utf8(output.stdout).ok()?;
    let dir = PathBuf::from(resource_dir.trim()).join("lib").join("darwin");
    dir.join("libclang_rt.osx.a").is_file().then_some(dir)
}
