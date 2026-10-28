//! Cartelle in cui Trascrittore salva impostazioni e modelli.
//!
//! Normalmente si usa il profilo dell'utente (su Windows `%APPDATA%\Trascrittore\config` e
//! `%LOCALAPPDATA%\Trascrittore\data`). In **modalità portable** tutto resta nella cartella
//! `dati` accanto all'eseguibile, così il programma si può usare da una chiavetta USB senza
//! lasciare file sul computer. La modalità si attiva se accanto all'eseguibile c'è la cartella
//! `dati` oppure il file `portable.txt` (in questo caso `dati` viene creata quando serve).
//!
//! Su macOS l'interfaccia grafica è un pacchetto `Trascrittore.app`, con l'eseguibile in
//! `Trascrittore.app/Contents/MacOS`: la cartella del programma è quella che contiene il
//! pacchetto, così GUI e CLI condividono la stessa cartella `dati` e il pacchetto firmato non
//! viene modificato.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{anyhow, Result};

/// Cartella dei dati della modalità portable, accanto all'eseguibile.
pub const PORTABLE_DATA_DIR: &str = "dati";
/// File che attiva la modalità portable anche quando la cartella `dati` non esiste ancora.
pub const PORTABLE_MARKER: &str = "portable.txt";

const SETTINGS_FILE: &str = "impostazioni.json";
const PORTABLE_MODELS_DIR: &str = "modelli";
const PORTABLE_OPENVINO_DIR: &str = "openvino";

/// Cartella dei dati portable, se il programma è in modalità portable.
pub fn portable_data_dir() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        portable_data_dir_in(program_dir(exe.parent()?))
    })
    .as_deref()
}

/// Cartella del programma per un eseguibile che si trova in `exe_dir`: la cartella stessa
/// oppure, dentro un pacchetto macOS (`Nome.app/Contents/MacOS`), quella che contiene il
/// pacchetto.
pub fn program_dir(exe_dir: &Path) -> &Path {
    let bundle = exe_dir
        .parent()
        .filter(|_| exe_dir.file_name().is_some_and(|n| n == "MacOS"))
        .filter(|contents| contents.file_name().is_some_and(|n| n == "Contents"))
        .and_then(Path::parent)
        .filter(|app| app.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")));
    bundle.and_then(Path::parent).unwrap_or(exe_dir)
}

/// Cartella dei dati portable per un programma che si trova in `exe_dir`, se in quella
/// cartella è attiva la modalità portable.
pub fn portable_data_dir_in(exe_dir: &Path) -> Option<PathBuf> {
    let data = exe_dir.join(PORTABLE_DATA_DIR);
    (data.is_dir() || exe_dir.join(PORTABLE_MARKER).is_file()).then_some(data)
}

/// File delle preferenze.
pub fn settings_file() -> Result<PathBuf> {
    settings_file_for(portable_data_dir())
}

/// File delle preferenze con la cartella portable indicata (`None` = profilo utente).
pub fn settings_file_for(portable: Option<&Path>) -> Result<PathBuf> {
    Ok(match portable {
        Some(data) => data.join(SETTINGS_FILE),
        None => project_dirs()?.config_dir().join(SETTINGS_FILE),
    })
}

/// Cartella predefinita dei modelli, senza considerare `TRASCRITTORE_MODELS_DIR`.
pub fn default_models_dir() -> Result<PathBuf> {
    default_models_dir_for(portable_data_dir())
}

/// Cartella predefinita dei modelli con la cartella portable indicata (`None` = profilo utente).
pub fn default_models_dir_for(portable: Option<&Path>) -> Result<PathBuf> {
    Ok(match portable {
        Some(data) => data.join(PORTABLE_MODELS_DIR),
        None => project_dirs()?.data_local_dir().join("models"),
    })
}

/// Cartella in cui viene installato il runtime OpenVINO scaricato dal programma.
pub fn openvino_dir() -> Result<PathBuf> {
    openvino_dir_for(portable_data_dir())
}

/// Cartella di OpenVINO con la cartella portable indicata (`None` = profilo utente).
pub fn openvino_dir_for(portable: Option<&Path>) -> Result<PathBuf> {
    Ok(match portable {
        Some(data) => data.join(PORTABLE_OPENVINO_DIR),
        None => project_dirs()?.data_local_dir().join("openvino"),
    })
}

fn project_dirs() -> Result<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "", crate::APP_NAME)
        .ok_or_else(|| anyhow!("Impossibile determinare la cartella dei dati utente."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn normal_mode_without_markers() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(portable_data_dir_in(dir.path()), None);
        // Un file (non una cartella) chiamato «dati» non attiva la modalità portable.
        fs::write(dir.path().join(PORTABLE_DATA_DIR), "").unwrap();
        assert_eq!(portable_data_dir_in(dir.path()), None);
    }

    #[test]
    fn data_folder_enables_portable_mode() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("dati")).unwrap();
        assert_eq!(portable_data_dir_in(dir.path()), Some(dir.path().join("dati")));
    }

    #[test]
    fn marker_file_enables_portable_mode() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("portable.txt"), "").unwrap();
        let data = portable_data_dir_in(dir.path()).unwrap();
        assert_eq!(data, dir.path().join("dati"));
        // La cartella non viene creata dal solo rilevamento.
        assert!(!data.exists());
    }

    #[test]
    fn app_bundle_uses_folder_containing_it() {
        let folder = Path::new("/Volumes/USB/Trascrittore");
        let exe_dir = folder.join("Trascrittore.app").join("Contents").join("MacOS");
        assert_eq!(program_dir(&exe_dir), folder);
        // Fuori da un pacchetto .app la cartella del programma è quella dell'eseguibile.
        assert_eq!(program_dir(folder), folder);
        let not_bundle = folder.join("Altro").join("Contents").join("MacOS");
        assert_eq!(program_dir(&not_bundle), not_bundle);
    }

    #[test]
    fn portable_paths_stay_in_data_folder() {
        let data = Path::new(r"E:\Trascrittore\dati");
        assert_eq!(settings_file_for(Some(data)).unwrap(), data.join("impostazioni.json"));
        assert_eq!(default_models_dir_for(Some(data)).unwrap(), data.join("modelli"));
        assert_eq!(openvino_dir_for(Some(data)).unwrap(), data.join("openvino"));
    }

    #[test]
    fn profile_paths_outside_data_folder() {
        let settings = settings_file_for(None).unwrap();
        let models = default_models_dir_for(None).unwrap();
        assert!(settings.ends_with("impostazioni.json"));
        assert!(models.ends_with("models"));
        assert!(!settings.components().any(|c| c.as_os_str() == PORTABLE_DATA_DIR));
    }

    #[test]
    fn test_executable_is_not_portable() {
        // Accanto agli eseguibili dei test non ci sono «dati» né «portable.txt».
        assert_eq!(portable_data_dir(), None);
    }
}
