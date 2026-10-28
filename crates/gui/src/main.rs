//! Trascrittore: interfaccia grafica accessibile.
//!
//! Su Windows tutti gli elementi sono controlli Win32 standard, quindi NVDA, JAWS e Assistente
//! vocale li riconoscono senza adattamenti; su macOS sono controlli AppKit nativi, letti da
//! VoiceOver allo stesso modo.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod a11y;
#[cfg(windows)]
mod app;
#[cfg(windows)]
mod dialogs;
#[cfg(target_os = "macos")]
mod mac;
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

#[cfg(target_os = "macos")]
fn main() {
    let options = match mac::Options::from_args(std::env::args_os().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    mac::run(options);
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!(
        "L'interfaccia grafica è disponibile solo su Windows e macOS. Usa «trascrittore-cli»."
    );
    std::process::exit(1);
}
