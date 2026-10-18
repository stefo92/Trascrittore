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
    /// Modalità rapida: ricerca greedy invece della beam search (più veloce, meno precisa).
    pub fast: bool,
}

/// Opzioni di caricamento del modello.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EngineOptions {
    /// Flash Attention. Disattivata per impostazione predefinita: misurata su AMD Radeon
    /// RX 6800 XT (Vulkan) rallenta del 30% sulla GPU e del 50% sulla CPU, ma su altre schede
    /// video può accelerare.
    pub flash_attention: bool,
}

/// Modello Whisper caricato in memoria, riutilizzabile per più trascrizioni.
pub struct Transcriber {
    context: WhisperContext,
    model_path: PathBuf,
    /// Dispositivo richiesto.
    requested: ResolvedDevice,
    /// Dispositivo effettivamente in uso (la CPU se la scheda video non si è attivata).
    effective: ResolvedDevice,
    engine: EngineOptions,
    gpu_warning: Option<String>,
}

static LOGGING: Once = Once::new();

/// Scarta i messaggi di whisper.cpp e ggml invece di stamparli su stderr
/// (senza un backend di log configurato non vengono mostrati da nessuna parte).
pub fn silence_engine_logs() {
    LOGGING.call_once(whisper_rs::install_logging_hooks);
}

impl Transcriber {
    /// Carica un modello GGML sul dispositivo indicato.
    ///
    /// Se è richiesta una scheda video, controlla dalla memoria video che il modello vi sia
    /// stato davvero caricato: whisper.cpp altrimenti ripiega sulla CPU senza segnalarlo.
    pub fn load(model_path: &Path, device: &ResolvedDevice, engine: EngineOptions) -> Result<Self> {
        silence_engine_logs();
        crate::models::validate_model_file(model_path)?;
        let model_size = std::fs::metadata(model_path).map(|m| m.len()).unwrap_or(0);

        let mut params = WhisperContextParameters::default();
        params.flash_attn = engine.flash_attention;
        let mut memory_before = None;
        match device {
            ResolvedDevice::Cpu => params.use_gpu = false,
            ResolvedDevice::Gpu(gpu) => {
                params.use_gpu = true;
                params.gpu_device = i32::try_from(gpu.index).unwrap_or(0);
                memory_before = crate::device::memory(gpu.index);
            }
        }
        let context = WhisperContext::new_with_params(model_path, params).map_err(|e| {
            anyhow!(
                "Impossibile caricare il modello «{}» su {}: {e}",
                model_path.display(),
                device.label()
            )
        })?;

        let mut effective = device.clone();
        let mut gpu_warning = None;
        if let ResolvedDevice::Gpu(gpu) = device {
            let memory_after = crate::device::memory(gpu.index);
            if !gpu_memory_used(memory_before, memory_after, model_size) {
                effective = ResolvedDevice::Cpu;
                gpu_warning = Some(format!(
                    "La scheda video {} non si è attivata: la trascrizione usa il processore. Controlla i driver della scheda video.",
                    gpu.description
                ));
            }
        }

        Ok(Self {
            context,
            model_path: model_path.to_path_buf(),
            requested: device.clone(),
            effective,
            engine,
            gpu_warning,
        })
    }

    /// Percorso del modello caricato.
    pub fn model_path(&self) -> &Path {
        &self.model_path
    }

    /// Dispositivo richiesto al caricamento.
    pub fn requested_device(&self) -> &ResolvedDevice {
        &self.requested
    }

    /// Dispositivo effettivamente in uso.
    pub fn device(&self) -> &ResolvedDevice {
        &self.effective
    }

    /// Opzioni con cui è stato caricato il modello.
    pub fn engine_options(&self) -> EngineOptions {
        self.engine
    }

    /// Avviso se la scheda video richiesta non è stata usata.
    pub fn gpu_warning(&self) -> Option<&str> {
        self.gpu_warning.as_deref()
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

        let strategy = if options.fast {
            SamplingStrategy::Greedy { best_of: 1 }
        } else {
            SamplingStrategy::BeamSearch {
                beam_size: 5,
                patience: -1.0,
            }
        };
        let mut params = FullParams::new(strategy);
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

/// Decide se il caricamento ha occupato memoria video.
///
/// `memory` è (libera, totale) in byte. Se il driver non riporta la memoria libera
/// (libera = totale sia prima sia dopo) non si può verificare e si assume che la GPU sia in uso.
fn gpu_memory_used(before: Option<(u64, u64)>, after: Option<(u64, u64)>, model_size: u64) -> bool {
    match (before, after) {
        (Some((free_before, total)), Some((free_after, _))) => {
            if free_before == total && free_after == total {
                return true;
            }
            free_before.saturating_sub(free_after) >= model_size / 2
        }
        _ => true,
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

#[cfg(test)]
mod tests {
    use super::gpu_memory_used;

    const GB: u64 = 1 << 30;

    #[test]
    fn detects_gpu_memory_usage() {
        let model = GB / 2;
        // 600 MB occupati: modello caricato sulla scheda video.
        assert!(gpu_memory_used(Some((10 * GB, 16 * GB)), Some((10 * GB - (600 << 20), 16 * GB)), model));
        // Nessuna variazione: whisper.cpp è ripiegato sulla CPU.
        assert!(!gpu_memory_used(Some((10 * GB, 16 * GB)), Some((10 * GB, 16 * GB)), model));
        // Il driver non riporta la memoria libera: non verificabile.
        assert!(gpu_memory_used(Some((16 * GB, 16 * GB)), Some((16 * GB, 16 * GB)), model));
        assert!(gpu_memory_used(None, None, model));
    }
}
