//! Lingue proposte per la trascrizione, con nome in italiano.

/// Una lingua supportata da Whisper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Language {
    /// Codice ISO 639-1 usato da Whisper.
    pub code: &'static str,
    /// Nome in italiano.
    pub name: &'static str,
    /// Tag di lingua completo per i documenti (es. `it-IT`).
    pub locale: &'static str,
}

const fn lang(code: &'static str, name: &'static str, locale: &'static str) -> Language {
    Language { code, name, locale }
}

/// Lingue più comuni, in ordine di presentazione (l'italiano per primo).
/// Whisper ne supporta altre: la CLI accetta qualunque codice valido.
pub const LANGUAGES: &[Language] = &[
    lang("it", "Italiano", "it-IT"),
    lang("en", "Inglese", "en-US"),
    lang("fr", "Francese", "fr-FR"),
    lang("de", "Tedesco", "de-DE"),
    lang("es", "Spagnolo", "es-ES"),
    lang("pt", "Portoghese", "pt-PT"),
    lang("nl", "Olandese", "nl-NL"),
    lang("ca", "Catalano", "ca-ES"),
    lang("ro", "Rumeno", "ro-RO"),
    lang("pl", "Polacco", "pl-PL"),
    lang("cs", "Ceco", "cs-CZ"),
    lang("sk", "Slovacco", "sk-SK"),
    lang("sl", "Sloveno", "sl-SI"),
    lang("hr", "Croato", "hr-HR"),
    lang("sr", "Serbo", "sr-RS"),
    lang("bs", "Bosniaco", "bs-BA"),
    lang("sq", "Albanese", "sq-AL"),
    lang("hu", "Ungherese", "hu-HU"),
    lang("el", "Greco", "el-GR"),
    lang("bg", "Bulgaro", "bg-BG"),
    lang("ru", "Russo", "ru-RU"),
    lang("uk", "Ucraino", "uk-UA"),
    lang("sv", "Svedese", "sv-SE"),
    lang("da", "Danese", "da-DK"),
    lang("no", "Norvegese", "nb-NO"),
    lang("fi", "Finlandese", "fi-FI"),
    lang("tr", "Turco", "tr-TR"),
    lang("ar", "Arabo", "ar-SA"),
    lang("he", "Ebraico", "he-IL"),
    lang("fa", "Persiano", "fa-IR"),
    lang("hi", "Hindi", "hi-IN"),
    lang("zh", "Cinese", "zh-CN"),
    lang("ja", "Giapponese", "ja-JP"),
    lang("ko", "Coreano", "ko-KR"),
    lang("la", "Latino", "la"),
];

/// Cerca una lingua per codice ISO 639-1.
pub fn find(code: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code))
}

/// Nome italiano della lingua, oppure il codice stesso se non è in elenco.
pub fn display_name(code: &str) -> String {
    find(code).map_or_else(|| code.to_string(), |l| l.name.to_string())
}

/// Tag di lingua completo (es. `it` → `it-IT`), usato nei documenti esportati.
pub fn locale_for(code: &str) -> String {
    find(code).map_or_else(|| code.to_string(), |l| l.locale.to_string())
}
