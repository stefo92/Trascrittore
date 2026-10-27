//! Flusso completo «file audio → trascrizione», condiviso da CLI e GUI.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;

use crate::audio;
use crate::device::{self, Device, ResolvedDevice};
use crate::transcriber::{EngineOptions, OpenVinoUse, TranscribeOptions, Transcriber};
use crate::transcript::Transcript;

/// Avanzamento di una trascrizione.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// Decodifica del file audio (0.0–1.0).
    Decoding(f32),
    /// Caricamento del modello in memoria sul dispositivo indicato.
    LoadingModel(ResolvedDevice),
    /// Trascrizione in corso (0–100) sul dispositivo effettivamente in uso.
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
    pub engine: EngineOptions,
    pub options: TranscribeOptions,
}

/// Esito di una trascrizione con il dispositivo effettivamente usato.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub transcript: Transcript,
    pub device: ResolvedDevice,
    /// Presente se la scheda video richiesta non è stata usata.
    pub gpu_warning: Option<String>,
    /// Presente se era richiesto l'encoder OpenVINO.
    pub openvino: Option<OpenVinoUse>,
}

/// Esegue decodifica e trascrizione.
///
/// `cache` conserva il modello caricato fra una chiamata e l'altra: se modello, dispositivo
/// e opzioni del motore sono gli stessi non viene ricaricato.
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

    let transcriber = load_cached(cache, &job.model_path, &device, job.engine, &progress)?;

    let report = Arc::clone(&progress);
    report(Progress::Transcribing(0));
    let transcription = transcriber.transcribe(
        &audio.samples,
        &job.options,
        move |percent| report(Progress::Transcribing(percent.clamp(0, 100))),
        cancel,
    )?;
    let mut transcript = transcription.transcript;
    transcript.audio_duration_ms = audio.duration_ms();
    Ok(Outcome {
        transcript,
        device: transcriber.device().clone(),
        gpu_warning: transcriber.gpu_warning().map(str::to_string),
        openvino: transcription.openvino,
    })
}

fn load_cached<'a>(
    cache: &'a mut Option<Transcriber>,
    model_path: &Path,
    device: &ResolvedDevice,
    engine: EngineOptions,
    progress: &ProgressFn,
) -> Result<&'a Transcriber> {
    let reload = cache.as_ref().is_none_or(|t| {
        t.model_path() != model_path
            || t.requested_device() != device
            || t.engine_options() != engine
    });
    if reload {
        // Libera prima il modello precedente: i modelli grandi occupano gigabyte.
        *cache = None;
        progress(Progress::LoadingModel(device.clone()));
        *cache = Some(Transcriber::load(model_path, device, engine)?);
    }
    Ok(cache.as_ref().expect("modello appena caricato"))
}
