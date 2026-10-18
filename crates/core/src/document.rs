//! Modello neutro del documento da esportare, indipendente dal formato.

use std::path::Path;

use chrono::{DateTime, Local};

use crate::languages;
use crate::transcript::format_duration_words;

/// Intestazione della sezione dei metadati.
pub const INFO_HEADING: &str = "Informazioni";
/// Intestazione della sezione con il testo trascritto.
pub const BODY_HEADING: &str = "Testo trascritto";

/// Documento pronto per l'esportazione.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Titolo (intestazione di primo livello e titolo dei metadati del file).
    pub title: String,
    /// Lingua dell'interfaccia del documento (etichette, titoli), es. `it-IT`.
    pub locale: String,
    /// Lingua del testo trascritto, es. `en-US`.
    pub content_locale: String,
    /// Coppie etichetta/valore mostrate nella sezione «Informazioni».
    pub metadata: Vec<(String, String)>,
    /// Paragrafi del testo trascritto.
    pub paragraphs: Vec<String>,
    /// Data di creazione.
    pub created: DateTime<Local>,
}

/// Dati descrittivi di una trascrizione, usati per comporre il documento.
#[derive(Debug, Clone, Default)]
pub struct DocumentInfo<'a> {
    pub audio_path: Option<&'a Path>,
    pub duration_ms: Option<u64>,
    /// Codice ISO 639-1 della lingua del testo.
    pub language: Option<&'a str>,
    pub model_name: Option<&'a str>,
    pub translated: bool,
}

impl Document {
    /// Crea un documento a partire dal testo (eventualmente modificato dall'utente).
    ///
    /// Ogni riga non vuota del testo diventa un paragrafo.
    pub fn from_text(text: &str, info: &DocumentInfo<'_>) -> Self {
        let created = Local::now();
        let stem = info
            .audio_path
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned());
        let title = match &stem {
            Some(stem) => format!("Trascrizione di {stem}"),
            None => "Trascrizione".to_string(),
        };

        let mut metadata = Vec::new();
        if let Some(name) = info.audio_path.and_then(|p| p.file_name()) {
            metadata.push(("File audio".to_string(), name.to_string_lossy().into_owned()));
        }
        if let Some(ms) = info.duration_ms {
            metadata.push(("Durata".to_string(), format_duration_words(ms)));
        }
        if let Some(code) = info.language {
            let mut value = languages::display_name(code);
            if info.translated {
                value.push_str(" (tradotto)");
            }
            metadata.push(("Lingua".to_string(), value));
        }
        if let Some(model) = info.model_name {
            metadata.push(("Modello".to_string(), model.to_string()));
        }
        metadata.push((
            "Data".to_string(),
            created.format("%d/%m/%Y %H:%M").to_string(),
        ));

        Self {
            title,
            locale: "it-IT".to_string(),
            content_locale: languages::locale_for(info.language.unwrap_or("it")),
            metadata,
            paragraphs: paragraphs_from_text(text),
            created,
        }
    }
}

/// Divide il testo in paragrafi: ogni riga non vuota è un paragrafo.
pub fn paragraphs_from_text(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_title_and_metadata() {
        let path = Path::new("C:/registrazioni/lezione 1.mp3");
        let doc = Document::from_text(
            "Primo paragrafo.\n\nSecondo paragrafo.\r\n",
            &DocumentInfo {
                audio_path: Some(path),
                duration_ms: Some(65_000),
                language: Some("en"),
                model_name: Some("small"),
                translated: false,
            },
        );
        assert_eq!(doc.title, "Trascrizione di lezione 1");
        assert_eq!(doc.paragraphs, vec!["Primo paragrafo.", "Secondo paragrafo."]);
        assert_eq!(doc.content_locale, "en-US");
        assert_eq!(doc.metadata[0], ("File audio".into(), "lezione 1.mp3".into()));
        assert_eq!(doc.metadata[1], ("Durata".into(), "1 minuto e 5 secondi".into()));
        assert_eq!(doc.metadata[2], ("Lingua".into(), "Inglese".into()));
    }
}
