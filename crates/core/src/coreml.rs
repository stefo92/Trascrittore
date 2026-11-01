//! Encoder di Whisper su Core ML: sui Mac Apple Silicon la parte più pesante del calcolo
//! (l'encoder) può girare sul Neural Engine.
//!
//! whisper.cpp, compilato con la feature `coreml`, cerca accanto al modello la cartella
//! `ggml-<famiglia>-encoder.mlmodelc` (il modello Core ML compilato) e, se la trova, la usa
//! al posto dell'encoder di ggml; il decoder resta su Metal o sul processore. Core ML decide
//! da solo se eseguire l'encoder sul Neural Engine, sulla scheda video o sul processore. Senza
//! la cartella whisper.cpp usa l'encoder normale.
//!
//! Gli encoder già convertiti sono pubblicati nel repository ufficiale di whisper.cpp su
//! Hugging Face, uno per famiglia di modelli (valgono anche per le varianti compresse).

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

/// Repository ufficiale di whisper.cpp, lo stesso dei modelli.
const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// File presente in ogni modello Core ML compilato.
const COMPILED_MODEL_MARKER: &str = "coremldata.bin";

/// Archivio ufficiale con l'encoder Core ML di una famiglia di modelli.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderPackage {
    /// Famiglia del modello (es. `base`): vale anche per le varianti compresse.
    pub family: &'static str,
    /// Dimensione dell'archivio zip.
    pub size_bytes: u64,
    /// Impronta SHA-256 dell'archivio zip (dal repository su Hugging Face).
    pub sha256: &'static str,
}

/// Encoder Core ML pubblicati per le famiglie del catalogo dei modelli.
pub const ENCODER_PACKAGES: &[EncoderPackage] = &[
    EncoderPackage {
        family: "tiny",
        size_bytes: 15_037_446,
        sha256: "c88cbd2648e1f5415092bcf5256add463a0f19943e6938f46e8d4ffdebd47739",
    },
    EncoderPackage {
        family: "base",
        size_bytes: 37_922_638,
        sha256: "7e6ab77041942572f239b5b602f8aaa1c3ed29d73e3d8f20abea03a773541089",
    },
    EncoderPackage {
        family: "small",
        size_bytes: 163_083_239,
        sha256: "de43fb9fed471e95c19e60ae67575c2bf09e8fb607016da171b06ddad313988b",
    },
    EncoderPackage {
        family: "medium",
        size_bytes: 567_829_413,
        sha256: "79b0b8d436d47d3f24dd3afc91f19447dd686a4f37521b2f6d9c30a642133fbd",
    },
    EncoderPackage {
        family: "large-v3",
        size_bytes: 1_175_711_232,
        sha256: "47837be7594a29429ec08620043390c4d6d467f8bd362df09e9390ace76a55a4",
    },
    EncoderPackage {
        family: "large-v3-turbo",
        size_bytes: 1_173_393_014,
        sha256: "84bedfe895bd7b5de6e8e89a0803dfc5addf8c0c5bc4c937451716bf7cf7988a",
    },
];

impl EncoderPackage {
    /// Nome della cartella dell'encoder (`ggml-base-encoder.mlmodelc`).
    pub fn encoder_dir_name(&self) -> String {
        format!("ggml-{}-encoder.mlmodelc", self.family)
    }

    /// Indirizzo dell'archivio.
    pub fn url(&self) -> String {
        format!("{BASE_URL}/{}.zip", self.encoder_dir_name())
    }
}

/// `true` se questa versione del programma include Core ML.
pub fn compiled() -> bool {
    cfg!(feature = "coreml")
}

/// Cartella dell'encoder che whisper.cpp cerca per il modello: toglie l'estensione e il
/// suffisso della compressione (`-q5_1`), poi aggiunge `-encoder.mlmodelc`.
pub fn encoder_path(model_path: &Path) -> PathBuf {
    let mut stem = model_path.with_extension("").to_string_lossy().into_owned();
    if let Some(pos) = stem.rfind('-') {
        let suffix = &stem.as_bytes()[pos..];
        if suffix.len() == 5 && suffix[1] == b'q' && suffix[3] == b'_' {
            stem.truncate(pos);
        }
    }
    PathBuf::from(format!("{stem}-encoder.mlmodelc"))
}

/// Encoder Core ML già presente e completo per il modello.
pub fn find_encoder(model_path: &Path) -> Option<PathBuf> {
    let path = encoder_path(model_path);
    path.join(COMPILED_MODEL_MARKER).is_file().then_some(path)
}

/// Archivio ufficiale con l'encoder adatto al modello, se esiste.
pub fn package_for_model(model_path: &Path) -> Option<&'static EncoderPackage> {
    let name = encoder_path(model_path).file_name()?.to_str()?.to_owned();
    ENCODER_PACKAGES.iter().find(|p| p.encoder_dir_name() == name)
}

/// Scarica l'encoder, ne verifica l'impronta e lo estrae nella cartella dei modelli.
pub fn download_encoder(
    package: &EncoderPackage,
    models_dir: &Path,
    progress: impl FnMut(u64, Option<u64>),
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    let zip_path = models_dir.join(format!("{}.zip", package.encoder_dir_name()));
    crate::models::download_url(&package.url(), &zip_path, progress, cancel)?;
    let result = verify_and_extract(package, &zip_path, models_dir);
    let _ = fs::remove_file(&zip_path);
    result
}

fn verify_and_extract(
    package: &EncoderPackage,
    zip_path: &Path,
    models_dir: &Path,
) -> Result<PathBuf> {
    if sha256_file(zip_path)? != package.sha256 {
        bail!(
            "L'encoder Core ML scaricato è danneggiato (impronta SHA-256 diversa da quella attesa). Riprova il download."
        );
    }
    extract(zip_path, models_dir, &package.encoder_dir_name())
}

/// Estrae la cartella `dir_name` dall'archivio in una cartella temporanea, rinominata solo
/// a estrazione completata.
fn extract(zip_path: &Path, models_dir: &Path, dir_name: &str) -> Result<PathBuf> {
    let dest = models_dir.join(dir_name);
    let staging = models_dir.join(format!("{dir_name}.part"));
    let _ = fs::remove_dir_all(&staging);
    let result = (|| -> Result<()> {
        let mut archive = zip::ZipArchive::new(File::open(zip_path)?)
            .context("L'archivio dell'encoder Core ML non è valido.")?;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            // `enclosed_name` scarta i percorsi assoluti o con «..».
            let Some(name) = entry.enclosed_name() else {
                continue;
            };
            let Ok(relative) = name.strip_prefix(dir_name) else {
                continue;
            };
            let target = staging.join(relative);
            if entry.is_dir() {
                fs::create_dir_all(&target)?;
            } else {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut file = File::create(&target)?;
                io::copy(&mut entry, &mut file)?;
            }
        }
        if !staging.join(COMPILED_MODEL_MARKER).is_file() {
            bail!("L'archivio non contiene l'encoder Core ML «{dir_name}».");
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(e).with_context(|| format!("Impossibile estrarre «{dir_name}»."));
    }
    let _ = fs::remove_dir_all(&dest);
    fs::rename(&staging, &dest)
        .with_context(|| format!("Impossibile salvare «{}».", dest.display()))?;
    Ok(dest)
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn encoder_path_follows_whisper_cpp() {
        let dir = Path::new("/modelli");
        assert_eq!(
            encoder_path(&dir.join("ggml-base.bin")),
            dir.join("ggml-base-encoder.mlmodelc")
        );
        // Le varianti compresse usano l'encoder della famiglia.
        assert_eq!(
            encoder_path(&dir.join("ggml-large-v3-turbo-q5_0.bin")),
            dir.join("ggml-large-v3-turbo-encoder.mlmodelc")
        );
        assert_eq!(
            encoder_path(&dir.join("ggml-tiny-q5_1.bin")),
            dir.join("ggml-tiny-encoder.mlmodelc")
        );
    }

    #[test]
    fn every_catalog_model_has_an_encoder() {
        for model in crate::models::MODELS {
            let path = Path::new("/modelli").join(model.file_name());
            assert!(package_for_model(&path).is_some(), "manca l'encoder per {}", model.id);
        }
        assert!(package_for_model(Path::new("/modelli/personale.bin")).is_none());
    }

    #[test]
    fn extracts_compiled_model_folder() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("encoder.zip");
        let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer.add_directory("ggml-base-encoder.mlmodelc/", options).unwrap();
        writer.start_file("ggml-base-encoder.mlmodelc/coremldata.bin", options).unwrap();
        writer.write_all(b"dati").unwrap();
        writer.start_file("ggml-base-encoder.mlmodelc/weights/weight.bin", options).unwrap();
        writer.write_all(b"pesi").unwrap();
        writer.start_file("../fuori.txt", options).unwrap();
        writer.write_all(b"no").unwrap();
        writer.finish().unwrap();

        let dest = extract(&zip_path, dir.path(), "ggml-base-encoder.mlmodelc").unwrap();
        assert_eq!(dest, dir.path().join("ggml-base-encoder.mlmodelc"));
        assert!(dest.join("weights/weight.bin").is_file());
        assert!(!dir.path().join("fuori.txt").exists());
        assert!(!dir.path().join("ggml-base-encoder.mlmodelc.part").exists());
        assert_eq!(find_encoder(&dir.path().join("ggml-base-q5_1.bin")), Some(dest));
    }

    #[test]
    fn rejects_archive_without_encoder() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("vuoto.zip");
        let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        writer.start_file("altro.txt", zip::write::SimpleFileOptions::default()).unwrap();
        writer.finish().unwrap();
        assert!(extract(&zip_path, dir.path(), "ggml-base-encoder.mlmodelc").is_err());
        assert!(!dir.path().join("ggml-base-encoder.mlmodelc").exists());
    }
}
