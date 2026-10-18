use embed_manifest::manifest::DpiAwareness;
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    // Manifest: controlli comuni v6 (stili visivi), DPI per monitor, codifica UTF-8.
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_manifest(new_manifest("Trascrittore").dpi_awareness(DpiAwareness::PerMonitorV2))
            .expect("impossibile incorporare il manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
