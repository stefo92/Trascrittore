//! Preferenze dell'utente, salvate in JSON nella cartella di configurazione.

use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

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
            export_format: ExportFormat::Docx,
            last_audio_dir: None,
            last_export_dir: None,
            transcript_font_points: 12,
        }
    }
}

impl Settings {
    /// Percorso del file delle preferenze.
    pub fn path() -> Result<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", crate::APP_NAME)
            .ok_or_else(|| anyhow!("Impossibile determinare la cartella di configurazione."))?;
        Ok(dirs.config_dir().join("impostazioni.json"))
    }

    /// Carica le preferenze; in caso di file mancante o non valido usa i valori predefiniti.
    pub fn load() -> Self {
        Self::path()
            .ok()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Salva le preferenze.
    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&path, json)
            .with_context(|| format!("Impossibile salvare le preferenze in «{}».", path.display()))
    }
}
