//! Catalogo dei modelli Whisper ufficiali (formato GGML di whisper.cpp) e loro download.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

use crate::Cancelled;

/// Indirizzo base del repository ufficiale dei modelli di whisper.cpp.
const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// Variabile d'ambiente che sostituisce la cartella predefinita dei modelli.
pub const MODELS_DIR_ENV: &str = "TRASCRITTORE_MODELS_DIR";

/// Un modello Whisper scaricabile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelInfo {
    /// Identificativo breve (es. `small`).
    pub id: &'static str,
    /// Descrizione in italiano.
    pub description: &'static str,
    /// Dimensione approssimativa del file in byte.
    pub size_bytes: u64,
}

/// Modello proposto di default: ottimo compromesso tra qualità e velocità.
pub const DEFAULT_MODEL_ID: &str = "large-v3-turbo-q5_0";

/// Modelli ufficiali, dal più leggero al più accurato.
///
/// Le varianti «compresse» (quantizzate q5) occupano circa un terzo dello spazio, si caricano
/// prima e sono più veloci, con una perdita di qualità minima.
pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "tiny-q5_1",
        description: "Tiny compresso: il più leggero e veloce, qualità bassa",
        size_bytes: 32_152_673,
    },
    ModelInfo {
        id: "tiny",
        description: "Tiny: velocissimo, qualità bassa",
        size_bytes: 77_691_713,
    },
    ModelInfo {
        id: "base-q5_1",
        description: "Base compresso: molto veloce, qualità discreta",
        size_bytes: 59_707_625,
    },
    ModelInfo {
        id: "base",
        description: "Base: veloce, qualità discreta",
        size_bytes: 147_951_465,
    },
    ModelInfo {
        id: "small-q5_1",
        description: "Small compresso: veloce, buona qualità",
        size_bytes: 190_085_487,
    },
    ModelInfo {
        id: "small",
        description: "Small: buon equilibrio tra velocità e qualità",
        size_bytes: 487_601_967,
    },
    ModelInfo {
        id: "medium-q5_0",
        description: "Medium compresso: qualità alta",
        size_bytes: 539_212_467,
    },
    ModelInfo {
        id: "medium",
        description: "Medium: qualità alta, lento",
        size_bytes: 1_533_763_059,
    },
    ModelInfo {
        id: "large-v3-turbo-q5_0",
        description: "Large v3 Turbo compresso: qualità ottima, consigliato",
        size_bytes: 574_041_195,
    },
    ModelInfo {
        id: "large-v3-turbo",
        description: "Large v3 Turbo: qualità ottima",
        size_bytes: 1_624_555_275,
    },
    ModelInfo {
        id: "large-v3",
        description: "Large v3: qualità massima, molto lento",
        size_bytes: 3_095_033_483,
    },
];

impl ModelInfo {
    /// Nome del file del modello (es. `ggml-small.bin`).
    pub fn file_name(&self) -> String {
        format!("ggml-{}.bin", self.id)
    }

    /// Indirizzo di download.
    pub fn url(&self) -> String {
        format!("{BASE_URL}/{}", self.file_name())
    }

    /// Percorso del modello nella cartella dei modelli.
    pub fn path(&self) -> Result<PathBuf> {
        Ok(models_dir()?.join(self.file_name()))
    }

    /// `true` se il modello è già stato scaricato.
    pub fn is_downloaded(&self) -> bool {
        self.path().map(|p| p.is_file()).unwrap_or(false)
    }
}

/// Cerca un modello del catalogo per identificativo.
pub fn find(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id.eq_ignore_ascii_case(id))
}

/// Cartella in cui sono conservati i modelli (creata se manca).
///
/// In ordine di priorità: la variabile d'ambiente [`MODELS_DIR_ENV`]; in modalità portable
/// `dati\modelli` accanto al programma (vedi [`crate::paths`]); altrimenti, su Windows,
/// `%LOCALAPPDATA%\Trascrittore\data\models` (Linux: `~/.local/share/trascrittore/models`;
/// macOS: `~/Library/Application Support/Trascrittore/models`).
pub fn models_dir() -> Result<PathBuf> {
    models_dir_for(std::env::var_os(MODELS_DIR_ENV), crate::paths::portable_data_dir())
}

/// Come [`models_dir`], con il valore della variabile d'ambiente e la cartella portable indicati.
fn models_dir_for(env: Option<OsString>, portable: Option<&Path>) -> Result<PathBuf> {
    let dir = match env {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => crate::paths::default_models_dir_for(portable)?,
    };
    fs::create_dir_all(&dir).with_context(|| {
        format!(
            "Impossibile creare la cartella dei modelli «{}».",
            dir.display()
        )
    })?;
    Ok(dir)
}

/// Formatta una dimensione in byte in italiano (es. "574 MB", "1,6 GB").
pub fn format_size(bytes: u64) -> String {
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB).replace('.', ",")
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else {
        format!("{:.0} kB", b / 1000.0)
    }
}

/// Scarica un modello nella cartella dei modelli e ne restituisce il percorso.
///
/// Il file viene scritto come `.part` e rinominato solo a download completato e verificato.
/// `progress` riceve i byte scaricati e, se noto, il totale.
pub fn download(
    model: &ModelInfo,
    progress: impl FnMut(u64, Option<u64>),
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    let dest = model.path()?;
    download_url(&model.url(), &dest, progress, cancel)?;
    Ok(dest)
}

/// Scarica `url` in `dest`, passando per un file `.part` rinominato solo a download
/// completato. Usata anche per i componenti di OpenVINO.
pub fn download_url(
    url: &str,
    dest: &Path,
    mut progress: impl FnMut(u64, Option<u64>),
    cancel: &AtomicBool,
) -> Result<()> {
    let mut partial = dest.as_os_str().to_owned();
    partial.push(".part");
    let partial = PathBuf::from(partial);

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(60)))
        .user_agent(format!("{}/{}", crate::APP_NAME, crate::APP_VERSION))
        .build()
        .into();

    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| anyhow!("Download non riuscito: {e}. Controlla la connessione a Internet."))?;
    let total = response.body().content_length();

    let mut reader = response.body_mut().as_reader();
    let mut file = File::create(&partial)
        .with_context(|| format!("Impossibile creare il file «{}».", partial.display()))?;

    let mut buffer = vec![0u8; 256 * 1024];
    let mut downloaded = 0u64;
    let result: Result<()> = (|| {
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Cancelled.into());
            }
            let n = reader
                .read(&mut buffer)
                .map_err(|e| anyhow!("Download interrotto: {e}."))?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n])
                .context("Errore di scrittura su disco: controlla lo spazio libero.")?;
            downloaded += n as u64;
            progress(downloaded, total);
        }
        file.flush()?;
        if let Some(total) = total {
            if downloaded != total {
                bail!("Download incompleto: ricevuti {downloaded} byte su {total}.");
            }
        }
        if downloaded < 1_000_000 {
            bail!("Il file scaricato è troppo piccolo: il server non ha inviato il file atteso.");
        }
        Ok(())
    })();

    drop(file);
    if let Err(e) = result {
        let _ = fs::remove_file(&partial);
        return Err(e);
    }
    fs::rename(&partial, dest)
        .with_context(|| format!("Impossibile salvare il file in «{}».", dest.display()))
}

/// Verifica che un file esista e abbia l'aspetto di un modello GGML.
pub fn validate_model_file(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("Il file modello «{}» non esiste.", path.display()))?;
    if !metadata.is_file() || metadata.len() < 1_000_000 {
        bail!("Il file «{}» non è un modello Whisper valido.", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_default() {
        assert!(find(DEFAULT_MODEL_ID).is_some());
        assert_eq!(
            find("small").unwrap().url(),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"
        );
    }

    #[test]
    fn lightest_model_comes_first() {
        let lightest = MODELS.iter().min_by_key(|m| m.size_bytes).unwrap();
        assert_eq!(lightest.id, MODELS[0].id);
        assert_eq!(MODELS[0].file_name(), "ggml-tiny-q5_1.bin");
    }

    #[test]
    fn sizes_in_italian() {
        assert_eq!(format_size(574_000_000), "574 MB");
        assert_eq!(format_size(1_620_000_000), "1,6 GB");
    }

    #[test]
    fn portable_models_dir_is_created_in_data_folder() {
        let exe_dir = tempfile::tempdir().unwrap();
        fs::write(exe_dir.path().join(crate::paths::PORTABLE_MARKER), "").unwrap();
        let data = crate::paths::portable_data_dir_in(exe_dir.path()).unwrap();

        let dir = models_dir_for(None, Some(&data)).unwrap();
        assert_eq!(dir, exe_dir.path().join("dati").join("modelli"));
        assert!(dir.is_dir());
        // Una variabile vuota equivale a nessuna variabile.
        assert_eq!(models_dir_for(Some(OsString::new()), Some(&data)).unwrap(), dir);
    }

    #[test]
    fn env_variable_wins_over_portable_mode() {
        let exe_dir = tempfile::tempdir().unwrap();
        let data = exe_dir.path().join("dati");
        let custom = exe_dir.path().join("altri-modelli");
        let dir = models_dir_for(Some(custom.clone().into_os_string()), Some(&data)).unwrap();
        assert_eq!(dir, custom);
        assert!(dir.is_dir());
        assert!(!data.exists());
    }
}
