//! Risultato di una trascrizione e sua conversione in testo leggibile.

use serde::{Deserialize, Serialize};

/// Pausa minima tra due segmenti che fa iniziare un nuovo paragrafo.
const PARAGRAPH_PAUSE_MS: u64 = 2_000;
/// Lunghezza oltre la quale un paragrafo viene chiuso alla prima fine di frase.
const PARAGRAPH_SOFT_LIMIT: usize = 600;

/// Un frammento di parlato con il suo intervallo temporale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Esito completo di una trascrizione.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub segments: Vec<Segment>,
    /// Codice ISO 639-1 della lingua usata o rilevata (es. `"it"`).
    pub language: Option<String>,
    /// Durata dell'audio elaborato.
    pub audio_duration_ms: u64,
}

impl Transcript {
    /// Converte la trascrizione in testo.
    ///
    /// - Con `timestamps`, una riga per segmento preceduta dall'orario di inizio: `[00:01:23] testo`.
    /// - Senza, i segmenti vengono uniti in paragrafi separati da una riga vuota; un nuovo
    ///   paragrafo inizia dopo una pausa lunga o quando il paragrafo diventa troppo lungo.
    pub fn to_text(&self, timestamps: bool) -> String {
        if timestamps {
            return self
                .segments
                .iter()
                .filter(|s| !s.text.trim().is_empty())
                .map(|s| format!("[{}] {}", format_timestamp(s.start_ms), s.text.trim()))
                .collect::<Vec<_>>()
                .join("\n");
        }

        let mut paragraphs: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut previous_end: Option<u64> = None;

        for segment in &self.segments {
            let text = segment.text.trim();
            if text.is_empty() {
                continue;
            }
            let pause = previous_end.map_or(0, |end| segment.start_ms.saturating_sub(end));
            let ends_sentence = current
                .trim_end()
                .ends_with(['.', '!', '?', '…', '"', '»']);
            let too_long = current.chars().count() >= PARAGRAPH_SOFT_LIMIT && ends_sentence;
            if !current.is_empty() && (pause >= PARAGRAPH_PAUSE_MS || too_long) {
                paragraphs.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(text);
            previous_end = Some(segment.end_ms);
        }
        if !current.is_empty() {
            paragraphs.push(current);
        }
        paragraphs.join("\n\n")
    }

    /// Numero di parole trascritte (approssimato).
    pub fn word_count(&self) -> usize {
        self.segments
            .iter()
            .map(|s| s.text.split_whitespace().count())
            .sum()
    }
}

/// Formatta millisecondi come `hh:mm:ss`.
pub fn format_timestamp(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

/// Formatta una durata in italiano leggibile, adatta anche alla sintesi vocale
/// (es. "1 ora, 2 minuti e 5 secondi").
pub fn format_duration_words(ms: u64) -> String {
    let total_seconds = (ms + 500) / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    let plural = |n: u64, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };

    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(plural(hours, "ora", "ore"));
    }
    if minutes > 0 {
        parts.push(plural(minutes, "minuto", "minuti"));
    }
    if seconds > 0 || parts.is_empty() {
        parts.push(plural(seconds, "secondo", "secondi"));
    }
    match parts.len() {
        1 => parts.remove(0),
        _ => {
            let last = parts.pop().unwrap_or_default();
            format!("{} e {last}", parts.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start_ms: u64, end_ms: u64, text: &str) -> Segment {
        Segment {
            start_ms,
            end_ms,
            text: text.to_string(),
        }
    }

    #[test]
    fn timestamp_format() {
        assert_eq!(format_timestamp(0), "00:00:00");
        assert_eq!(format_timestamp(61_500), "00:01:01");
        assert_eq!(format_timestamp(3_723_000), "01:02:03");
    }

    #[test]
    fn duration_in_words() {
        assert_eq!(format_duration_words(0), "0 secondi");
        assert_eq!(format_duration_words(1_000), "1 secondo");
        assert_eq!(format_duration_words(65_000), "1 minuto e 5 secondi");
        assert_eq!(format_duration_words(3_725_000), "1 ora, 2 minuti e 5 secondi");
        assert_eq!(format_duration_words(7_200_000), "2 ore");
    }

    #[test]
    fn text_with_timestamps() {
        let t = Transcript {
            segments: vec![seg(0, 1000, " Ciao."), seg(61_000, 62_000, "Mondo. ")],
            ..Default::default()
        };
        assert_eq!(t.to_text(true), "[00:00:00] Ciao.\n[00:01:01] Mondo.");
    }

    #[test]
    fn paragraphs_split_on_long_pause() {
        let t = Transcript {
            segments: vec![
                seg(0, 1000, "Prima frase."),
                seg(1100, 2000, "Seconda frase."),
                seg(5000, 6000, "Dopo una pausa."),
            ],
            ..Default::default()
        };
        assert_eq!(
            t.to_text(false),
            "Prima frase. Seconda frase.\n\nDopo una pausa."
        );
        assert_eq!(t.word_count(), 7);
    }
}
