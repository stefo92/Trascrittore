//! Accelerazione dell'encoder di Whisper con OpenVINO (Intel).
//!
//! OpenVINO esegue l'encoder, la parte più pesante del calcolo, su un acceleratore neurale
//! (NPU), su una scheda video Intel o sul processore; il decoder resta su ggml (processore o
//! scheda video con Vulkan).
//!
//! whisper.cpp è compilato con `WHISPER_USE_OPENVINO` (vedi `.cargo/config.toml`) e chiama le tre
//! funzioni `whisper_openvino_*` definite qui, che usano l'API C di OpenVINO caricata a runtime:
//! - il programma non dipende da OpenVINO né per compilare né per partire;
//! - con l'API C non si mescolano runtime C++ diversi (Trascrittore usa il runtime statico);
//! - non si usa l'enum dei tipi degli elementi, i cui valori cambiano tra le versioni di
//!   OpenVINO: i dati vengono copiati nei tensori allocati dalla richiesta di inferenza, dopo
//!   averne controllato la dimensione in byte.
//!
//! Il runtime viene cercato nella cartella indicata da [`DIR_ENV`] e poi nella cartella
//! `openvino` dei dati del programma, dove lo installa [`install_runtime`].

use std::ffi::{c_char, c_int, c_void, CStr};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};

/// Variabile d'ambiente con la cartella che contiene le librerie di OpenVINO
/// (per esempio `...\runtime\bin\intel64\Release` di un'installazione ufficiale).
pub const DIR_ENV: &str = "TRASCRITTORE_OPENVINO_DIR";

/// Versione del runtime installato da [`install_runtime`].
pub const RUNTIME_VERSION: &str = "2026.4.1";
/// Dimensione dell'archivio del runtime da scaricare.
pub const RUNTIME_DOWNLOAD_BYTES: u64 = 237_069_634;
/// Archivio ufficiale Intel per Windows, compilato con il runtime C++ statico (`vc_mt`): le
/// librerie non richiedono il Visual C++ Redistributable, come Trascrittore.
const RUNTIME_URL: &str = "https://storage.openvinotoolkit.org/repositories/openvino/packages/2026.4.1/windows_vc_mt/openvino_toolkit_windows_vc_mt_2026.4.1.22982.07f9c262b05_x86_64.zip";

/// Repository ufficiale Intel con gli encoder convertiti per OpenVINO.
const ENCODER_BASE_URL: &str =
    "https://huggingface.co/Intel/whisper.cpp-openvino-models/resolve/main";

#[cfg(windows)]
const C_LIBRARY: &str = "openvino_c.dll";
#[cfg(target_os = "macos")]
const C_LIBRARY: &str = "libopenvino_c.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
const C_LIBRARY: &str = "libopenvino_c.so";

/// Librerie del runtime estratte dall'archivio: nucleo, API C, lettore dei modelli IR,
/// plugin per CPU, GPU e NPU Intel e le librerie TBB da cui dipendono.
const RUNTIME_FILES: &[&str] = &[
    "openvino.dll",
    "openvino_c.dll",
    "openvino_ir_frontend.dll",
    "openvino_intel_cpu_plugin.dll",
    "openvino_intel_gpu_plugin.dll",
    "openvino_intel_npu_plugin.dll",
    "openvino_intel_npu_compiler.dll",
    "openvino_intel_npu_compiler_loader.dll",
    "openvino_intel_npu_vm_runtime.dll",
    "openvino_auto_plugin.dll",
    "openvino_auto_batch_plugin.dll",
    "openvino_hetero_plugin.dll",
    "cache.json",
    "tbb12.dll",
    "tbbmalloc.dll",
    "tbbbind_2_5.dll",
];

const VERSION_FILE: &str = "versione.txt";
const LICENSES_DIR: &str = "licenze";

// ---------------------------------------------------------------------------------------------
// API C di OpenVINO caricata dinamicamente
// ---------------------------------------------------------------------------------------------

/// `ov_status_e`: 0 = OK.
type Status = c_int;

/// `ov_available_devices_t`.
#[repr(C)]
struct AvailableDevices {
    devices: *mut *mut c_char,
    size: usize,
}

/// Funzioni dell'API C usate dal programma (firme da `openvino/c/*.h`).
struct Fns {
    core_create: unsafe extern "C" fn(*mut *mut c_void) -> Status,
    core_free: unsafe extern "C" fn(*mut c_void),
    core_read_model:
        unsafe extern "C" fn(*const c_void, *const c_char, *const c_char, *mut *mut c_void) -> Status,
    core_compile_model: unsafe extern "C" fn(
        *const c_void,
        *const c_void,
        *const c_char,
        usize,
        *mut *mut c_void,
        ...
    ) -> Status,
    core_set_property: unsafe extern "C" fn(*const c_void, *const c_char, ...) -> Status,
    core_get_property:
        unsafe extern "C" fn(*const c_void, *const c_char, *const c_char, *mut *mut c_char) -> Status,
    core_get_available_devices: unsafe extern "C" fn(*const c_void, *mut AvailableDevices) -> Status,
    available_devices_free: unsafe extern "C" fn(*mut AvailableDevices),
    compiled_model_create_infer_request: unsafe extern "C" fn(*const c_void, *mut *mut c_void) -> Status,
    compiled_model_free: unsafe extern "C" fn(*mut c_void),
    model_free: unsafe extern "C" fn(*mut c_void),
    infer_request_get_input_tensor: unsafe extern "C" fn(*const c_void, *mut *mut c_void) -> Status,
    infer_request_get_output_tensor: unsafe extern "C" fn(*const c_void, *mut *mut c_void) -> Status,
    infer_request_infer: unsafe extern "C" fn(*mut c_void) -> Status,
    infer_request_free: unsafe extern "C" fn(*mut c_void),
    tensor_data: unsafe extern "C" fn(*const c_void, *mut *mut c_void) -> Status,
    tensor_get_byte_size: unsafe extern "C" fn(*const c_void, *mut usize) -> Status,
    tensor_free: unsafe extern "C" fn(*mut c_void),
    free: unsafe extern "C" fn(*const c_char),
    /// Presente dalla versione 2023.1.
    get_last_err_msg: Option<unsafe extern "C" fn() -> *const c_char>,
}

/// Copia il puntatore a funzione di un simbolo; il tipo viene dedotto dal campo di destinazione.
macro_rules! symbol {
    ($lib:expr, $name:literal) => {
        *$lib
            .get(concat!($name, "\0").as_bytes())
            .map_err(|e| format!("funzione {} mancante: {e}", $name))?
    };
}

impl Fns {
    /// # Safety
    /// `lib` deve essere l'API C di OpenVINO, con le firme dichiarate in [`Fns`].
    unsafe fn load(lib: &libloading::Library) -> std::result::Result<Self, String> {
        // SAFETY: le firme corrispondono alle intestazioni dell'API C di OpenVINO.
        unsafe {
            Ok(Self {
                core_create: symbol!(lib, "ov_core_create"),
                core_free: symbol!(lib, "ov_core_free"),
                core_read_model: symbol!(lib, "ov_core_read_model"),
                core_compile_model: symbol!(lib, "ov_core_compile_model"),
                core_set_property: symbol!(lib, "ov_core_set_property"),
                core_get_property: symbol!(lib, "ov_core_get_property"),
                core_get_available_devices: symbol!(lib, "ov_core_get_available_devices"),
                available_devices_free: symbol!(lib, "ov_available_devices_free"),
                compiled_model_create_infer_request: symbol!(
                    lib,
                    "ov_compiled_model_create_infer_request"
                ),
                compiled_model_free: symbol!(lib, "ov_compiled_model_free"),
                model_free: symbol!(lib, "ov_model_free"),
                infer_request_get_input_tensor: symbol!(lib, "ov_infer_request_get_input_tensor"),
                infer_request_get_output_tensor: symbol!(lib, "ov_infer_request_get_output_tensor"),
                infer_request_infer: symbol!(lib, "ov_infer_request_infer"),
                infer_request_free: symbol!(lib, "ov_infer_request_free"),
                tensor_data: symbol!(lib, "ov_tensor_data"),
                tensor_get_byte_size: symbol!(lib, "ov_tensor_get_byte_size"),
                tensor_free: symbol!(lib, "ov_tensor_free"),
                free: symbol!(lib, "ov_free"),
                get_last_err_msg: lib
                    .get::<unsafe extern "C" fn() -> *const c_char>(b"ov_get_last_err_msg\0")
                    .ok()
                    .map(|s| *s),
            })
        }
    }

    /// Converte uno stato di errore in un messaggio, con il dettaglio fornito da OpenVINO.
    fn check(&self, status: Status, what: &str) -> std::result::Result<(), String> {
        if status == 0 {
            return Ok(());
        }
        let detail = self
            .get_last_err_msg
            // SAFETY: restituisce una stringa C gestita da OpenVINO (o null).
            .map(|f| unsafe { f() })
            .filter(|p| !p.is_null())
            // SAFETY: stringa terminata da zero, valida fino alla prossima chiamata.
            .map(|p| unsafe { CStr::from_ptr(p) }.to_string_lossy().trim().to_string())
            .filter(|s| !s.is_empty());
        Err(match detail {
            Some(detail) => format!("{what} (codice {status}): {detail}"),
            None => format!("{what} (codice {status})"),
        })
    }
}

/// Runtime OpenVINO caricato.
struct Api {
    fns: Fns,
    dir: PathBuf,
    _lib: libloading::Library,
}

/// Errore di caricamento del runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LoadError {
    NotInstalled,
    Failed(String),
}

static API: Mutex<Option<Arc<Api>>> = Mutex::new(None);
static DEVICES: Mutex<Option<Vec<Device>>> = Mutex::new(None);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);
static ENCODE_FAILURES: AtomicUsize = AtomicUsize::new(0);

/// Cartelle in cui cercare il runtime, in ordine di priorità.
pub fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os(DIR_ENV).filter(|d| !d.is_empty()) {
        dirs.push(PathBuf::from(dir));
    }
    if let Ok(dir) = crate::paths::openvino_dir() {
        dirs.push(dir);
    }
    dirs
}

/// # Safety
/// Caricare una libreria esegue il suo codice di inizializzazione.
unsafe fn open_library(path: &Path) -> std::result::Result<libloading::Library, libloading::Error> {
    #[cfg(windows)]
    {
        use libloading::os::windows::{
            Library, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        };
        // Le dipendenze (openvino.dll, tbb12.dll) si cercano nella cartella della libreria.
        // SAFETY: vedi la funzione.
        unsafe {
            Library::load_with_flags(
                path,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        }
        .map(Into::into)
    }
    #[cfg(not(windows))]
    {
        // SAFETY: vedi la funzione.
        unsafe { libloading::Library::new(path) }
    }
}

fn api() -> std::result::Result<Arc<Api>, LoadError> {
    let mut guard = API.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(api) = guard.as_ref() {
        return Ok(Arc::clone(api));
    }
    // Le librerie non sono ancora caricate: è il momento di attivare un aggiornamento
    // preparato mentre la versione precedente era in uso.
    apply_pending_install();
    let mut errors = Vec::new();
    for dir in candidate_dirs() {
        let path = dir.join(C_LIBRARY);
        if !path.is_file() {
            continue;
        }
        // SAFETY: è la libreria dell'API C di OpenVINO cercata nelle cartelle previste.
        let loaded = unsafe { open_library(&path) }
            .map_err(|e| e.to_string())
            // SAFETY: come sopra.
            .and_then(|lib| unsafe { Fns::load(&lib) }.map(|fns| (lib, fns)));
        match loaded {
            Ok((lib, fns)) => {
                let api = Arc::new(Api {
                    fns,
                    dir,
                    _lib: lib,
                });
                *guard = Some(Arc::clone(&api));
                return Ok(api);
            }
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }
    if errors.is_empty() {
        Err(LoadError::NotInstalled)
    } else {
        Err(LoadError::Failed(errors.join("; ")))
    }
}

/// Core OpenVINO liberato automaticamente.
struct Core<'a> {
    fns: &'a Fns,
    ptr: *mut c_void,
}

impl<'a> Core<'a> {
    fn new(fns: &'a Fns) -> std::result::Result<Self, String> {
        let mut ptr = ptr::null_mut();
        // SAFETY: `ptr` riceve un core valido in caso di successo.
        fns.check(unsafe { (fns.core_create)(&mut ptr) }, "creazione del core OpenVINO")?;
        Ok(Self { fns, ptr })
    }
}

impl Drop for Core<'_> {
    fn drop(&mut self) {
        // SAFETY: core creato da `ov_core_create` e non ancora liberato.
        unsafe { (self.fns.core_free)(self.ptr) }
    }
}

// ---------------------------------------------------------------------------------------------
// Stato e dispositivi
// ---------------------------------------------------------------------------------------------

/// Un dispositivo OpenVINO (es. `NPU`, `GPU.0`, `CPU`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// Nome usato da OpenVINO.
    pub id: String,
    /// Nome completo (es. «Intel(R) AI Boost»).
    pub name: String,
}

impl Device {
    /// Tipo di dispositivo in italiano.
    pub fn kind(&self) -> &'static str {
        let id = self.id.to_ascii_uppercase();
        if id.starts_with("NPU") {
            "Acceleratore neurale (NPU)"
        } else if id.starts_with("GPU") {
            "Scheda video"
        } else if id.starts_with("CPU") {
            "Processore"
        } else {
            "Dispositivo"
        }
    }

    /// Descrizione per menu e sintesi vocale (es. «Acceleratore neurale (NPU): Intel(R) AI Boost»).
    pub fn label(&self) -> String {
        if self.name.is_empty() || self.name == self.id {
            format!("{}: {}", self.kind(), self.id)
        } else {
            format!("{}: {} ({})", self.kind(), self.name, self.id)
        }
    }
}

/// Disponibilità di OpenVINO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeStatus {
    /// Runtime non trovato.
    NotInstalled,
    /// Runtime trovato ma non utilizzabile.
    Unavailable(String),
    /// Runtime pronto, con i dispositivi utilizzabili.
    Ready { dir: PathBuf, devices: Vec<Device> },
}

/// Stato del runtime e dispositivi disponibili. L'elenco viene calcolato una volta e
/// riutilizzato fino a [`refresh`].
pub fn status() -> RuntimeStatus {
    let api = match api() {
        Ok(api) => api,
        Err(LoadError::NotInstalled) => return RuntimeStatus::NotInstalled,
        Err(LoadError::Failed(e)) => return RuntimeStatus::Unavailable(e),
    };
    let mut cache = DEVICES.lock().unwrap_or_else(|e| e.into_inner());
    if cache.is_none() {
        match enumerate(&api.fns) {
            Ok(devices) => *cache = Some(devices),
            Err(e) => return RuntimeStatus::Unavailable(e),
        }
    }
    RuntimeStatus::Ready {
        dir: api.dir.clone(),
        devices: cache.clone().unwrap_or_default(),
    }
}

/// Dispositivi utilizzabili (vuoto se OpenVINO non è disponibile).
pub fn devices() -> Vec<Device> {
    match status() {
        RuntimeStatus::Ready { devices, .. } => devices,
        _ => Vec::new(),
    }
}

/// Dimentica lo stato calcolato (dopo un'installazione).
pub fn refresh() {
    DEVICES.lock().unwrap_or_else(|e| e.into_inner()).take();
}

fn enumerate(fns: &Fns) -> std::result::Result<Vec<Device>, String> {
    let core = Core::new(fns)?;
    let mut list = AvailableDevices {
        devices: ptr::null_mut(),
        size: 0,
    };
    // SAFETY: `list` riceve un elenco allocato da OpenVINO, liberato più sotto.
    fns.check(
        unsafe { (fns.core_get_available_devices)(core.ptr, &mut list) },
        "elenco dei dispositivi OpenVINO",
    )?;
    let mut devices = Vec::new();
    for i in 0..list.size {
        // SAFETY: `devices` contiene `size` stringhe C valide.
        let id_ptr = unsafe { *list.devices.add(i) };
        if id_ptr.is_null() {
            continue;
        }
        // SAFETY: stringa terminata da zero.
        let id = unsafe { CStr::from_ptr(id_ptr) }.to_string_lossy().into_owned();
        let mut value: *mut c_char = ptr::null_mut();
        // SAFETY: `value`, se valorizzato, va liberato con `ov_free`.
        let status = unsafe {
            (fns.core_get_property)(core.ptr, id_ptr, c"FULL_DEVICE_NAME".as_ptr(), &mut value)
        };
        let name = if status == 0 && !value.is_null() {
            // SAFETY: stringa allocata da OpenVINO, liberata subito dopo la copia.
            let name = unsafe { CStr::from_ptr(value) }.to_string_lossy().trim().to_string();
            unsafe { (fns.free)(value) };
            name
        } else {
            String::new()
        };
        devices.push(Device { id, name });
    }
    // SAFETY: elenco ottenuto da `ov_core_get_available_devices`.
    unsafe { (fns.available_devices_free)(&mut list) };
    // Il plugin GPU di OpenVINO usa OpenCL e vede anche schede AMD o NVIDIA, ma supporta solo
    // quelle Intel: su una AMD Radeon RX 6800 XT la preparazione dell'encoder fallisce.
    devices.retain(is_supported);
    // Prima gli acceleratori più efficienti per l'encoder.
    devices.sort_by_key(|d| match d.kind() {
        "Acceleratore neurale (NPU)" => 0,
        "Scheda video" => 1,
        "Processore" => 2,
        _ => 3,
    });
    Ok(devices)
}

/// `false` per le schede video non Intel, che il plugin GPU di OpenVINO non supporta.
fn is_supported(device: &Device) -> bool {
    !device.id.to_ascii_uppercase().starts_with("GPU")
        || device.name.to_ascii_lowercase().contains("intel")
}

/// Ultimo errore di OpenVINO durante la preparazione o l'esecuzione dell'encoder.
pub fn last_error() -> Option<String> {
    LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Dimentica l'ultimo errore (all'inizio di una trascrizione).
pub fn clear_last_error() {
    LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()).take();
}

fn set_last_error(message: String) {
    log::warn!("OpenVINO: {message}");
    *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = Some(message);
}

/// Numero di elaborazioni dell'encoder non riuscite dall'avvio del programma.
///
/// whisper.cpp ignora l'esito di `whisper_openvino_encode`: chi trascrive confronta questo
/// contatore prima e dopo per non restituire un testo calcolato su dati non validi.
pub fn encode_failures() -> usize {
    ENCODE_FAILURES.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------------------------------------
// Encoder chiamato da whisper.cpp
// ---------------------------------------------------------------------------------------------

/// Contesto restituito a whisper.cpp come `whisper_openvino_context*`.
struct Encoder {
    api: Arc<Api>,
    core: *mut c_void,
    model: *mut c_void,
    compiled: *mut c_void,
    request: *mut c_void,
}

impl Encoder {
    /// # Safety
    /// I puntatori devono essere stringhe C valide (o null, ma solo `cache_dir`).
    unsafe fn open(
        path_model: *const c_char,
        device: *const c_char,
        cache_dir: *const c_char,
    ) -> std::result::Result<Self, String> {
        if path_model.is_null() || device.is_null() {
            return Err("percorso dell'encoder o dispositivo mancante".into());
        }
        let api = match api() {
            Ok(api) => api,
            Err(LoadError::NotInstalled) => return Err("runtime OpenVINO non installato".into()),
            Err(LoadError::Failed(e)) => return Err(e),
        };
        let mut encoder = Self {
            api,
            core: ptr::null_mut(),
            model: ptr::null_mut(),
            compiled: ptr::null_mut(),
            request: ptr::null_mut(),
        };
        let fns = &encoder.api.fns;
        // SAFETY: argomenti validi come richiesto; gli oggetti creati sono liberati da Drop.
        unsafe {
            fns.check((fns.core_create)(&mut encoder.core), "creazione del core OpenVINO")?;
            if !cache_dir.is_null() {
                // La cache dei modelli compilati rende molto più veloci gli avvii successivi
                // su GPU e NPU. Se non è impostabile si procede senza.
                let status = (fns.core_set_property)(
                    encoder.core,
                    device,
                    c"CACHE_DIR".as_ptr(),
                    cache_dir,
                );
                if status != 0 {
                    log::warn!("OpenVINO: cache dei modelli compilati non disponibile ({status})");
                }
            }
            fns.check(
                (fns.core_read_model)(encoder.core, path_model, ptr::null(), &mut encoder.model),
                "lettura dell'encoder OpenVINO",
            )?;
            fns.check(
                (fns.core_compile_model)(
                    encoder.core,
                    encoder.model,
                    device,
                    0,
                    &mut encoder.compiled,
                ),
                &format!(
                    "preparazione dell'encoder per il dispositivo {}",
                    CStr::from_ptr(device).to_string_lossy()
                ),
            )?;
            fns.check(
                (fns.compiled_model_create_infer_request)(encoder.compiled, &mut encoder.request),
                "creazione della richiesta di inferenza",
            )?;
        }
        Ok(encoder)
    }

    /// Prende un tensore della richiesta e ne restituisce dati e dimensione in byte.
    ///
    /// # Safety
    /// `get` deve essere una delle funzioni `ov_infer_request_get_*_tensor`.
    unsafe fn tensor(
        &self,
        get: unsafe extern "C" fn(*const c_void, *mut *mut c_void) -> Status,
        what: &str,
    ) -> std::result::Result<TensorData<'_>, String> {
        let fns = &self.api.fns;
        let mut handle = ptr::null_mut();
        // SAFETY: richiesta valida; il tensore restituito va liberato (vedi Tensor).
        fns.check(unsafe { get(self.request, &mut handle) }, what)?;
        let tensor = Tensor { fns, handle };
        let mut data = ptr::null_mut();
        let mut size = 0usize;
        // SAFETY: tensore valido.
        unsafe {
            fns.check((fns.tensor_data)(handle, &mut data), what)?;
            fns.check((fns.tensor_get_byte_size)(handle, &mut size), what)?;
        }
        if data.is_null() {
            return Err(format!("{what}: memoria del tensore non disponibile"));
        }
        Ok(tensor.with_data(data, size))
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        let fns = &self.api.fns;
        // SAFETY: oggetti creati da OpenVINO, liberati una sola volta in ordine inverso.
        unsafe {
            if !self.request.is_null() {
                (fns.infer_request_free)(self.request);
            }
            if !self.compiled.is_null() {
                (fns.compiled_model_free)(self.compiled);
            }
            if !self.model.is_null() {
                (fns.model_free)(self.model);
            }
            if !self.core.is_null() {
                (fns.core_free)(self.core);
            }
        }
    }
}

/// Tensore di una richiesta di inferenza, con i suoi dati.
struct Tensor<'a> {
    fns: &'a Fns,
    handle: *mut c_void,
}

struct TensorData<'a> {
    _tensor: Tensor<'a>,
    data: *mut c_void,
    size: usize,
}

impl<'a> Tensor<'a> {
    fn with_data(self, data: *mut c_void, size: usize) -> TensorData<'a> {
        TensorData {
            _tensor: self,
            data,
            size,
        }
    }
}

impl Drop for Tensor<'_> {
    fn drop(&mut self) {
        // SAFETY: tensore restituito da `ov_infer_request_get_*_tensor`.
        unsafe { (self.fns.tensor_free)(self.handle) }
    }
}

#[cfg(feature = "whisper")]
mod whisper_hooks {
    //! Le tre funzioni dichiarate in `whisper.cpp/src/openvino/whisper-openvino-encoder.h`.

    use std::ffi::{c_char, c_int, c_void};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::ptr;
    use std::sync::atomic::Ordering;

    use whisper_rs::whisper_rs_sys::{
        ggml_backend_tensor_get, ggml_backend_tensor_set, ggml_nbytes, ggml_tensor,
    };

    use super::{set_last_error, Encoder, ENCODE_FAILURES};

    /// Prepara l'encoder OpenVINO. Restituisce null in caso di errore (whisper.cpp usa allora
    /// l'encoder normale); il motivo si legge con [`super::last_error`].
    ///
    /// # Safety
    /// Chiamata da whisper.cpp con stringhe C valide.
    #[no_mangle]
    pub unsafe extern "C" fn whisper_openvino_init(
        path_model: *const c_char,
        device: *const c_char,
        cache_dir: *const c_char,
    ) -> *mut c_void {
        // SAFETY: argomenti forniti da whisper.cpp.
        let result = catch_unwind(|| unsafe { Encoder::open(path_model, device, cache_dir) });
        match result {
            Ok(Ok(encoder)) => Box::into_raw(Box::new(encoder)).cast(),
            Ok(Err(message)) => {
                set_last_error(message);
                ptr::null_mut()
            }
            Err(_) => {
                set_last_error("errore interno durante la preparazione dell'encoder".into());
                ptr::null_mut()
            }
        }
    }

    /// Libera un contesto restituito da [`whisper_openvino_init`].
    ///
    /// # Safety
    /// `ctx` deve essere null o un contesto non ancora liberato.
    #[no_mangle]
    pub unsafe extern "C" fn whisper_openvino_free(ctx: *mut c_void) {
        if !ctx.is_null() {
            // SAFETY: creato con Box::into_raw in `whisper_openvino_init`.
            let _ = catch_unwind(|| drop(unsafe { Box::from_raw(ctx.cast::<Encoder>()) }));
        }
    }

    /// Esegue l'encoder: legge lo spettrogramma `mel` e scrive il risultato in `out`.
    /// Restituisce 1 se riesce, 0 altrimenti.
    ///
    /// # Safety
    /// Chiamata da whisper.cpp con un contesto valido e tensori allocati.
    #[no_mangle]
    pub unsafe extern "C" fn whisper_openvino_encode(
        ctx: *mut c_void,
        mel: *mut ggml_tensor,
        out: *mut ggml_tensor,
    ) -> c_int {
        if ctx.is_null() || mel.is_null() || out.is_null() {
            set_last_error("encoder OpenVINO chiamato senza dati".into());
            ENCODE_FAILURES.fetch_add(1, Ordering::SeqCst);
            return 0;
        }
        // SAFETY: contesto creato da `whisper_openvino_init`.
        let encoder = unsafe { &*ctx.cast::<Encoder>() };
        // SAFETY: tensori validi forniti da whisper.cpp.
        let result = catch_unwind(AssertUnwindSafe(|| unsafe { encode(encoder, mel, out) }));
        match result {
            Ok(Ok(())) => 1,
            Ok(Err(message)) => {
                set_last_error(message);
                ENCODE_FAILURES.fetch_add(1, Ordering::SeqCst);
                0
            }
            Err(_) => {
                set_last_error("errore interno durante l'elaborazione dell'encoder".into());
                ENCODE_FAILURES.fetch_add(1, Ordering::SeqCst);
                0
            }
        }
    }

    /// # Safety
    /// Vedi [`whisper_openvino_encode`].
    unsafe fn encode(
        encoder: &Encoder,
        mel: *mut ggml_tensor,
        out: *mut ggml_tensor,
    ) -> Result<(), String> {
        let fns = &encoder.api.fns;
        // SAFETY: tensori ggml validi; le copie usano le funzioni di ggml, che funzionano
        // anche se il tensore si trova nella memoria di una scheda video.
        unsafe {
            let mel_bytes = ggml_nbytes(mel);
            let out_bytes = ggml_nbytes(out);

            let input = encoder.tensor(fns.infer_request_get_input_tensor, "ingresso dell'encoder")?;
            if input.size != mel_bytes {
                return Err(format!(
                    "l'encoder OpenVINO non corrisponde al modello: ingresso di {} byte invece di {mel_bytes}",
                    input.size
                ));
            }
            ggml_backend_tensor_get(mel, input.data, 0, mel_bytes);
            drop(input);

            fns.check((fns.infer_request_infer)(encoder.request), "esecuzione dell'encoder")?;

            let output = encoder.tensor(fns.infer_request_get_output_tensor, "uscita dell'encoder")?;
            if output.size != out_bytes {
                return Err(format!(
                    "l'encoder OpenVINO non corrisponde al modello: uscita di {} byte invece di {out_bytes}",
                    output.size
                ));
            }
            ggml_backend_tensor_set(out, output.data, 0, out_bytes);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Encoder dei modelli
// ---------------------------------------------------------------------------------------------

/// Pacchetto ufficiale Intel con l'encoder OpenVINO di una famiglia di modelli.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderPackage {
    /// Famiglia del modello (es. `base`): vale anche per le varianti compresse.
    pub family: &'static str,
    /// Dimensione del pacchetto (contiene anche il modello ggml non compresso).
    pub size_bytes: u64,
}

/// Pacchetti disponibili. Non esistono encoder già pronti per tiny e large-v3-turbo.
pub const ENCODER_PACKAGES: &[EncoderPackage] = &[
    EncoderPackage {
        family: "base",
        size_bytes: 171_306_938,
    },
    EncoderPackage {
        family: "small",
        size_bytes: 610_456_149,
    },
    EncoderPackage {
        family: "medium",
        size_bytes: 2_110_515_739,
    },
    EncoderPackage {
        family: "large-v3",
        size_bytes: 4_277_163_902,
    },
];

impl EncoderPackage {
    /// Indirizzo del pacchetto.
    pub fn url(&self) -> String {
        format!("{ENCODER_BASE_URL}/ggml-{}-models.zip", self.family)
    }

    /// Nome del file XML dell'encoder.
    pub fn encoder_file_name(&self) -> String {
        encoder_file_name(self.family)
    }
}

/// Nome del file XML dell'encoder di una famiglia (convenzione di whisper.cpp).
pub fn encoder_file_name(family: &str) -> String {
    format!("ggml-{family}-encoder-openvino.xml")
}

/// Famiglia di un modello, senza il suffisso della compressione: `base-q5_1` → `base`.
pub fn model_family(model_id: &str) -> &str {
    for suffix in ["-q5_0", "-q5_1", "-q8_0", "-q4_0", "-q4_1"] {
        if let Some(family) = model_id.strip_suffix(suffix) {
            return family;
        }
    }
    model_id
}

/// Identificativo del modello dal nome del file (`ggml-base-q5_1.bin` → `base-q5_1`).
fn model_id_from_path(model_path: &Path) -> Option<&str> {
    let stem = model_path.file_stem()?.to_str()?;
    Some(stem.strip_prefix("ggml-").unwrap_or(stem))
}

/// Pacchetto ufficiale con l'encoder adatto al modello, se esiste.
pub fn package_for_model(model_path: &Path) -> Option<&'static EncoderPackage> {
    let family = model_family(model_id_from_path(model_path)?);
    ENCODER_PACKAGES.iter().find(|p| p.family == family)
}

/// Encoder OpenVINO già presente per il modello: con lo stesso nome del modello
/// (`ggml-x-encoder-openvino.xml`, convenzione di whisper.cpp) o con quello della sua famiglia.
pub fn find_encoder(model_path: &Path) -> Option<PathBuf> {
    let dir = model_path.parent()?;
    let stem = model_path.file_stem()?.to_str()?;
    let mut candidates = vec![dir.join(format!("{stem}-encoder-openvino.xml"))];
    if let Some(id) = model_id_from_path(model_path) {
        candidates.push(dir.join(encoder_file_name(model_family(id))));
    }
    candidates
        .into_iter()
        .find(|xml| xml.is_file() && xml.with_extension("bin").is_file())
}

/// Scarica il pacchetto Intel e ne estrae l'encoder nella cartella dei modelli.
/// Se il modello ggml non compresso della stessa famiglia manca, viene estratto anch'esso.
pub fn download_encoder(
    package: &EncoderPackage,
    models_dir: &Path,
    progress: impl FnMut(u64, Option<u64>),
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    let zip_path = models_dir.join(format!("ggml-{}-models.zip", package.family));
    crate::models::download_url(&package.url(), &zip_path, progress, cancel)?;
    let result = extract_encoder(package, &zip_path, models_dir);
    let _ = fs::remove_file(&zip_path);
    result
}

fn extract_encoder(package: &EncoderPackage, zip_path: &Path, models_dir: &Path) -> Result<PathBuf> {
    let xml_name = package.encoder_file_name();
    let bin_name = Path::new(&xml_name)
        .with_extension("bin")
        .to_string_lossy()
        .into_owned();
    let model_name = format!("ggml-{}.bin", package.family);

    let mut archive = zip::ZipArchive::new(File::open(zip_path)?)
        .context("Il pacchetto dell'encoder non è un archivio valido.")?;
    let mut found = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let full_name = entry.name()?.into_owned();
        let Some(name) = Path::new(&full_name)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
        else {
            continue;
        };
        let wanted = name == xml_name || name == bin_name;
        let dest = models_dir.join(&name);
        if wanted || (name == model_name && !dest.exists()) {
            write_entry(&mut entry, &dest)?;
            if wanted {
                found += 1;
            }
        }
    }
    if found < 2 {
        bail!("Il pacchetto non contiene l'encoder «{xml_name}».");
    }
    Ok(models_dir.join(xml_name))
}

/// Scrive un elemento dell'archivio passando per un file temporaneo.
fn write_entry(entry: &mut impl Read, dest: &Path) -> Result<()> {
    let mut partial = dest.as_os_str().to_owned();
    partial.push(".part");
    let partial = PathBuf::from(partial);
    let result = (|| -> Result<()> {
        let mut file = File::create(&partial)?;
        io::copy(entry, &mut file)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&partial);
        return Err(e).with_context(|| format!("Impossibile estrarre «{}».", dest.display()));
    }
    fs::rename(&partial, dest)
        .with_context(|| format!("Impossibile salvare «{}».", dest.display()))
}

// ---------------------------------------------------------------------------------------------
// Installazione del runtime
// ---------------------------------------------------------------------------------------------

/// `true` se il runtime installato dal programma è presente.
pub fn runtime_installed() -> bool {
    crate::paths::openvino_dir()
        .map(|d| d.join(C_LIBRARY).is_file())
        .unwrap_or(false)
}

/// Esito di [`install_runtime`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallOutcome {
    /// Runtime installato e utilizzabile subito.
    Installed(PathBuf),
    /// La versione precedente è in uso: la nuova verrà attivata al prossimo avvio.
    AfterRestart(PathBuf),
}

/// Cartella in cui si prepara un'installazione prima di attivarla.
fn staging_dir(dir: &Path) -> PathBuf {
    let mut staging = dir.as_os_str().to_owned();
    staging.push(".nuovo");
    PathBuf::from(staging)
}

/// Sostituisce il runtime con quello preparato, se ce n'è uno completo.
fn apply_pending_install() {
    if let Ok(dir) = crate::paths::openvino_dir() {
        apply_pending_install_in(&dir);
    }
}

fn apply_pending_install_in(dir: &Path) {
    let dir = dir.to_path_buf();
    let staging = staging_dir(&dir);
    if !staging.join(C_LIBRARY).is_file() || !staging.join(VERSION_FILE).is_file() {
        return;
    }
    if dir.exists() && fs::remove_dir_all(&dir).is_err() {
        return;
    }
    if let Err(e) = fs::rename(&staging, &dir) {
        log::warn!("OpenVINO: aggiornamento non applicato: {e}");
    }
}

/// Scarica il runtime ufficiale di Intel, ne verifica l'impronta SHA-256 e ne estrae le
/// librerie necessarie (con le licenze) nella cartella `openvino` dei dati del programma.
pub fn install_runtime(
    progress: impl FnMut(u64, Option<u64>),
    cancel: &AtomicBool,
) -> Result<InstallOutcome> {
    if !cfg!(windows) {
        bail!(
            "L'installazione automatica di OpenVINO è disponibile solo su Windows: installa OpenVINO e imposta la variabile {DIR_ENV}."
        );
    }
    let dir = crate::paths::openvino_dir()?;
    // Su Windows le DLL caricate non si possono sostituire: se la versione installata è in
    // uso, la nuova viene preparata a parte e attivata al prossimo avvio.
    let in_use = API
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|api| api.dir == dir);
    let parent = dir
        .parent()
        .ok_or_else(|| anyhow!("Cartella di OpenVINO non valida."))?;
    fs::create_dir_all(parent)?;

    let expected = fetch_text(&format!("{RUNTIME_URL}.sha256"))?
        .split_whitespace()
        .next()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| anyhow!("Impronta SHA-256 di OpenVINO non disponibile."))?;

    let zip_path = parent.join("openvino-runtime.zip");
    crate::models::download_url(RUNTIME_URL, &zip_path, progress, cancel)?;
    let result = (|| {
        let actual = sha256_file(&zip_path)?;
        if actual != expected {
            bail!("L'archivio di OpenVINO scaricato è danneggiato (impronta SHA-256 diversa). Riprova.");
        }
        let staging = staging_dir(&dir);
        let _ = fs::remove_dir_all(&staging);
        extract_runtime(&zip_path, &staging)?;
        if in_use {
            return Ok(InstallOutcome::AfterRestart(dir.clone()));
        }
        if dir.exists() {
            fs::remove_dir_all(&dir).with_context(|| {
                format!("Impossibile sostituire la cartella «{}».", dir.display())
            })?;
        }
        fs::rename(&staging, &dir)
            .with_context(|| format!("Impossibile installare OpenVINO in «{}».", dir.display()))?;
        Ok(InstallOutcome::Installed(dir.clone()))
    })();
    let _ = fs::remove_file(&zip_path);
    refresh();
    result
}

fn extract_runtime(zip_path: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest.join(LICENSES_DIR))?;
    let mut archive = zip::ZipArchive::new(File::open(zip_path)?)
        .context("L'archivio di OpenVINO non è valido.")?;
    let mut extracted = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let path = entry.name()?.replace('\\', "/");
        let Some(name) = path.rsplit('/').next().map(str::to_string) else {
            continue;
        };
        let target = if (path.contains("/runtime/bin/intel64/Release/")
            || path.contains("/runtime/3rdparty/tbb/bin/"))
            && RUNTIME_FILES.contains(&name.as_str())
        {
            extracted += 1;
            dest.join(&name)
        } else if path.contains("/docs/licensing/") {
            dest.join(LICENSES_DIR).join(&name)
        } else {
            continue;
        };
        write_entry(&mut entry, &target)?;
    }
    if !dest.join(C_LIBRARY).is_file() || extracted < 8 {
        bail!("L'archivio di OpenVINO non contiene le librerie attese.");
    }
    fs::write(dest.join(VERSION_FILE), RUNTIME_VERSION)?;
    Ok(())
}

fn fetch_text(url: &str) -> Result<String> {
    let mut response = ureq::get(url)
        .call()
        .map_err(|e| anyhow!("Download non riuscito: {e}. Controlla la connessione a Internet."))?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow!("Risposta non valida dal server: {e}"))
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
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_and_packages() {
        assert_eq!(model_family("base-q5_1"), "base");
        assert_eq!(model_family("medium-q5_0"), "medium");
        assert_eq!(model_family("large-v3-turbo-q5_0"), "large-v3-turbo");
        assert_eq!(model_family("small"), "small");
        let base = package_for_model(Path::new("C:/m/ggml-base-q5_1.bin")).unwrap();
        assert_eq!(base.family, "base");
        assert_eq!(
            base.url(),
            "https://huggingface.co/Intel/whisper.cpp-openvino-models/resolve/main/ggml-base-models.zip"
        );
        assert!(package_for_model(Path::new("ggml-large-v3-turbo-q5_0.bin")).is_none());
        assert!(package_for_model(Path::new("ggml-tiny.bin")).is_none());
    }

    #[test]
    fn finds_encoder_by_model_or_family() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("ggml-base-q5_1.bin");
        assert_eq!(find_encoder(&model), None);
        fs::write(dir.path().join("ggml-base-encoder-openvino.xml"), "x").unwrap();
        // Serve anche il file .bin dei pesi.
        assert_eq!(find_encoder(&model), None);
        fs::write(dir.path().join("ggml-base-encoder-openvino.bin"), "x").unwrap();
        assert_eq!(
            find_encoder(&model),
            Some(dir.path().join("ggml-base-encoder-openvino.xml"))
        );
        // Un encoder con il nome esatto del modello ha la precedenza.
        fs::write(dir.path().join("ggml-base-q5_1-encoder-openvino.xml"), "x").unwrap();
        fs::write(dir.path().join("ggml-base-q5_1-encoder-openvino.bin"), "x").unwrap();
        assert_eq!(
            find_encoder(&model),
            Some(dir.path().join("ggml-base-q5_1-encoder-openvino.xml"))
        );
    }

    #[test]
    fn device_labels() {
        let npu = Device {
            id: "NPU".into(),
            name: "Intel(R) AI Boost".into(),
        };
        assert_eq!(npu.label(), "Acceleratore neurale (NPU): Intel(R) AI Boost (NPU)");
        let gpu = Device {
            id: "GPU.0".into(),
            name: String::new(),
        };
        assert_eq!(gpu.label(), "Scheda video: GPU.0");
    }

    #[test]
    fn only_intel_gpus_are_supported() {
        let device = |id: &str, name: &str| Device {
            id: id.into(),
            name: name.into(),
        };
        assert!(is_supported(&device("GPU.0", "Intel(R) Arc(TM) Graphics (iGPU)")));
        assert!(!is_supported(&device("GPU", "gfx1030 (dGPU)")));
        assert!(!is_supported(&device("GPU.1", "NVIDIA GeForce RTX 4070")));
        assert!(is_supported(&device("NPU", "Intel(R) AI Boost")));
        assert!(is_supported(&device("CPU", "AMD Ryzen 7 3800X 8-Core Processor")));
    }

    #[test]
    fn pending_install_replaces_old_runtime() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("openvino");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(C_LIBRARY), "vecchia").unwrap();
        // Preparazione incompleta (manca versione.txt): non viene applicata.
        let staging = staging_dir(&dir);
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join(C_LIBRARY), "nuova").unwrap();
        apply_pending_install_in(&dir);
        assert_eq!(fs::read_to_string(dir.join(C_LIBRARY)).unwrap(), "vecchia");
        // Preparazione completa: sostituisce la versione precedente.
        fs::write(staging.join(VERSION_FILE), RUNTIME_VERSION).unwrap();
        apply_pending_install_in(&dir);
        assert_eq!(fs::read_to_string(dir.join(C_LIBRARY)).unwrap(), "nuova");
        assert!(!staging.exists());
    }

    #[test]
    fn sha256_of_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, "hello world").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }
}
