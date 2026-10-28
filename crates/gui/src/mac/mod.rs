//! Interfaccia grafica per macOS (AppKit nativo).
//!
//! Ogni elemento è un vero controllo di AppKit (pulsante, menu a comparsa, campo di testo,
//! indicatore di avanzamento), quindi VoiceOver ne legge nome, ruolo, stato e valore senza
//! adattamenti. I cambi di stato vengono annunciati con le notifiche di accessibilità di
//! macOS, senza spostare il focus.

mod app;
mod check;
mod menu;
mod ui;

use std::ffi::OsString;
use std::path::PathBuf;

use objc2::runtime::ProtocolObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

/// Opzioni della riga di comando.
#[derive(Debug, Default)]
pub struct Options {
    /// File audio da aprire all'avvio.
    pub audio_file: Option<PathBuf>,
    /// Verifica automatica dell'accessibilità (vedi [`check`]).
    pub check: Option<check::CheckOptions>,
}

impl Options {
    /// Riconosce `[--verifica-accessibilita [--istantanea FILE.png] [--modello ID|FILE.bin]]
    /// [FILE_AUDIO]`.
    pub fn from_args(args: impl Iterator<Item = OsString>) -> Result<Self, String> {
        let mut options = Options::default();
        let mut check = None::<check::CheckOptions>;
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--verifica-accessibilita") => {
                    check.get_or_insert_with(Default::default);
                }
                Some("--istantanea") => {
                    let path = args.next().ok_or("Manca il file dopo --istantanea.")?;
                    check.get_or_insert_with(Default::default).snapshot = Some(path.into());
                }
                Some("--modello") => {
                    let id = args.next().ok_or("Manca il modello dopo --modello.")?;
                    check.get_or_insert_with(Default::default).model =
                        Some(id.to_string_lossy().into_owned());
                }
                // Argomento aggiunto da vecchie versioni di macOS all'avvio dal Finder.
                Some(a) if a.starts_with("-psn_") => {}
                Some(a) if a.starts_with("--") => {
                    return Err(format!(
                        "Opzione sconosciuta: {a}\nUso: trascrittore [--verifica-accessibilita [--istantanea FILE.png] [--modello ID]] [FILE_AUDIO]"
                    ));
                }
                _ => {
                    let path = PathBuf::from(arg);
                    if path.is_file() {
                        options.audio_file = Some(path);
                    }
                }
            }
        }
        options.check = check;
        Ok(options)
    }
}

/// Avvia l'applicazione e il suo ciclo di eventi (non ritorna).
pub fn run(options: Options) {
    let mtm = MainThreadMarker::new().expect("l'interfaccia va avviata sul thread principale");
    std::panic::set_hook(Box::new(|info| {
        eprintln!("Errore interno imprevisto: {info}");
    }));
    let app = NSApplication::sharedApplication(mtm);
    // Necessario quando il programma è avviato direttamente e non dal pacchetto .app.
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    let controller = app::Controller::new(mtm, options);
    app.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
    app.run();
    // `run` ritorna solo dopo `stop:`; il delegato deve restare vivo fino ad allora.
    drop(controller);
}
