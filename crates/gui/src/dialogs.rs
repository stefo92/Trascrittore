//! Finestre di dialogo comuni di Windows per aprire e salvare file.

use std::path::{Path, PathBuf};

use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, FileSaveDialog, IFileDialog, IFileOpenDialog, IFileSaveDialog, IShellItem,
    SHCreateItemFromParsingName, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT,
    FOS_PATHMUSTEXIST, SIGDN_FILESYSPATH,
};

/// Filtro dei tipi di file: descrizione e modelli separati da punto e virgola (es. `*.mp3;*.wav`).
pub struct Filter {
    pub name: String,
    pub pattern: String,
}

/// Mantiene vive le stringhe UTF-16 a cui puntano i `COMDLG_FILTERSPEC`.
struct FilterSpecs {
    _strings: Vec<(HSTRING, HSTRING)>,
    specs: Vec<COMDLG_FILTERSPEC>,
}

impl FilterSpecs {
    fn new(filters: &[Filter]) -> Self {
        let strings: Vec<(HSTRING, HSTRING)> = filters
            .iter()
            .map(|f| (HSTRING::from(f.name.as_str()), HSTRING::from(f.pattern.as_str())))
            .collect();
        let specs = strings
            .iter()
            .map(|(name, pattern)| COMDLG_FILTERSPEC {
                pszName: PCWSTR(name.as_ptr()),
                pszSpec: PCWSTR(pattern.as_ptr()),
            })
            .collect();
        Self {
            _strings: strings,
            specs,
        }
    }
}

unsafe fn prepare(dialog: &IFileDialog, title: &str, filters: &FilterSpecs, folder: Option<&Path>) {
    let _ = dialog.SetTitle(&HSTRING::from(title));
    if !filters.specs.is_empty() {
        let _ = dialog.SetFileTypes(&filters.specs);
        let _ = dialog.SetFileTypeIndex(1);
    }
    if let Some(folder) = folder.filter(|f| f.is_dir()) {
        if let Ok(item) =
            SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(folder.as_os_str()), None)
        {
            let _ = dialog.SetFolder(&item);
        }
    }
}

unsafe fn result_path(dialog: &IFileDialog) -> Option<PathBuf> {
    let item = dialog.GetResult().ok()?;
    let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
    let path = name.to_string().ok().map(PathBuf::from);
    CoTaskMemFree(Some(name.0 as *const _));
    path
}

/// Mostra la finestra «Apri». Restituisce `None` se l'utente annulla.
pub fn open_file(owner: HWND, title: &str, filters: &[Filter], folder: Option<&Path>) -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let specs = FilterSpecs::new(filters);
        let base: IFileDialog = dialog.cast().ok()?;
        prepare(&base, title, &specs, folder);
        if let Ok(options) = base.GetOptions() {
            let _ = base.SetOptions(
                options | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST | FOS_FORCEFILESYSTEM,
            );
        }
        base.Show(Some(owner)).ok()?;
        result_path(&base)
    }
}

/// Mostra la finestra «Salva con nome». Restituisce il percorso scelto e
/// l'indice (da 0) del filtro selezionato.
pub fn save_file(
    owner: HWND,
    title: &str,
    filters: &[Filter],
    folder: Option<&Path>,
    file_name: &str,
    default_extension: &str,
) -> Option<(PathBuf, usize)> {
    unsafe {
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let specs = FilterSpecs::new(filters);
        let base: IFileDialog = dialog.cast().ok()?;
        prepare(&base, title, &specs, folder);
        let _ = base.SetFileName(&HSTRING::from(file_name));
        let _ = base.SetDefaultExtension(&HSTRING::from(default_extension));
        if let Ok(options) = base.GetOptions() {
            let _ = base.SetOptions(
                options | FOS_OVERWRITEPROMPT | FOS_PATHMUSTEXIST | FOS_FORCEFILESYSTEM,
            );
        }
        base.Show(Some(owner)).ok()?;
        let index = base.GetFileTypeIndex().map(|i| i.saturating_sub(1) as usize).unwrap_or(0);
        result_path(&base).map(|p| (p, index))
    }
}
