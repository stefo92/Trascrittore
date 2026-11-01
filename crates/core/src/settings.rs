//! Preferenze dell'utente, salvate in JSON nella cartella di configurazione
//! (o nella cartella `dati` in modalità portable).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::device::Device;
use crate::export::ExportFormat;

/// Preferenze persistenti condivise da GUI e CLI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Modello del catalogo selezionato.
    pub model_id: String,
    /// Modello personalizzato (ha la precedenza su `model_id`).
    pub custom_model_path: Option<PathBuf>,
    /// Lingua del parlato; `None` = rilevamento automatico.
    pub language: Option<String>,
    pub translate: bool,
    pub timestamps: bool,
    /// Processore o scheda video.
    pub device: Device,
    /// Flash Attention (disattivata per impostazione predefinita, vedi `EngineOptions`).
    pub flash_attention: bool,
    /// Modalità rapida: decodifica greedy, più veloce sulla GPU ma meno precisa.
    pub fast_mode: bool,
    /// Dispositivo OpenVINO per l'encoder (es. `NPU`); `None` = non usare OpenVINO.
    pub openvino_device: Option<String>,
    /// Encoder su Core ML (Neural Engine dei Mac Apple Silicon), dove è disponibile.
    pub neural_engine: bool,
    pub export_format: ExportFormat,
    pub last_audio_dir: Option<PathBuf>,
    pub last_export_dir: Option<PathBuf>,
    /// Dimensione del carattere del testo trascritto, in punti.
    pub transcript_font_points: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model_id: crate::models::DEFAULT_MODEL_ID.to_string(),
            custom_model_path: None,
            language: Some("it".to_string()),
            translate: false,
            timestamps: false,
            device: Device::Auto,
            flash_attention: false,
            fast_mode: false,
            openvino_device: None,
            neural_engine: true,
            export_format: ExportFormat::Docx,
            last_audio_dir: None,
            last_export_dir: None,
            transcript_font_points: 12,
        }
    }
}

impl Settings {
    /// Percorso del file delle preferenze: nel profilo utente o, in modalità portable,
    /// nella cartella `dati` accanto al programma (vedi [`crate::paths`]).
    pub fn path() -> Result<PathBuf> {
        crate::paths::settings_file()
    }

    /// Carica le preferenze; in caso di file mancante o non valido usa i valori predefiniti.
    pub fn load() -> Self {
        Self::path().map(|p| Self::load_from(&p)).unwrap_or_default()
    }

    /// Carica le preferenze dal file indicato.
    pub fn load_from(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Salva le preferenze.
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path()?)
    }

    /// Salva le preferenze nel file indicato, creando le cartelle mancanti.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let context = || format!("Impossibile salvare le preferenze in «{}».", path.display());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(context)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json).with_context(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths;

    #[test]
    fn saved_in_portable_data_folder() {
        let exe_dir = tempfile::tempdir().unwrap();
        fs::write(exe_dir.path().join(paths::PORTABLE_MARKER), "").unwrap();
        let data = paths::portable_data_dir_in(exe_dir.path()).unwrap();
        let file = paths::settings_file_for(Some(&data)).unwrap();

        let settings = Settings {
            device: Device::Cpu,
            fast_mode: true,
            transcript_font_points: 16,
            ..Settings::default()
        };
        settings.save_to(&file).unwrap();

        // La cartella «dati» viene creata al primo salvataggio.
        assert_eq!(file, exe_dir.path().join("dati").join("impostazioni.json"));
        assert!(file.is_file());
        assert_eq!(Settings::load_from(&file), settings);
    }

    #[test]
    fn missing_or_invalid_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("impostazioni.json");
        assert_eq!(Settings::load_from(&file), Settings::default());
        fs::write(&file, "non è json").unwrap();
        assert_eq!(Settings::load_from(&file), Settings::default());
    }
}
