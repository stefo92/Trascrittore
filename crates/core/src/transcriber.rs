//! Trascrizione con whisper.cpp tramite `whisper-rs`.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};

use anyhow::{anyhow, Result};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::device::ResolvedDevice;
use crate::transcript::{Segment, Transcript};
use crate::Cancelled;

/// Opzioni di trascrizione.
#[derive(Debug, Clone, Default)]
pub struct TranscribeOptions {
    /// Codice ISO 639-1 della lingua parlata; `None` = rilevamento automatico.
    pub language: Option<String>,
    /// Traduce in inglese invece di trascrivere.
    pub translate: bool,
    /// Numero di thread; `None` = scelta automatica.
    pub threads: Option<usize>,
    /// Testo iniziale che orienta lo stile e il vocabolario (nomi propri, sigle…).
    pub initial_prompt: Option<String>,
}

/// Modello Whisper caricato in memoria, riutilizzabile per più trascrizioni.
pub struct Transcriber {
    context: WhisperContext,
    model_path: PathBuf,
    device: ResolvedDevice,
}

static LOGGING: Once = Once::new();

/// Scarta i messaggi di whisper.cpp e ggml invece di stamparli su stderr
/// (senza un backend di log configurato non vengono mostrati da nessuna parte).
pub fn silence_engine_logs() {
    LOGGING.call_once(whisper_rs::install_logging_hooks);
}

impl Transcriber {
    /// Carica un modello GGML sul dispositivo indicato.
    pub fn load(model_path: &Path, device: &ResolvedDevice) -> Result<Self> {
        silence_engine_logs();
        crate::models::validate_model_file(model_path)?;
        let mut params = WhisperContextParameters::default();
        match device {
            ResolvedDevice::Cpu => params.use_gpu = false,
            ResolvedDevice::Gpu(gpu) => {
                params.use_gpu = true;
                params.gpu_device = i32::try_from(gpu.index).unwrap_or(0);
            }
        }
        let context = WhisperContext::new_with_params(model_path, params).map_err(|e| {
            anyhow!(
                "Impossibile caricare il modello «{}» su {}: {e}",
                model_path.display(),
                device.label()
            )
        })?;
        Ok(Self {
            context,
            model_path: model_path.to_path_buf(),
            device: device.clone(),
        })
    }

    /// Percorso del modello caricato.
    pub fn model_path(&self) -> &Path {
        &self.model_path
    }

    /// Dispositivo su cui è caricato il modello.
    pub fn device(&self) -> &ResolvedDevice {
        &self.device
    }

    /// `true` se il modello supporta più lingue (i modelli `.en` solo l'inglese).
    pub fn is_multilingual(&self) -> bool {
        self.context.is_multilingual()
    }

    /// Trascrive campioni mono a 16 kHz.
    ///
    /// `progress` riceve la percentuale (0–100); se `cancel` diventa `true` la trascrizione
    /// si interrompe e viene restituito [`Cancelled`].
    pub fn transcribe(
        &self,
        samples: &[f32],
        options: &TranscribeOptions,
        progress: impl FnMut(i32) + 'static,
        cancel: Arc<AtomicBool>,
    ) -> Result<Transcript> {
        let mut state = self
            .context
            .create_state()
            .map_err(|e| anyhow!("Impossibile preparare il modello: {e}"))?;

        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: -1.0,
        });
        let threads = options.threads.unwrap_or_else(default_threads).max(1);
        params.set_n_threads(i32::try_from(threads).unwrap_or(4));
        params.set_translate(options.translate);
        let language = options
            .language
            .as_deref()
            .filter(|l| !l.is_empty())
            .unwrap_or("auto");
        params.set_language(Some(language));
        if let Some(prompt) = options.initial_prompt.as_deref().filter(|p| !p.is_empty()) {
            params.set_initial_prompt(prompt);
        }
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_progress_callback_safe(progress);
        // Non si usa `set_abort_callback_safe`: in whisper-rs 0.16.0 il suo trampolino
        // interpreta male il puntatore alla closure e interrompe l'encoder («failed to encode»).
        // `cancel` resta vivo per tutta la durata di `full`, quindi il puntatore è valido.
        unsafe {
            params.set_abort_callback(Some(abort_trampoline));
            params.set_abort_callback_user_data(Arc::as_ptr(&cancel) as *mut c_void);
        }

        let result = state.full(params, samples);
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        result.map_err(|e| anyhow!("La trascrizione non è riuscita: {e}"))?;

        let mut segments = Vec::new();
        for segment in state.as_iter() {
            let text = segment
                .to_str_lossy()
                .map_err(|e| anyhow!("Testo del segmento non leggibile: {e}"))?;
            segments.push(Segment {
                // whisper.cpp esprime i tempi in centesimi di secondo.
                start_ms: u64::try_from(segment.start_timestamp()).unwrap_or(0) * 10,
                end_ms: u64::try_from(segment.end_timestamp()).unwrap_or(0) * 10,
                text: text.trim().to_string(),
            });
        }

        let detected = whisper_rs::get_lang_str(state.full_lang_id_from_state()).map(str::to_string);
        let language = if options.translate {
            Some("en".to_string())
        } else {
            detected.or_else(|| options.language.clone())
        };

        Ok(Transcript {
            segments,
            language,
            audio_duration_ms: samples.len() as u64 * 1000
                / u64::from(crate::audio::WHISPER_SAMPLE_RATE),
        })
    }
}

/// Richiamata da whisper.cpp durante il calcolo: `true` interrompe la trascrizione.
unsafe extern "C" fn abort_trampoline(data: *mut c_void) -> bool {
    if data.is_null() {
        return false;
    }
    // SAFETY: `data` punta all'`AtomicBool` di `cancel`, vivo per tutta la chiamata a `full`.
    unsafe { (*(data as *const AtomicBool)).load(Ordering::Relaxed) }
}

/// Versione di whisper.cpp inclusa.
pub fn engine_version() -> &'static str {
    whisper_rs::WHISPER_CPP_VERSION
}

/// `true` se Whisper riconosce il codice di lingua (es. `it`, `en`).
pub fn is_supported_language(code: &str) -> bool {
    whisper_rs::get_lang_id(code).is_some()
}

/// Numero di thread predefinito: whisper.cpp non migliora oltre 8 thread circa.
pub fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8)
}
