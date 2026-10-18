//! Esportazione in testo semplice UTF-8.

use crate::document::{Document, BODY_HEADING, INFO_HEADING};

#[cfg(windows)]
const NEWLINE: &str = "\r\n";
#[cfg(not(windows))]
const NEWLINE: &str = "\n";

pub(crate) fn render(doc: &Document) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(doc.title.clone());
    lines.push("=".repeat(doc.title.chars().count()));
    lines.push(String::new());

    if !doc.metadata.is_empty() {
        lines.push(INFO_HEADING.to_string());
        lines.push("-".repeat(INFO_HEADING.chars().count()));
        for (key, value) in &doc.metadata {
            lines.push(format!("{key}: {value}"));
        }
        lines.push(String::new());
    }

    lines.push(BODY_HEADING.to_string());
    lines.push("-".repeat(BODY_HEADING.chars().count()));
    for paragraph in &doc.paragraphs {
        lines.push(paragraph.clone());
        lines.push(String::new());
    }

    let mut text = lines.join(NEWLINE);
    let trimmed_len = text.trim_end().len();
    text.truncate(trimmed_len);
    text.push_str(NEWLINE);
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::tests::sample_document;

    #[test]
    fn contains_title_metadata_and_text() {
        let text = render(&sample_document());
        assert!(text.starts_with("Trascrizione di intervista"));
        assert!(text.contains("File audio: intervista.mp3"));
        assert!(text.contains("<tag> & \"virgolette\""));
        assert!(text.contains("perché città è così."));
    }
}
