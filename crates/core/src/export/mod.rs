//! Esportazione del documento nei formati supportati.

mod docx;
mod html;
mod pdf;
mod txt;

use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::document::Document;

pub use pdf::find_system_fonts;

/// Formati di esportazione.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Txt,
    Html,
    Pdf,
    Docx,
}

impl ExportFormat {
    /// Tutti i formati, nell'ordine in cui vengono proposti.
    pub const ALL: [ExportFormat; 4] = [Self::Docx, Self::Pdf, Self::Html, Self::Txt];

    /// Estensione del file, senza punto.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Txt => "txt",
            Self::Html => "html",
            Self::Pdf => "pdf",
            Self::Docx => "docx",
        }
    }

    /// Descrizione in italiano, adatta a menu e caselle combinate.
    pub fn label(self) -> &'static str {
        match self {
            Self::Txt => "Testo semplice (TXT)",
            Self::Html => "Pagina web (HTML)",
            Self::Pdf => "PDF accessibile (PDF)",
            Self::Docx => "Documento Word (DOCX)",
        }
    }

    /// Riconosce il formato dall'estensione di un percorso.
    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(|e| e.parse().ok())
    }
}

impl fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

impl FromStr for ExportFormat {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().trim_start_matches('.').to_ascii_lowercase().as_str() {
            "txt" | "text" | "testo" => Ok(Self::Txt),
            "html" | "htm" => Ok(Self::Html),
            "pdf" => Ok(Self::Pdf),
            "docx" | "word" => Ok(Self::Docx),
            other => bail!("Formato «{other}» non supportato. Formati validi: txt, html, pdf, docx."),
        }
    }
}

/// Genera il contenuto del file nel formato richiesto.
pub fn render(doc: &Document, format: ExportFormat) -> Result<Vec<u8>> {
    match format {
        ExportFormat::Txt => Ok(txt::render(doc).into_bytes()),
        ExportFormat::Html => Ok(html::render(doc).into_bytes()),
        ExportFormat::Pdf => pdf::render(doc),
        ExportFormat::Docx => docx::render(doc),
    }
}

/// Esporta il documento in un file. Il file viene scritto solo se la generazione riesce.
pub fn export(doc: &Document, format: ExportFormat, path: &Path) -> Result<()> {
    let bytes = render(doc, format)
        .with_context(|| format!("Impossibile generare il file {}.", format.extension().to_uppercase()))?;
    fs::write(path, bytes)
        .with_context(|| format!("Impossibile scrivere il file «{}».", path.display()))
}

/// Sostituisce i caratteri speciali di XML/HTML ed elimina i caratteri di controllo non ammessi.
pub(crate) fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentInfo;

    pub(crate) fn sample_document() -> Document {
        Document::from_text(
            "Questa è la prima frase, con caratteri speciali: <tag> & \"virgolette\".\n\nSeconda riga: perché città è così.",
            &DocumentInfo {
                audio_path: Some(Path::new("intervista.mp3")),
                duration_ms: Some(90_000),
                language: Some("it"),
                model_name: Some("small"),
                translated: false,
            },
        )
    }

    #[test]
    fn parse_formats() {
        assert_eq!("PDF".parse::<ExportFormat>().unwrap(), ExportFormat::Pdf);
        assert_eq!(".docx".parse::<ExportFormat>().unwrap(), ExportFormat::Docx);
        assert!("odt".parse::<ExportFormat>().is_err());
        assert_eq!(
            ExportFormat::from_path(Path::new("a/b.HTML")),
            Some(ExportFormat::Html)
        );
    }

    #[test]
    fn escaping() {
        assert_eq!(escape_xml("a<b>&\"c'\u{1}"), "a&lt;b&gt;&amp;&quot;c&#39;");
    }

    #[test]
    fn every_format_renders_and_writes() {
        let doc = sample_document();
        let dir = tempfile::tempdir().unwrap();
        for format in ExportFormat::ALL {
            if format == ExportFormat::Pdf && find_system_fonts().is_err() {
                continue;
            }
            let path = dir.path().join(format!("prova.{}", format.extension()));
            export(&doc, format, &path).unwrap();
            assert!(fs::metadata(&path).unwrap().len() > 100, "{format} troppo piccolo");
        }
    }
}
