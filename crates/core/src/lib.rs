//! Nucleo di Trascrittore: decodifica audio, trascrizione con Whisper (whisper.cpp),
//! gestione dei modelli ed esportazione in TXT, HTML, PDF e DOCX.
//!
//! Il nucleo non dipende da alcuna interfaccia grafica: è usato sia dalla CLI sia dalla GUI.

pub mod audio;
pub mod device;
pub mod document;
pub mod export;
pub mod languages;
pub mod models;
pub mod openvino;
pub mod paths;
#[cfg(feature = "whisper")]
pub mod pipeline;
pub mod settings;
#[cfg(feature = "whisper")]
pub mod transcriber;
pub mod transcript;
pub mod vulkan;

/// Nome dell'applicazione mostrato nei documenti e nei metadati.
pub const APP_NAME: &str = "Trascrittore";
/// Versione dell'applicazione.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Errore restituito quando l'utente annulla un'operazione.
///
/// Si riconosce con `err.is::<Cancelled>()` su un [`anyhow::Error`], così l'annullamento
/// non viene presentato come un errore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Operazione annullata.")]
pub struct Cancelled;

/// Restituisce `true` se l'errore indica un annullamento richiesto dall'utente.
pub fn is_cancelled(err: &anyhow::Error) -> bool {
    err.is::<Cancelled>()
}
