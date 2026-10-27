fn main() {
    // vulkan-1.dll a caricamento ritardato: senza driver Vulkan il programma parte comunque e
    // usa il processore (vedi trascrittore_core::vulkan). /IGNORE:4199 evita l'avviso nelle
    // build senza Vulkan, dove la DLL non è importata.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DELAYLOAD:vulkan-1.dll");
        println!("cargo:rustc-link-arg-bins=/IGNORE:4199");
        println!("cargo:rustc-link-arg-bins=delayimp.lib");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
