//! Trascrittore: interfaccia grafica accessibile (Win32 nativo).
//!
//! Tutti gli elementi sono controlli standard di Windows, quindi NVDA, JAWS e Assistente
//! vocale li riconoscono senza adattamenti.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod a11y;
#[cfg(windows)]
mod app;
#[cfg(windows)]
mod dialogs;
#[cfg(windows)]
mod win;

trascrittore_core::vulkan_delay_load_hook!();

#[cfg(windows)]
fn main() {
    std::panic::set_hook(Box::new(|info| {
        win::error_box(None, &format!("Errore interno imprevisto:\n{info}"));
    }));
    let initial_file = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    if let Err(e) = app::run(initial_file) {
        win::error_box(None, &format!("Impossibile avviare Trascrittore:\n{e:#}"));
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("L'interfaccia grafica è disponibile solo su Windows. Usa «trascrittore-cli».");
    std::process::exit(1);
}
