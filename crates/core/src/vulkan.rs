//! Caricatore Vulkan (`vulkan-1.dll`) a caricamento ritardato su Windows.
//!
//! ggml ottiene quasi tutte le funzioni Vulkan tramite `vkGetInstanceProcAddr`; importa
//! direttamente solo quella e due funzioni (`vkGetPhysicalDeviceFeatures2`, `vkCmdCopyBuffer`)
//! usate dopo aver creato l'istanza Vulkan. Gli eseguibili sono collegati con
//! `/DELAYLOAD:vulkan-1.dll` (vedi `build.rs` della GUI e della CLI): la DLL viene caricata alla
//! prima chiamata di ggml e non all'avvio, quindi il programma parte anche dove i driver Vulkan
//! non sono installati.
//!
//! Prima che ggml usi Vulkan, l'hook del caricamento ritardato ([`vulkan_delay_load_hook!`])
//! decide quale DLL usare:
//! - quella della cartella di sistema, installata dai driver della scheda video. Non si cerca
//!   mai nella cartella del programma: su una chiavetta USB chiunque potrebbe affiancargli una
//!   DLL con quel nome;
//! - se manca (o se è impostata [`DISABLE_ENV`]), un `vkGetInstanceProcAddr` sostitutivo le cui
//!   funzioni falliscono con `VK_ERROR_INITIALIZATION_FAILED`. ggml lo tratta come «nessuna
//!   scheda video Vulkan» e la trascrizione usa il processore.
//!
//! `vulkan-1.dll` (Khronos Vulkan Loader, licenza Apache 2.0) non viene quindi distribuita con il
//! programma: la installano tutti i driver recenti delle schede AMD, NVIDIA e Intel.

/// Variabile d'ambiente che, se impostata a un valore diverso da `0`, impedisce di usare Vulkan
/// (utile se un driver difettoso si blocca all'avvio della scheda video).
pub const DISABLE_ENV: &str = "TRASCRITTORE_NO_VULKAN";

/// Disponibilità del caricatore Vulkan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderStatus {
    /// Questa versione è compilata senza Vulkan.
    NotIncluded,
    /// Vulkan disattivato con [`DISABLE_ENV`].
    Disabled,
    /// `vulkan-1.dll` non è installata nella cartella di sistema.
    Missing,
    Available,
}

/// Stato del caricatore Vulkan. Il controllo viene fatto una sola volta per esecuzione.
pub fn status() -> LoaderStatus {
    if !cfg!(feature = "vulkan") {
        LoaderStatus::NotIncluded
    } else if disabled() {
        LoaderStatus::Disabled
    } else if loader_available() {
        LoaderStatus::Available
    } else {
        LoaderStatus::Missing
    }
}

fn disabled() -> bool {
    std::env::var_os(DISABLE_ENV).is_some_and(|v| !v.is_empty() && v != "0")
}

#[cfg(all(windows, target_env = "msvc"))]
fn loader_available() -> bool {
    delay_load::system_loader().is_some()
}

#[cfg(not(all(windows, target_env = "msvc")))]
fn loader_available() -> bool {
    true
}

/// Definisce l'hook del caricamento ritardato di Windows (`__pfnDliNotifyHook2`).
///
/// Va invocata una volta nel crate di ogni eseguibile: il simbolo deve far parte del programma
/// stesso, altrimenti il linker potrebbe preferire la definizione vuota di `delayimp.lib`.
#[macro_export]
macro_rules! vulkan_delay_load_hook {
    () => {
        #[cfg(all(windows, target_env = "msvc"))]
        #[allow(non_upper_case_globals)]
        #[no_mangle]
        #[used]
        static __pfnDliNotifyHook2: $crate::vulkan::delay_load::Hook =
            $crate::vulkan::delay_load::notify;
    };
}

#[cfg(all(windows, target_env = "msvc"))]
#[doc(hidden)]
pub mod delay_load {
    //! Hook per l'helper di caricamento ritardato di MSVC (`delayimp.h`).

    use std::ffi::{c_char, c_void, CStr};
    use std::ptr;
    use std::sync::OnceLock;

    /// `PfnDliHook`.
    pub type Hook = unsafe extern "system" fn(u32, *mut DelayLoadInfo) -> *const c_void;

    /// `DelayLoadProc`.
    #[repr(C)]
    pub struct DelayLoadProc {
        import_by_name: i32,
        /// Unione con l'ordinale, valido solo se `import_by_name` è vero.
        name: *const c_char,
    }

    /// `DelayLoadInfo`.
    #[repr(C)]
    pub struct DelayLoadInfo {
        size: u32,
        descriptor: *const c_void,
        import_address: *mut *const c_void,
        dll_name: *const c_char,
        procedure: DelayLoadProc,
        module: *mut c_void,
        function: *const c_void,
        last_error: u32,
    }

    const DLI_NOTE_PRE_LOAD_LIBRARY: u32 = 1;
    const DLI_NOTE_PRE_GET_PROC_ADDRESS: u32 = 2;
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;
    const VK_ERROR_INITIALIZATION_FAILED: i32 = -3;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }

    /// `vulkan-1.dll` della cartella di sistema, caricata alla prima richiesta.
    pub(super) fn system_loader() -> Option<*mut c_void> {
        // HMODULE conservato come intero perché i puntatori non sono `Sync`.
        static LOADER: OnceLock<usize> = OnceLock::new();
        let module = *LOADER.get_or_init(|| {
            if super::disabled() {
                return 0;
            }
            let name: Vec<u16> = "vulkan-1.dll\0".encode_utf16().collect();
            // SAFETY: nome terminato da zero; il modulo resta caricato per tutta l'esecuzione.
            let module = unsafe {
                LoadLibraryExW(name.as_ptr(), ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32)
            };
            module as usize
        });
        (module != 0).then_some(module as *mut c_void)
    }

    /// Chiamata dall'helper di caricamento ritardato prima di caricare una DLL e prima di
    /// cercarne una funzione. Restituire null lascia procedere l'helper normalmente.
    ///
    /// # Safety
    /// `info` deve essere il `DelayLoadInfo` valido passato dall'helper di MSVC.
    pub unsafe extern "system" fn notify(event: u32, info: *mut DelayLoadInfo) -> *const c_void {
        // SAFETY: l'helper passa sempre una struttura valida, con il nome della DLL.
        let info = unsafe { &*info };
        if info.dll_name.is_null()
            || !unsafe { CStr::from_ptr(info.dll_name) }
                .to_bytes()
                .eq_ignore_ascii_case(b"vulkan-1.dll")
        {
            return ptr::null();
        }
        match (event, system_loader()) {
            (DLI_NOTE_PRE_LOAD_LIBRARY, Some(loader)) => loader,
            // Al posto della DLL mancante si indica il programma stesso: l'helper chiederà poi
            // le funzioni, che vengono sostituite qui sotto.
            // SAFETY: con un nome null GetModuleHandleW restituisce l'eseguibile.
            (DLI_NOTE_PRE_LOAD_LIBRARY, None) => unsafe { GetModuleHandleW(ptr::null()) },
            (DLI_NOTE_PRE_GET_PROC_ADDRESS, None) => {
                let wanted = info.procedure.import_by_name != 0
                    && !info.procedure.name.is_null()
                    // SAFETY: con l'importazione per nome `name` è una stringa C valida.
                    && unsafe { CStr::from_ptr(info.procedure.name) }.to_bytes()
                        == b"vkGetInstanceProcAddr";
                if wanted {
                    missing_get_instance_proc_addr as *const c_void
                } else {
                    missing_function as *const c_void
                }
            }
            _ => ptr::null(),
        }
    }

    /// Sostituto di `vkGetInstanceProcAddr` quando il caricatore Vulkan non è disponibile.
    ///
    /// Restituisce per ogni nome una funzione che fallisce: prima di creare l'istanza ggml
    /// chiede solo funzioni globali che restituiscono un `VkResult` (`vkEnumerateInstanceVersion`,
    /// `vkCreateInstance`...); il primo errore interrompe l'inizializzazione di Vulkan, quindi le
    /// funzioni che richiedono un'istanza (comprese quelle importate direttamente) non vengono
    /// mai chiamate.
    unsafe extern "system" fn missing_get_instance_proc_addr(
        _instance: *mut c_void,
        _name: *const c_char,
    ) -> *const c_void {
        missing_function as *const c_void
    }

    /// Funzione Vulkan sostitutiva. Con la convenzione di chiamata x64 di Windows gli argomenti
    /// passati dal chiamante possono essere ignorati senza conseguenze.
    unsafe extern "system" fn missing_function() -> i32 {
        VK_ERROR_INITIALIZATION_FAILED
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn info(dll: &'static [u8], function: &'static [u8]) -> DelayLoadInfo {
            DelayLoadInfo {
                size: std::mem::size_of::<DelayLoadInfo>() as u32,
                descriptor: ptr::null(),
                import_address: ptr::null_mut(),
                dll_name: dll.as_ptr().cast(),
                procedure: DelayLoadProc {
                    import_by_name: 1,
                    name: function.as_ptr().cast(),
                },
                module: ptr::null_mut(),
                function: ptr::null(),
                last_error: 0,
            }
        }

        #[test]
        fn layout_matches_delayimp() {
            // DelayLoadInfo di delayimp.h su x64: 72 byte, hmodCur a 48.
            assert_eq!(std::mem::size_of::<DelayLoadInfo>(), 72);
            assert_eq!(std::mem::offset_of!(DelayLoadInfo, module), 48);
        }

        #[test]
        fn ignores_other_dlls() {
            let mut info = info(b"altra.dll\0", b"Funzione\0");
            for event in 0..=5 {
                // SAFETY: struttura valida per tutta la chiamata.
                assert!(unsafe { notify(event, &mut info) }.is_null());
            }
        }

        #[test]
        fn vulkan_loader_or_fallback() {
            let mut info = info(b"VULKAN-1.DLL\0", b"vkGetInstanceProcAddr\0");
            // SAFETY: struttura valida per tutta la chiamata.
            let module = unsafe { notify(DLI_NOTE_PRE_LOAD_LIBRARY, &mut info) };
            assert!(!module.is_null(), "serve sempre un modulo, vero o sostitutivo");
            // SAFETY: come sopra.
            let function = unsafe { notify(DLI_NOTE_PRE_GET_PROC_ADDRESS, &mut info) };
            if system_loader().is_some() {
                assert!(function.is_null(), "con il caricatore si usa la funzione vera");
                return;
            }
            // SAFETY: il sostituto ha la firma di vkGetInstanceProcAddr.
            let get_proc: unsafe extern "system" fn(*mut c_void, *const c_char) -> *const c_void =
                unsafe { std::mem::transmute(function) };
            let create = unsafe { get_proc(ptr::null_mut(), c"vkCreateInstance".as_ptr()) };
            // SAFETY: le funzioni sostitutive restituiscono un VkResult.
            let create: unsafe extern "system" fn() -> i32 = unsafe { std::mem::transmute(create) };
            assert_eq!(unsafe { create() }, VK_ERROR_INITIALIZATION_FAILED);
        }

        #[test]
        fn stubs_fail_cleanly() {
            // SAFETY: funzioni sostitutive senza effetti collaterali.
            let function = unsafe { missing_get_instance_proc_addr(ptr::null_mut(), ptr::null()) };
            assert!(!function.is_null(), "una funzione null farebbe bloccare ggml");
            // SAFETY: le funzioni sostitutive restituiscono un VkResult.
            let function: unsafe extern "system" fn() -> i32 =
                unsafe { std::mem::transmute(function) };
            assert_eq!(unsafe { function() }, VK_ERROR_INITIALIZATION_FAILED);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_matches_build() {
        let status = status();
        if cfg!(feature = "vulkan") {
            assert_ne!(status, LoaderStatus::NotIncluded);
        } else {
            assert_eq!(status, LoaderStatus::NotIncluded);
        }
    }
}
