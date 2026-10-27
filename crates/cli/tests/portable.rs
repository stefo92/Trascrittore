//! Modalità portable: l'eseguibile copiato in una cartella con `portable.txt` tiene i dati
//! nella cartella `dati` accanto a sé.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Percorso stampato dopo `prefix`, in forma canonica (su macOS `/var` è un collegamento a
/// `/private/var`, su Windows la forma canonica usa il prefisso `\\?\`).
fn printed_path(stdout: &str, prefix: &str) -> PathBuf {
    let path = stdout
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("manca «{prefix}» in:\n{stdout}"));
    fs::canonicalize(path).unwrap()
}

#[test]
fn portable_copy_uses_data_folder() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join(format!("trascrittore-cli{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_trascrittore-cli"), &exe).unwrap();
    fs::write(dir.path().join("portable.txt"), "").unwrap();

    let output = Command::new(&exe)
        .arg("modelli")
        .env_remove("TRASCRITTORE_MODELS_DIR")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");

    let data = dir.path().join("dati");
    let models = data.join("modelli");
    assert!(models.is_dir());
    assert_eq!(
        printed_path(&stdout, "Modalità portable: impostazioni e modelli in "),
        fs::canonicalize(&data).unwrap()
    );
    assert_eq!(
        printed_path(&stdout, "Cartella dei modelli: "),
        fs::canonicalize(&models).unwrap()
    );
}
