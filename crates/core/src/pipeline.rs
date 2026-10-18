//! Flusso completo «file audio → trascrizione», condiviso da CLI e GUI.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;

use crate::audio;
use crate::device::{self, Device, ResolvedDevice};
use crate::transcriber::{TranscribeOptions, Transcriber};
use crate::transcript::Transcript;

/// Avanzamento di una trascrizione.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// Decodifica del file audio (0.0–1.0).
    Decoding(f32),
    /// Caricamento del modello in memoria sul dispositivo indicato.
    LoadingModel(ResolvedDevice),
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
    pub device: Device,
    pub options: TranscribeOptions,
}

/// Esito di una trascrizione con il dispositivo effettivamente usato.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub transcript: Transcript,
    pub device: ResolvedDevice,
}

/// Esegue decodifica e trascrizione.
///
/// `cache` conserva il modello caricato fra una chiamata e l'altra: se modello e dispositivo
/// richiesti sono già in memoria non vengono ricaricati.
pub fn run(
    job: &Job,
    cache: &mut Option<Transcriber>,
    progress: ProgressFn,
    cancel: Arc<AtomicBool>,
) -> Result<Outcome> {
    let device = job.device.resolve(device::list_gpus())?;

    let report = Arc::clone(&progress);
    let audio = audio::load_for_whisper(
        &job.audio_path,
        move |fraction| report(Progress::Decoding(fraction)),
        &cancel,
    )?;

    let transcriber = load_cached(cache, &job.model_path, &device, &progress)?;

    let report = Arc::clone(&progress);
    report(Progress::Transcribing(0));
    let mut transcript = transcriber.transcribe(
        &audio.samples,
        &job.options,
        move |percent| report(Progress::Transcribing(percent.clamp(0, 100))),
        cancel,
    )?;
    transcript.audio_duration_ms = audio.duration_ms();
    Ok(Outcome { transcript, device })
}

fn load_cached<'a>(
    cache: &'a mut Option<Transcriber>,
    model_path: &Path,
    device: &ResolvedDevice,
    progress: &ProgressFn,
) -> Result<&'a Transcriber> {
    let reload = cache
        .as_ref()
        .is_none_or(|t| t.model_path() != model_path || t.device() != device);
    if reload {
        // Libera prima il modello precedente: i modelli grandi occupano gigabyte.
        *cache = None;
        progress(Progress::LoadingModel(device.clone()));
        *cache = Some(Transcriber::load(model_path, device)?);
    }
    Ok(cache.as_ref().expect("modello appena caricato"))
}
