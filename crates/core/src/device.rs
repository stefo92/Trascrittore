//! Scelta del dispositivo di calcolo: processore (CPU) o scheda video (GPU).
//!
//! Le schede video sono quelle viste dai backend GPU compilati in whisper.cpp:
//! con la feature `vulkan` funzionano schede AMD, NVIDIA e Intel; con `cuda` solo NVIDIA.

use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Dispositivo richiesto dall'utente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Device {
    /// La prima scheda video dedicata (o integrata) disponibile, altrimenti il processore.
    #[default]
    Auto,
    /// Solo il processore.
    Cpu,
    /// Una scheda video, con l'indice usato da whisper.cpp (conteggio delle sole GPU).
    Gpu(u32),
}

/// Produttore della scheda video, dedotto dalla descrizione del dispositivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Amd,
    Nvidia,
    Intel,
    Other,
}

impl Vendor {
    fn detect(description: &str) -> Self {
        let d = description.to_ascii_lowercase();
        if d.contains("amd") || d.contains("radeon") {
            Vendor::Amd
        } else if d.contains("nvidia") || d.contains("geforce") || d.contains("quadro") {
            Vendor::Nvidia
        } else if d.contains("intel") {
            Vendor::Intel
        } else {
            Vendor::Other
        }
    }

    /// Nome del produttore.
    pub fn name(self) -> &'static str {
        match self {
            Vendor::Amd => "AMD",
            Vendor::Nvidia => "NVIDIA",
            Vendor::Intel => "Intel",
            Vendor::Other => "",
        }
    }
}

/// Una scheda video utilizzabile per la trascrizione.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuInfo {
    /// Indice da passare a whisper.cpp.
    pub index: u32,
    /// Backend di calcolo (es. «Vulkan», «CUDA»).
    pub backend: String,
    /// Nome della scheda (es. «AMD Radeon RX 6800 XT»).
    pub description: String,
    pub vendor: Vendor,
    /// Memoria video totale in byte (0 se sconosciuta).
    pub memory_bytes: u64,
    /// `true` per le schede integrate nel processore.
    pub integrated: bool,
}

impl GpuInfo {
    /// Descrizione completa in italiano, adatta a menu e sintesi vocale.
    pub fn label(&self) -> String {
        let kind = if self.integrated {
            "Scheda video integrata"
        } else {
            "Scheda video"
        };
        format!("{kind}: {}", self.short_label())
    }

    /// Nome e dettagli senza il prefisso «Scheda video», per elenchi già etichettati.
    pub fn short_label(&self) -> String {
        let mut details = Vec::new();
        if self.integrated {
            details.push("integrata".to_string());
        }
        if self.memory_bytes > 0 {
            details.push(format_memory(self.memory_bytes));
        }
        details.push(self.backend.clone());
        format!("{} ({})", self.description, details.join(", "))
    }
}

/// Dispositivo effettivo dopo aver risolto la scelta «Automatico».
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedDevice {
    Cpu,
    Gpu(GpuInfo),
}

impl ResolvedDevice {
    /// Descrizione breve in italiano.
    pub fn label(&self) -> String {
        match self {
            ResolvedDevice::Cpu => "processore (CPU)".to_string(),
            ResolvedDevice::Gpu(gpu) => gpu.description.clone(),
        }
    }
}

impl Device {
    /// Risolve la scelta rispetto alle schede video presenti.
    pub fn resolve(self, gpus: &[GpuInfo]) -> Result<ResolvedDevice> {
        match self {
            Device::Cpu => Ok(ResolvedDevice::Cpu),
            Device::Auto => Ok(gpus
                .iter()
                .find(|g| !g.integrated)
                .or_else(|| gpus.first())
                .cloned()
                .map_or(ResolvedDevice::Cpu, ResolvedDevice::Gpu)),
            Device::Gpu(index) => match gpus.iter().find(|g| g.index == index) {
                Some(gpu) => Ok(ResolvedDevice::Gpu(gpu.clone())),
                None if gpus.is_empty() => bail!(
                    "Nessuna scheda video utilizzabile: questa versione non include il supporto GPU oppure i driver non lo consentono. Scegli «Processore (CPU)» o «Automatico»."
                ),
                None => bail!(
                    "La scheda video numero {index} non è disponibile. Scegli un'altra scheda, «Processore (CPU)» o «Automatico»."
                ),
            },
        }
    }
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Device::Auto => f.write_str("auto"),
            Device::Cpu => f.write_str("cpu"),
            Device::Gpu(index) => write!(f, "gpu:{index}"),
        }
    }
}

impl FromStr for Device {
    type Err = anyhow::Error;

    /// Accetta `auto`, `cpu`, `gpu` (prima scheda video), `gpu:N` o `gpuN`.
    fn from_str(s: &str) -> Result<Self> {
        let s = s.trim().to_ascii_lowercase();
        match s.as_str() {
            "auto" | "automatico" => Ok(Device::Auto),
            "cpu" | "processore" => Ok(Device::Cpu),
            "gpu" => Ok(Device::Gpu(0)),
            other => {
                let number = other
                    .strip_prefix("gpu")
                    .map(|rest| rest.trim_start_matches([':', '=']))
                    .unwrap_or(other);
                match number.parse::<u32>() {
                    Ok(index) => Ok(Device::Gpu(index)),
                    Err(_) => bail!(
                        "Dispositivo «{other}» non valido. Usa auto, cpu, gpu oppure gpu:N (vedi il comando «dispositivi»)."
                    ),
                }
            }
        }
    }
}

/// Backend GPU inclusi in questa versione del programma.
pub fn compiled_gpu_backends() -> Vec<&'static str> {
    let mut backends = Vec::new();
    if cfg!(feature = "vulkan") {
        backends.push("Vulkan");
    }
    if cfg!(feature = "cuda") {
        backends.push("CUDA");
    }
    backends
}

/// Schede video utilizzabili. L'elenco viene calcolato una sola volta per esecuzione,
/// perché l'inizializzazione dei backend GPU richiede tempo.
pub fn list_gpus() -> &'static [GpuInfo] {
    static GPUS: OnceLock<Vec<GpuInfo>> = OnceLock::new();
    GPUS.get_or_init(enumerate)
}

#[cfg(feature = "whisper")]
fn enumerate() -> Vec<GpuInfo> {
    use std::ffi::CStr;
    use whisper_rs::whisper_rs_sys as sys;

    crate::transcriber::silence_engine_logs();

    let text = |ptr: *const std::os::raw::c_char| {
        if ptr.is_null() {
            String::new()
        } else {
            // SAFETY: ggml restituisce stringhe C terminate da zero e valide per tutta l'esecuzione.
            unsafe { CStr::from_ptr(ptr) }.to_string_lossy().trim().to_string()
        }
    };

    let mut gpus = Vec::new();
    // SAFETY: funzioni di sola lettura del registro dei dispositivi di ggml; gli indici
    // usati sono minori di `ggml_backend_dev_count()`.
    unsafe {
        let mut index = 0u32;
        for i in 0..sys::ggml_backend_dev_count() {
            let device = sys::ggml_backend_dev_get(i);
            if device.is_null() {
                continue;
            }
            let kind = sys::ggml_backend_dev_type(device);
            let integrated = kind == sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU;
            // Stesso criterio di whisper.cpp per numerare le GPU.
            if kind != sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU && !integrated {
                continue;
            }
            let name = text(sys::ggml_backend_dev_name(device));
            let mut description = text(sys::ggml_backend_dev_description(device));
            if description.is_empty() {
                description = name.clone();
            }
            let mut free = 0usize;
            let mut total = 0usize;
            sys::ggml_backend_dev_memory(device, &mut free, &mut total);
            gpus.push(GpuInfo {
                index,
                backend: name.trim_end_matches(|c: char| c.is_ascii_digit()).to_string(),
                vendor: Vendor::detect(&description),
                description,
                memory_bytes: total as u64,
                integrated,
            });
            index += 1;
        }
    }
    gpus
}

#[cfg(not(feature = "whisper"))]
fn enumerate() -> Vec<GpuInfo> {
    Vec::new()
}

/// Memoria (libera, totale) in byte della scheda video con l'indice di whisper.cpp.
#[cfg(feature = "whisper")]
pub fn memory(gpu_index: u32) -> Option<(u64, u64)> {
    use whisper_rs::whisper_rs_sys as sys;

    // SAFETY: stesse funzioni di sola lettura usate in `enumerate`.
    unsafe {
        let mut index = 0u32;
        for i in 0..sys::ggml_backend_dev_count() {
            let device = sys::ggml_backend_dev_get(i);
            if device.is_null() {
                continue;
            }
            let kind = sys::ggml_backend_dev_type(device);
            if kind != sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU
                && kind != sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU
            {
                continue;
            }
            if index == gpu_index {
                let mut free = 0usize;
                let mut total = 0usize;
                sys::ggml_backend_dev_memory(device, &mut free, &mut total);
                return Some((free as u64, total as u64));
            }
            index += 1;
        }
    }
    None
}

#[cfg(not(feature = "whisper"))]
pub fn memory(_gpu_index: u32) -> Option<(u64, u64)> {
    None
}

/// Formatta la memoria video (es. «16 GB»).
fn format_memory(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let gib = bytes as f64 / GIB;
    if gib >= 1.0 {
        format!("{} GB di memoria", gib.round() as u64)
    } else {
        format!("{} MB di memoria", bytes / (1024 * 1024))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(index: u32, description: &str, integrated: bool) -> GpuInfo {
        GpuInfo {
            index,
            backend: "Vulkan".into(),
            description: description.into(),
            vendor: Vendor::detect(description),
            memory_bytes: 16 * 1024 * 1024 * 1024,
            integrated,
        }
    }

    #[test]
    fn parse_devices() {
        assert_eq!("auto".parse::<Device>().unwrap(), Device::Auto);
        assert_eq!("CPU".parse::<Device>().unwrap(), Device::Cpu);
        assert_eq!("gpu".parse::<Device>().unwrap(), Device::Gpu(0));
        assert_eq!("gpu:1".parse::<Device>().unwrap(), Device::Gpu(1));
        assert_eq!("gpu2".parse::<Device>().unwrap(), Device::Gpu(2));
        assert_eq!("3".parse::<Device>().unwrap(), Device::Gpu(3));
        assert!("scheda".parse::<Device>().is_err());
        assert_eq!(Device::Gpu(1).to_string(), "gpu:1");
    }

    #[test]
    fn auto_prefers_dedicated_gpu() {
        let gpus = vec![gpu(0, "Intel(R) UHD Graphics", true), gpu(1, "AMD Radeon RX 6800 XT", false)];
        match Device::Auto.resolve(&gpus).unwrap() {
            ResolvedDevice::Gpu(g) => assert_eq!(g.index, 1),
            other => panic!("atteso GPU, ottenuto {other:?}"),
        }
        assert_eq!(Device::Auto.resolve(&[]).unwrap(), ResolvedDevice::Cpu);
        assert_eq!(Device::Cpu.resolve(&gpus).unwrap(), ResolvedDevice::Cpu);
        assert!(Device::Gpu(5).resolve(&gpus).is_err());
        assert!(Device::Gpu(0).resolve(&[]).is_err());
    }

    #[test]
    fn vendor_and_label() {
        let g = gpu(0, "NVIDIA GeForce RTX 4070", false);
        assert_eq!(g.vendor, Vendor::Nvidia);
        assert_eq!(
            g.label(),
            "Scheda video: NVIDIA GeForce RTX 4070 (16 GB di memoria, Vulkan)"
        );
        assert_eq!(Vendor::detect("AMD Radeon RX 6800 XT"), Vendor::Amd);
    }

    #[test]
    fn settings_roundtrip() {
        let json = serde_json::to_string(&Device::Gpu(1)).unwrap();
        assert_eq!(serde_json::from_str::<Device>(&json).unwrap(), Device::Gpu(1));
        assert_eq!(serde_json::from_str::<Device>("\"auto\"").unwrap(), Device::Auto);
    }
}
