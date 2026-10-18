//! Flusso completo «file audio → trascrizione», condiviso da CLI e GUI.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;

use crate::audio;
use crate::transcriber::{TranscribeOptions, Transcriber};
use crate::transcript::Transcript;

/// Avanzamento di una trascrizione.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Progress {
    /// Decodifica del file audio (0.0–1.0).
    Decoding(f32),
    /// Caricamento del modello in memoria.
    LoadingModel,
    /// Trascrizione in corso (0–100).
    Transcribing(i32),
}

/// Funzione di notifica dell'avanzamento, chiamabile da più thread.
pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

/// Richiesta di trascrizione.
#[derive(Debug, Clone)]
pub struct Job {
    pub audio_path: PathBuf,
    pub model_path: PathBuf,
    pub options: TranscribeOptions,
}

/// Esegue decodifica e trascrizione.
///
/// `cache` conserva il modello caricato fra una chiamata e l'altra: se il modello richiesto
/// è già in memoria non viene ricaricato.
pub fn run(
    job: &Job,
    cache: &mut Option<Transcriber>,
    progress: ProgressFn,
    cancel: Arc<AtomicBool>,
) -> Result<Transcript> {
    let report = Arc::clone(&progress);
    let audio = audio::load_for_whisper(
        &job.audio_path,
        move |fraction| report(Progress::Decoding(fraction)),
        &cancel,
    )?;

    let transcriber = load_cached(cache, &job.model_path, &progress)?;

    let report = Arc::clone(&progress);
    report(Progress::Transcribing(0));
    let mut transcript = transcriber.transcribe(
        &audio.samples,
        &job.options,
        move |percent| report(Progress::Transcribing(percent.clamp(0, 100))),
        cancel,
    )?;
    transcript.audio_duration_ms = audio.duration_ms();
    Ok(transcript)
}

fn load_cached<'a>(
    cache: &'a mut Option<Transcriber>,
    model_path: &Path,
    progress: &ProgressFn,
) -> Result<&'a Transcriber> {
    let reload = cache
        .as_ref()
        .is_none_or(|t| t.model_path() != model_path);
    if reload {
        // Libera prima il modello precedente: i modelli grandi occupano gigabyte.
        *cache = None;
        progress(Progress::LoadingModel);
        *cache = Some(Transcriber::load(model_path)?);
    }
    Ok(cache.as_ref().expect("modello appena caricato"))
}
