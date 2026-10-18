//! Finestra principale, stato dell'applicazione e comunicazione con il thread di lavoro.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{anyhow, Result};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, GetDC, GetSysColorBrush, GetTextExtentPoint32W,
    ReleaseDC, SelectObject, UpdateWindow, COLOR_BTNFACE, HFONT, HGDIOBJ,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, EM_GETMODIFY, EM_GETSEL, EM_SCROLLCARET, EM_SETLIMITTEXT,
    EM_SETMODIFY, EM_SETSEL, ICC_PROGRESS_CLASS, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX,
    PBM_SETPOS, PBM_SETRANGE32, PROGRESS_CLASSW,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SystemParametersInfoForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, IsWindowEnabled, SetFocus, VK_CONTROL,
};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::*;

use trascrittore_core::device::{self, Device, ResolvedDevice};
use trascrittore_core::document::{Document, DocumentInfo};
use trascrittore_core::export::{self, ExportFormat};
use trascrittore_core::models::{self, ModelInfo};
use trascrittore_core::pipeline::{self, Job, Outcome, Progress, ProgressFn};
use trascrittore_core::settings::Settings;
use trascrittore_core::transcriber::{self, TranscribeOptions, Transcriber};
use trascrittore_core::transcript::{format_duration_words, Transcript};
use trascrittore_core::{audio, is_cancelled, languages};

use crate::a11y::{self, Annotator};
use crate::dialogs::{self, Filter};
use crate::win::{self, hiword, loword};

/// Messaggio inviato dal thread di lavoro quando ci sono eventi da leggere.
const WM_APP_WORKER: u32 = WM_APP + 1;

// Identificativi dei controlli.
const IDC_BROWSE: u16 = 102;
const IDC_MODEL: u16 = 104;
const IDC_LANG: u16 = 106;
const IDC_TRANSLATE: u16 = 107;
const IDC_TIMESTAMPS: u16 = 108;
const IDC_START: u16 = 109;
const IDC_CANCEL: u16 = 110;
const IDC_TEXT: u16 = 116;
const IDC_FORMAT: u16 = 118;
const IDC_EXPORT: u16 = 119;
const IDC_DEVICE: u16 = 120;

// Identificativi dei comandi di menu.
const IDM_OPEN: u16 = 201;
const IDM_EXPORT: u16 = 202;
const IDM_EXIT: u16 = 203;
const IDM_START: u16 = 211;
const IDM_CANCEL: u16 = 212;
const IDM_DOWNLOAD: u16 = 213;
const IDM_CUSTOM_MODEL: u16 = 214;
const IDM_MODELS_DIR: u16 = 215;
const IDM_STATUS: u16 = 216;
const IDM_COPY: u16 = 221;
const IDM_ZOOM_IN: u16 = 222;
const IDM_ZOOM_OUT: u16 = 223;
const IDM_ZOOM_RESET: u16 = 224;
const IDM_HELP: u16 = 231;
const IDM_ABOUT: u16 = 232;

/// IDCANCEL: inviato da IsDialogMessage quando si preme Esc.
const ID_ESCAPE: u16 = 2;

/// Stile SS_NOPREFIX: la «&» nel testo di stato non diventa un tasto di scelta rapida.
const SS_NOPREFIX: u32 = 0x80;

const MIN_FONT_POINTS: u32 = 8;
const MAX_FONT_POINTS: u32 = 40;
const DEFAULT_FONT_POINTS: u32 = 12;

const HELP_TEXT: &str = "Uso di Trascrittore\n\n\
1. Premi Ctrl+O (oppure il pulsante Sfoglia) e scegli un file audio.\n\
2. Scegli modello e lingua. Il modello consigliato è «Large v3 Turbo compresso»;\n\
   i modelli «compressi» più piccoli sono più veloci ma meno precisi.\n\
   In «Dispositivo» (Alt+P) puoi usare la scheda video, molto più veloce del processore.\n\
3. Premi F5 per avviare la trascrizione. Esc la annulla.\n\
4. Al termine il cursore si sposta sul testo trascritto: puoi leggerlo e correggerlo.\n\
5. Scegli il formato e premi Ctrl+S per esportare.\n\n\
Tasti rapidi\n\
Ctrl+O: apri file audio\n\
F5: avvia la trascrizione\n\
Esc: annulla l'operazione in corso\n\
Ctrl+S: esporta\n\
Ctrl+Maiusc+C: copia tutto il testo\n\
F2: leggi lo stato corrente\n\
Ctrl+più / Ctrl+meno / Ctrl+0: dimensione del testo\n\
Alt+lettera sottolineata: vai al campo corrispondente\n\
F1: questa guida";

/// Fase di un'operazione in corso.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Downloading,
    Decoding,
    LoadingModel,
    Transcribing,
}

impl Phase {
    fn description(self) -> &'static str {
        match self {
            Phase::Downloading => "Download del modello in corso",
            Phase::Decoding => "Decodifica dell'audio in corso",
            Phase::LoadingModel => "Caricamento del modello in memoria",
            Phase::Transcribing => "Trascrizione in corso",
        }
    }
}

/// Eventi dal thread di lavoro all'interfaccia.
enum WorkerEvent {
    Downloading(u32),
    Pipeline(Progress),
    Transcribed(Outcome),
    DownloadFinished,
    Failed(String),
    Cancelled,
}

/// Inoltra gli eventi all'interfaccia e la «sveglia» con un messaggio di finestra.
#[derive(Clone)]
struct Notifier {
    tx: Sender<WorkerEvent>,
    hwnd: isize,
}

impl Notifier {
    fn send(&self, event: WorkerEvent) {
        if self.tx.send(event).is_ok() {
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.hwnd as *mut c_void)),
                    WM_APP_WORKER,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }
}

/// Modello scelto nella casella combinata.
#[derive(Debug, Clone)]
enum ModelChoice {
    Catalog(&'static ModelInfo),
    Custom(PathBuf),
}

impl ModelChoice {
    fn display_name(&self) -> String {
        match self {
            ModelChoice::Catalog(info) => info.id.to_string(),
            ModelChoice::Custom(path) => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }
}

/// Trascrizione corrente con le informazioni necessarie all'esportazione.
struct CurrentTranscript {
    transcript: Transcript,
    model_name: String,
    translated: bool,
}

struct Controls {
    audio_label: HWND,
    audio_path: HWND,
    browse: HWND,
    model_label: HWND,
    model: HWND,
    lang_label: HWND,
    lang: HWND,
    device_label: HWND,
    device: HWND,
    translate: HWND,
    timestamps: HWND,
    start: HWND,
    cancel: HWND,
    progress_label: HWND,
    progress: HWND,
    status_label: HWND,
    status: HWND,
    text_label: HWND,
    text: HWND,
    format_label: HWND,
    format: HWND,
    export: HWND,
}

struct App {
    hwnd: HWND,
    menu: HMENU,
    c: Controls,
    ui_font: Cell<HFONT>,
    text_font: Cell<HFONT>,
    dpi: Cell<u32>,
    settings: RefCell<Settings>,
    audio_path: RefCell<Option<PathBuf>>,
    current: RefCell<Option<CurrentTranscript>>,
    /// Modello e traduzione della trascrizione in corso, applicati solo se riesce.
    pending: RefCell<Option<(String, bool)>>,
    exported: Cell<bool>,
    busy: Cell<bool>,
    phase: Cell<Option<Phase>>,
    last_milestone: Cell<u32>,
    started: Cell<Option<Instant>>,
    status: RefCell<String>,
    last_focus: Cell<HWND>,
    cancel_flag: RefCell<Arc<AtomicBool>>,
    events_tx: Sender<WorkerEvent>,
    events_rx: Receiver<WorkerEvent>,
    cache: Arc<Mutex<Option<Transcriber>>>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

fn app() -> Option<Rc<App>> {
    APP.with(|a| a.borrow().clone())
}

/// Avvia l'interfaccia e il ciclo dei messaggi.
pub fn run(initial_file: Option<PathBuf>) -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| anyhow!("COM non disponibile: {e}"))?;
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_PROGRESS_CLASS,
        };
        let _ = InitCommonControlsEx(&icc);

        let instance = GetModuleHandleW(None)?;
        let class_name = w!("TrascrittoreFinestraPrincipale");
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            hIcon: LoadIconW(None, IDI_APPLICATION)?,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszClassName: class_name,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(anyhow!("Registrazione della classe di finestra non riuscita."));
        }

        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class_name,
            w!("Trascrittore"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        let app = Rc::new(App::create(hwnd)?);
        APP.with(|a| *a.borrow_mut() = Some(Rc::clone(&app)));
        app.initialize(initial_file);

        let accelerators = create_accelerators()?;
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if TranslateAcceleratorW(hwnd, accelerators, &msg) != 0 {
                continue;
            }
            if IsDialogMessageW(hwnd, &msg).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = DestroyAcceleratorTable(accelerators);
        APP.with(|a| *a.borrow_mut() = None);
    }
    Ok(())
}

unsafe fn create_accelerators() -> Result<HACCEL> {
    let ctrl = FCONTROL | FVIRTKEY;
    let table = [
        ACCEL { fVirt: ctrl, key: u16::from(b'O'), cmd: IDM_OPEN },
        ACCEL { fVirt: ctrl, key: u16::from(b'S'), cmd: IDM_EXPORT },
        ACCEL { fVirt: ctrl | FSHIFT, key: u16::from(b'C'), cmd: IDM_COPY },
        ACCEL { fVirt: FVIRTKEY, key: 0x74 /* F5 */, cmd: IDM_START },
        ACCEL { fVirt: FVIRTKEY, key: 0x71 /* F2 */, cmd: IDM_STATUS },
        ACCEL { fVirt: FVIRTKEY, key: 0x70 /* F1 */, cmd: IDM_HELP },
        ACCEL { fVirt: ctrl, key: 0xBB /* + */, cmd: IDM_ZOOM_IN },
        ACCEL { fVirt: ctrl, key: 0x6B /* + tastierino */, cmd: IDM_ZOOM_IN },
        ACCEL { fVirt: ctrl, key: 0xBD /* - */, cmd: IDM_ZOOM_OUT },
        ACCEL { fVirt: ctrl, key: 0x6D /* - tastierino */, cmd: IDM_ZOOM_OUT },
        ACCEL { fVirt: ctrl, key: u16::from(b'0'), cmd: IDM_ZOOM_RESET },
        ACCEL { fVirt: ctrl, key: 0x60 /* 0 tastierino */, cmd: IDM_ZOOM_RESET },
    ];
    Ok(CreateAcceleratorTableW(&table)?)
}

unsafe fn create_menu() -> Result<HMENU> {
    let bar = CreateMenu()?;

    let file = CreatePopupMenu()?;
    AppendMenuW(file, MF_STRING, IDM_OPEN as usize, w!("&Apri file audio...\tCtrl+O"))?;
    AppendMenuW(file, MF_STRING, IDM_EXPORT as usize, w!("&Esporta...\tCtrl+S"))?;
    AppendMenuW(file, MF_SEPARATOR, 0, PCWSTR::null())?;
    AppendMenuW(file, MF_STRING, IDM_EXIT as usize, w!("E&sci\tAlt+F4"))?;
    AppendMenuW(bar, MF_POPUP, file.0 as usize, w!("&File"))?;

    let edit = CreatePopupMenu()?;
    AppendMenuW(edit, MF_STRING, IDM_COPY as usize, w!("&Copia tutto il testo\tCtrl+Maiusc+C"))?;
    AppendMenuW(edit, MF_SEPARATOR, 0, PCWSTR::null())?;
    AppendMenuW(edit, MF_STRING, IDM_ZOOM_IN as usize, w!("&Ingrandisci il testo\tCtrl+più"))?;
    AppendMenuW(edit, MF_STRING, IDM_ZOOM_OUT as usize, w!("&Riduci il testo\tCtrl+meno"))?;
    AppendMenuW(edit, MF_STRING, IDM_ZOOM_RESET as usize, w!("Dimensione &normale\tCtrl+0"))?;
    AppendMenuW(bar, MF_POPUP, edit.0 as usize, w!("Mo&difica"))?;

    let job = CreatePopupMenu()?;
    AppendMenuW(job, MF_STRING, IDM_START as usize, w!("&Avvia\tF5"))?;
    AppendMenuW(job, MF_STRING, IDM_CANCEL as usize, w!("A&nnulla\tEsc"))?;
    AppendMenuW(job, MF_SEPARATOR, 0, PCWSTR::null())?;
    AppendMenuW(job, MF_STRING, IDM_DOWNLOAD as usize, w!("&Scarica il modello selezionato"))?;
    AppendMenuW(job, MF_STRING, IDM_CUSTOM_MODEL as usize, w!("Usa un file &modello personalizzato..."))?;
    AppendMenuW(job, MF_STRING, IDM_MODELS_DIR as usize, w!("Apri la &cartella dei modelli"))?;
    AppendMenuW(job, MF_SEPARATOR, 0, PCWSTR::null())?;
    AppendMenuW(job, MF_STRING, IDM_STATUS as usize, w!("Leggi lo s&tato\tF2"))?;
    AppendMenuW(bar, MF_POPUP, job.0 as usize, w!("&Trascrizione"))?;

    let help = CreatePopupMenu()?;
    AppendMenuW(help, MF_STRING, IDM_HELP as usize, w!("&Guida rapida\tF1"))?;
    AppendMenuW(help, MF_STRING, IDM_ABOUT as usize, w!("&Informazioni su Trascrittore"))?;
    AppendMenuW(bar, MF_POPUP, help.0 as usize, w!("&Aiuto"))?;

    Ok(bar)
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let Some(app) = app() {
        if let Some(result) = app.handle_message(msg, wparam, lparam) {
            return result;
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Sottoclasse del campo di testo multiriga: Tab ed Esc escono dal campo invece di essere
/// inseriti nel testo, e Ctrl+A seleziona tutto.
unsafe extern "system" fn text_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_GETDLGCODE => {
            let mut code = DefSubclassProc(hwnd, msg, wparam, lparam).0;
            code &= !(DLGC_WANTTAB as isize);
            if lparam.0 != 0 {
                let message = &*(lparam.0 as *const MSG);
                let key = message.wParam.0;
                if message.message == WM_KEYDOWN && (key == 0x09 || key == 0x1B) {
                    code &= !(DLGC_WANTALLKEYS as isize | DLGC_WANTMESSAGE as isize);
                }
            }
            LRESULT(code)
        }
        WM_KEYDOWN if wparam.0 == usize::from(b'A') && GetKeyState(i32::from(VK_CONTROL.0)) < 0 => {
            win::send(hwnd, EM_SETSEL, 0, -1);
            LRESULT(0)
        }
        // Caratteri Ctrl+A ed Esc: niente segnale acustico.
        WM_CHAR if wparam.0 == 0x01 || wparam.0 == 0x1B => LRESULT(0),
        _ => DefSubclassProc(hwnd, msg, wparam, lparam),
    }
}

unsafe fn create_control(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    id: u16,
) -> Result<HWND> {
    let instance = GetModuleHandleW(None)?;
    Ok(CreateWindowExW(
        ex_style,
        class,
        &HSTRING::from(text),
        WS_CHILD | WS_VISIBLE | style,
        0,
        0,
        10,
        10,
        Some(parent),
        Some(HMENU(id as usize as *mut c_void)),
        Some(instance.into()),
        None,
    )?)
}

impl App {
    /// Crea i controlli. L'ordine di creazione determina l'ordine di tabulazione e,
    /// per i campi senza testo proprio, l'etichetta usata come nome accessibile.
    unsafe fn create(hwnd: HWND) -> Result<Self> {
        let label = |text: &str| {
            create_control(hwnd, w!("STATIC"), text, WINDOW_STYLE(0), WINDOW_EX_STYLE(0), 0)
        };
        let button = |text: &str, id: u16, extra: i32| {
            create_control(
                hwnd,
                w!("BUTTON"),
                text,
                WS_TABSTOP | WINDOW_STYLE(extra as u32),
                WINDOW_EX_STYLE(0),
                id,
            )
        };
        let combo = |id: u16| {
            create_control(
                hwnd,
                w!("COMBOBOX"),
                "",
                WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
                WINDOW_EX_STYLE(0),
                id,
            )
        };

        let audio_label = label("File a&udio:")?;
        let audio_path = create_control(
            hwnd,
            w!("EDIT"),
            "Nessun file selezionato",
            WS_TABSTOP | WINDOW_STYLE((ES_READONLY | ES_AUTOHSCROLL) as u32),
            WS_EX_CLIENTEDGE,
            101,
        )?;
        let browse = button("Sfo&glia...", IDC_BROWSE, BS_PUSHBUTTON)?;
        let model_label = label("&Modello:")?;
        let model = combo(IDC_MODEL)?;
        let lang_label = label("&Lingua:")?;
        let lang = combo(IDC_LANG)?;
        let device_label = label("Dis&positivo:")?;
        let device = combo(IDC_DEVICE)?;
        let translate = button("Traduci in &inglese", IDC_TRANSLATE, BS_AUTOCHECKBOX)?;
        let timestamps = button("Includi time&stamp", IDC_TIMESTAMPS, BS_AUTOCHECKBOX)?;
        let start = button("A&vvia trascrizione", IDC_START, BS_PUSHBUTTON)?;
        let cancel = button("A&nnulla", IDC_CANCEL, BS_PUSHBUTTON)?;
        let progress_label = label("Avanzamento:")?;
        let progress = create_control(
            hwnd,
            PROGRESS_CLASSW,
            "Avanzamento",
            WINDOW_STYLE(0),
            WINDOW_EX_STYLE(0),
            112,
        )?;
        let status_label = label("Stato:")?;
        let status = create_control(
            hwnd,
            w!("STATIC"),
            "Pronto.",
            WINDOW_STYLE(SS_NOPREFIX),
            WINDOW_EX_STYLE(0),
            114,
        )?;
        let text_label = label("Testo t&rascritto:")?;
        let text = create_control(
            hwnd,
            w!("EDIT"),
            "",
            WS_TABSTOP
                | WS_VSCROLL
                | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN | ES_NOHIDESEL) as u32),
            WS_EX_CLIENTEDGE,
            IDC_TEXT,
        )?;
        let format_label = label("F&ormato di esportazione:")?;
        let format = combo(IDC_FORMAT)?;
        let export = button("&Esporta...", IDC_EXPORT, BS_PUSHBUTTON)?;

        // Nessun limite di lunghezza al testo (il predefinito è 32 KB).
        win::send(text, EM_SETLIMITTEXT, 0, 0);
        let _ = SetWindowSubclass(text, Some(text_subclass_proc), 1, 0);
        win::send(progress, PBM_SETRANGE32, 0, 100);

        let annotator = Annotator::new();
        annotator.set_name(audio_path, "File audio");
        annotator.set_name(model, "Modello");
        annotator.set_name(lang, "Lingua");
        annotator.set_name(device, "Dispositivo di calcolo");
        annotator.set_description(
            device,
            if device::compiled_gpu_backends().is_empty() {
                "Questa versione usa solo il processore: non include il supporto per le schede video."
            } else {
                "La scheda video è molto più veloce del processore."
            },
        );
        annotator.set_name(progress, "Avanzamento");
        // Il controllo di stato non riceve un nome: per un testo statico il nome è il
        // contenuto stesso, e va letto così com'è.
        annotator.set_name(text, "Testo trascritto");
        annotator.set_description(
            text,
            "Puoi leggere e correggere il testo prima di esportarlo.",
        );
        annotator.set_name(format, "Formato di esportazione");
        annotator.set_description(start, "F5");
        annotator.set_description(cancel, "Esc");
        annotator.set_description(export, "Control+S");

        let menu = create_menu()?;
        SetMenu(hwnd, Some(menu))?;

        let (events_tx, events_rx) = mpsc::channel();
        let dpi = GetDpiForWindow(hwnd).max(96);

        Ok(Self {
            hwnd,
            menu,
            c: Controls {
                audio_label,
                audio_path,
                browse,
                model_label,
                model,
                lang_label,
                lang,
                device_label,
                device,
                translate,
                timestamps,
                start,
                cancel,
                progress_label,
                progress,
                status_label,
                status,
                text_label,
                text,
                format_label,
                format,
                export,
            },
            ui_font: Cell::new(HFONT::default()),
            text_font: Cell::new(HFONT::default()),
            dpi: Cell::new(dpi),
            settings: RefCell::new(Settings::load()),
            audio_path: RefCell::new(None),
            current: RefCell::new(None),
            pending: RefCell::new(None),
            exported: Cell::new(true),
            busy: Cell::new(false),
            phase: Cell::new(None),
            last_milestone: Cell::new(0),
            started: Cell::new(None),
            status: RefCell::new("Pronto.".to_string()),
            last_focus: Cell::new(browse),
            cancel_flag: RefCell::new(Arc::new(AtomicBool::new(false))),
            events_tx,
            events_rx,
            cache: Arc::new(Mutex::new(None)),
        })
    }

    fn all_controls(&self) -> [HWND; 22] {
        let c = &self.c;
        [
            c.audio_label,
            c.audio_path,
            c.browse,
            c.model_label,
            c.model,
            c.lang_label,
            c.lang,
            c.device_label,
            c.device,
            c.translate,
            c.timestamps,
            c.start,
            c.cancel,
            c.progress_label,
            c.progress,
            c.status_label,
            c.status,
            c.text_label,
            c.text,
            c.format_label,
            c.format,
            c.export,
        ]
    }

    /// Popola i controlli con le preferenze salvate e mostra la finestra.
    fn initialize(&self, initial_file: Option<PathBuf>) {
        self.update_fonts();
        let settings = self.settings.borrow().clone();

        self.fill_model_combo();

        win::combo_add(self.c.lang, "Rilevamento automatico");
        for lang in languages::LANGUAGES {
            win::combo_add(self.c.lang, lang.name);
        }
        let lang_index = settings
            .language
            .as_deref()
            .and_then(|code| languages::LANGUAGES.iter().position(|l| l.code == code))
            .map_or(0, |i| i + 1);
        win::combo_select(self.c.lang, lang_index);

        self.fill_device_combo(settings.device);

        for format in ExportFormat::ALL {
            win::combo_add(self.c.format, format.label());
        }
        let format_index = ExportFormat::ALL
            .iter()
            .position(|f| *f == settings.export_format)
            .unwrap_or(0);
        win::combo_select(self.c.format, format_index);

        win::set_checked(self.c.translate, settings.translate);
        win::set_checked(self.c.timestamps, settings.timestamps);

        self.set_idle_state();
        self.layout();
        unsafe {
            let w = self.px(900);
            let h = self.px(720);
            let _ = SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                w,
                h,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = UpdateWindow(self.hwnd);
        }

        if let Some(path) = initial_file.filter(|p| p.is_file()) {
            self.set_audio_file(path, false);
            self.focus(self.c.start);
        } else {
            self.focus(self.c.browse);
        }
    }

    fn fill_model_combo(&self) {
        let settings = self.settings.borrow();
        win::combo_reset(self.c.model);
        for info in models::MODELS {
            let state = if info.is_downloaded() {
                "scaricato"
            } else {
                "da scaricare"
            };
            win::combo_add(
                self.c.model,
                &format!(
                    "{} ({}, {state})",
                    info.description,
                    models::format_size(info.size_bytes)
                ),
            );
        }
        if let Some(custom) = &settings.custom_model_path {
            let name = custom
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            win::combo_add(self.c.model, &format!("File personalizzato: {name}"));
        }
        let index = if settings.custom_model_path.is_some() && settings.model_id.is_empty() {
            models::MODELS.len()
        } else {
            models::MODELS
                .iter()
                .position(|m| m.id == settings.model_id)
                .or_else(|| models::MODELS.iter().position(|m| m.id == models::DEFAULT_MODEL_ID))
                .unwrap_or(0)
        };
        win::combo_select(self.c.model, index);
    }

    fn selected_model(&self) -> Option<ModelChoice> {
        let index = win::combo_selected(self.c.model)?;
        if let Some(info) = models::MODELS.get(index) {
            return Some(ModelChoice::Catalog(info));
        }
        self.settings
            .borrow()
            .custom_model_path
            .clone()
            .map(ModelChoice::Custom)
    }

    /// Voci: «Automatico», «Processore (CPU)», poi una voce per ogni scheda video trovata.
    fn fill_device_combo(&self, selected: Device) {
        let gpus = device::list_gpus();
        let automatic = match Device::Auto.resolve(gpus) {
            Ok(ResolvedDevice::Gpu(gpu)) => format!("Automatico (ora: {})", gpu.description),
            _ => "Automatico (ora: processore)".to_string(),
        };
        win::combo_reset(self.c.device);
        win::combo_add(self.c.device, &automatic);
        win::combo_add(self.c.device, "Processore (CPU)");
        for gpu in gpus {
            win::combo_add(self.c.device, &gpu.label());
        }
        let index = match selected {
            Device::Auto => 0,
            Device::Cpu => 1,
            Device::Gpu(i) => gpus.iter().position(|g| g.index == i).map_or(0, |p| p + 2),
        };
        win::combo_select(self.c.device, index);
    }

    fn selected_device(&self) -> Device {
        match win::combo_selected(self.c.device) {
            Some(1) => Device::Cpu,
            Some(n) if n >= 2 => device::list_gpus()
                .get(n - 2)
                .map_or(Device::Auto, |g| Device::Gpu(g.index)),
            _ => Device::Auto,
        }
    }

    fn selected_language(&self) -> Option<String> {
        let index = win::combo_selected(self.c.lang)?;
        index
            .checked_sub(1)
            .and_then(|i| languages::LANGUAGES.get(i))
            .map(|l| l.code.to_string())
    }

    fn selected_format(&self) -> ExportFormat {
        win::combo_selected(self.c.format)
            .and_then(|i| ExportFormat::ALL.get(i).copied())
            .unwrap_or(ExportFormat::Docx)
    }

    // ----- Messaggi -----

    fn handle_message(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_COMMAND => {
                self.on_command(loword(wparam.0) as u16, hiword(wparam.0));
                Some(LRESULT(0))
            }
            WM_APP_WORKER => {
                self.drain_events();
                Some(LRESULT(0))
            }
            WM_SIZE => {
                self.layout();
                Some(LRESULT(0))
            }
            WM_GETMINMAXINFO => {
                let info = lparam.0 as *mut MINMAXINFO;
                if !info.is_null() {
                    unsafe {
                        (*info).ptMinTrackSize = POINT {
                            x: self.px(640),
                            y: self.px(540),
                        };
                    }
                }
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                self.dpi.set(hiword(wparam.0).max(96));
                self.update_fonts();
                let suggested = lparam.0 as *const RECT;
                if !suggested.is_null() {
                    unsafe {
                        let r = *suggested;
                        let _ = SetWindowPos(
                            self.hwnd,
                            None,
                            r.left,
                            r.top,
                            r.right - r.left,
                            r.bottom - r.top,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                }
                self.layout();
                Some(LRESULT(0))
            }
            WM_SETTINGCHANGE => {
                self.update_fonts();
                self.layout();
                None
            }
            WM_ACTIVATE => {
                if loword(wparam.0) == WA_INACTIVE {
                    let focused = unsafe { GetFocus() };
                    if self.is_own_control(focused) {
                        self.last_focus.set(focused);
                    }
                    None
                } else {
                    // Al ritorno nella finestra (Alt+Tab) il focus torna dov'era.
                    self.restore_focus();
                    Some(LRESULT(0))
                }
            }
            WM_SETFOCUS => {
                self.restore_focus();
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                if self.confirm_close() {
                    unsafe {
                        let _ = DestroyWindow(self.hwnd);
                    }
                }
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                self.save_settings();
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(self.ui_font.get().0));
                    let _ = DeleteObject(HGDIOBJ(self.text_font.get().0));
                    PostQuitMessage(0);
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn on_command(&self, id: u16, code: u32) {
        match id {
            IDC_BROWSE | IDM_OPEN if !self.busy.get() => self.choose_audio(),
            IDC_START | IDM_START if !self.busy.get() => self.start_transcription(),
            IDC_CANCEL | IDM_CANCEL | ID_ESCAPE => self.cancel_job(),
            IDC_EXPORT | IDM_EXPORT if !self.busy.get() => self.export(),
            IDC_TIMESTAMPS if code == BN_CLICKED => self.on_timestamps_toggled(),
            IDC_TRANSLATE if code == BN_CLICKED => {
                self.settings.borrow_mut().translate = win::is_checked(self.c.translate);
            }
            IDC_MODEL if code == CBN_SELCHANGE => self.remember_model_choice(),
            IDC_LANG if code == CBN_SELCHANGE => {
                self.settings.borrow_mut().language = self.selected_language();
            }
            IDC_DEVICE if code == CBN_SELCHANGE => {
                self.settings.borrow_mut().device = self.selected_device();
            }
            IDC_FORMAT if code == CBN_SELCHANGE => {
                self.settings.borrow_mut().export_format = self.selected_format();
            }
            IDM_DOWNLOAD if !self.busy.get() => self.download_selected_model(),
            IDM_CUSTOM_MODEL if !self.busy.get() => self.choose_custom_model(),
            IDM_MODELS_DIR => self.open_models_dir(),
            IDM_STATUS => self.announce_status(),
            IDM_COPY => self.copy_all(),
            IDM_ZOOM_IN => self.zoom(2),
            IDM_ZOOM_OUT => self.zoom(-2),
            IDM_ZOOM_RESET => self.zoom(0),
            IDM_HELP => {
                win::message_box(Some(self.hwnd), HELP_TEXT, MB_OK | MB_ICONINFORMATION);
            }
            IDM_ABOUT => self.show_about(),
            IDM_EXIT => unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            },
            _ => {}
        }
    }

    // ----- Azioni -----

    fn choose_audio(&self) {
        let extensions = audio::SUPPORTED_EXTENSIONS
            .iter()
            .map(|e| format!("*.{e}"))
            .collect::<Vec<_>>()
            .join(";");
        let filters = [
            Filter {
                name: "File audio".into(),
                pattern: extensions,
            },
            Filter {
                name: "Tutti i file".into(),
                pattern: "*.*".into(),
            },
        ];
        let folder = self.settings.borrow().last_audio_dir.clone();
        if let Some(path) =
            dialogs::open_file(self.hwnd, "Apri file audio", &filters, folder.as_deref())
        {
            self.set_audio_file(path, true);
        }
    }

    fn set_audio_file(&self, path: PathBuf, announce: bool) {
        win::set_text(self.c.audio_path, &path.display().to_string());
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        win::set_text(self.hwnd, &format!("{name} - Trascrittore"));
        self.settings.borrow_mut().last_audio_dir = path.parent().map(Path::to_path_buf);
        *self.audio_path.borrow_mut() = Some(path);
        self.set_status(
            &format!("File audio selezionato: {name}. Premi F5 per avviare la trascrizione."),
            announce,
        );
    }

    fn remember_model_choice(&self) {
        let choice = self.selected_model();
        let mut settings = self.settings.borrow_mut();
        match choice {
            Some(ModelChoice::Catalog(info)) => settings.model_id = info.id.to_string(),
            Some(ModelChoice::Custom(_)) => settings.model_id.clear(),
            None => {}
        }
    }

    fn choose_custom_model(&self) {
        let filters = [
            Filter {
                name: "Modelli Whisper (GGML)".into(),
                pattern: "*.bin".into(),
            },
            Filter {
                name: "Tutti i file".into(),
                pattern: "*.*".into(),
            },
        ];
        let folder = models::models_dir().ok();
        let Some(path) = dialogs::open_file(
            self.hwnd,
            "Scegli un file modello Whisper",
            &filters,
            folder.as_deref(),
        ) else {
            return;
        };
        if let Err(e) = models::validate_model_file(&path) {
            win::error_box(Some(self.hwnd), &format!("{e:#}"));
            return;
        }
        {
            let mut settings = self.settings.borrow_mut();
            settings.custom_model_path = Some(path.clone());
            settings.model_id.clear();
        }
        self.fill_model_combo();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.set_status(&format!("Modello personalizzato selezionato: {name}."), true);
        self.focus(self.c.model);
    }

    fn open_models_dir(&self) {
        match models::models_dir() {
            Ok(dir) => unsafe {
                ShellExecuteW(
                    Some(self.hwnd),
                    w!("open"),
                    &HSTRING::from(dir.as_os_str()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    SW_SHOWNORMAL,
                );
            },
            Err(e) => win::error_box(Some(self.hwnd), &format!("{e:#}")),
        }
    }

    fn has_unsaved_edits(&self) -> bool {
        let modified = win::send(self.c.text, EM_GETMODIFY, 0, 0) != 0;
        let has_text = unsafe { GetWindowTextLengthW(self.c.text) } > 0;
        has_text && (modified || !self.exported.get())
    }

    fn confirm(&self, question: &str) -> bool {
        win::message_box(Some(self.hwnd), question, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2)
            == IDYES
    }

    fn start_transcription(&self) {
        let Some(audio_path) = self.audio_path.borrow().clone() else {
            win::message_box(
                Some(self.hwnd),
                "Seleziona prima un file audio con Ctrl+O o con il pulsante Sfoglia.",
                MB_OK | MB_ICONINFORMATION,
            );
            self.focus(self.c.browse);
            return;
        };
        if !audio_path.is_file() {
            win::error_box(
                Some(self.hwnd),
                &format!("Il file «{}» non esiste più.", audio_path.display()),
            );
            return;
        }
        if self.has_unsaved_edits()
            && !self.confirm(
                "Il testo attuale non è stato esportato. Vuoi sostituirlo con una nuova trascrizione?",
            )
        {
            return;
        }
        let Some(model) = self.selected_model() else {
            return;
        };
        let (model_path, download) = match &model {
            ModelChoice::Catalog(info) => {
                let Ok(path) = info.path() else {
                    win::error_box(Some(self.hwnd), "Cartella dei modelli non disponibile.");
                    return;
                };
                if info.is_downloaded() {
                    (path, None)
                } else if self.confirm(&format!(
                    "Il modello «{}» non è ancora presente sul computer. Vuoi scaricarlo ora? Dimensione: circa {}. Il download avviene una sola volta.",
                    info.description,
                    models::format_size(info.size_bytes)
                )) {
                    (path, Some(*info))
                } else {
                    return;
                }
            }
            ModelChoice::Custom(path) => (path.clone(), None),
        };

        let translated = win::is_checked(self.c.translate);
        let options = TranscribeOptions {
            language: self.selected_language(),
            translate: translated,
            threads: None,
            initial_prompt: None,
        };
        let job = Job {
            audio_path,
            model_path,
            device: self.selected_device(),
            options,
        };
        let model_name = model.display_name();

        let cancel = self.begin_busy();
        let notifier = self.notifier();
        let cache = Arc::clone(&self.cache);
        *self.pending.borrow_mut() = Some((model_name, translated));

        std::thread::spawn(move || {
            if let Some(info) = download {
                if !download_with_progress(info, &notifier, &cancel) {
                    return;
                }
            }
            let progress_notifier = notifier.clone();
            let progress: ProgressFn =
                Arc::new(move |p| progress_notifier.send(WorkerEvent::Pipeline(p)));
            let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
            match pipeline::run(&job, &mut cache, progress, Arc::clone(&cancel)) {
                Ok(outcome) => notifier.send(WorkerEvent::Transcribed(outcome)),
                Err(e) if is_cancelled(&e) => notifier.send(WorkerEvent::Cancelled),
                Err(e) => notifier.send(WorkerEvent::Failed(format!("{e:#}"))),
            }
        });
    }

    fn download_selected_model(&self) {
        let Some(ModelChoice::Catalog(info)) = self.selected_model() else {
            win::message_box(
                Some(self.hwnd),
                "Seleziona un modello dell'elenco da scaricare.",
                MB_OK | MB_ICONINFORMATION,
            );
            return;
        };
        if info.is_downloaded() {
            self.set_status(&format!("Il modello «{}» è già scaricato.", info.description), true);
            return;
        }
        if !self.confirm(&format!(
            "Vuoi scaricare il modello «{}»? Dimensione: circa {}.",
            info.description,
            models::format_size(info.size_bytes)
        )) {
            return;
        }
        let cancel = self.begin_busy();
        let notifier = self.notifier();
        std::thread::spawn(move || {
            if download_with_progress(info, &notifier, &cancel) {
                notifier.send(WorkerEvent::DownloadFinished);
            }
        });
    }

    fn cancel_job(&self) {
        // Il pulsante resta abilitato: disabilitarlo mentre ha il focus farebbe perdere
        // la posizione allo screen reader. Le pressioni ripetute sono ignorate.
        if self.busy.get() && !self.cancel_flag.borrow().swap(true, Ordering::Relaxed) {
            // whisper.cpp si ferma solo al termine del blocco di audio in elaborazione.
            self.set_status(
                "Annullamento in corso: attendi qualche secondo, termina il blocco di audio in elaborazione.",
                true,
            );
        }
    }

    fn notifier(&self) -> Notifier {
        Notifier {
            tx: self.events_tx.clone(),
            hwnd: self.hwnd.0 as isize,
        }
    }

    fn on_timestamps_toggled(&self) {
        let checked = win::is_checked(self.c.timestamps);
        let text = {
            let current = self.current.borrow();
            match current.as_ref() {
                Some(c) if !c.transcript.segments.is_empty() => Some(c.transcript.to_text(checked)),
                _ => None,
            }
        };
        if let Some(text) = text {
            let modified = win::send(self.c.text, EM_GETMODIFY, 0, 0) != 0;
            if modified
                && !self.confirm(
                    "Le correzioni fatte a mano nel testo andranno perse. Vuoi continuare?",
                )
            {
                win::set_checked(self.c.timestamps, !checked);
                return;
            }
            self.set_transcript_text(&text);
            let message = if checked {
                "Timestamp inseriti nel testo."
            } else {
                "Timestamp rimossi dal testo."
            };
            self.set_status(message, true);
        }
        self.settings.borrow_mut().timestamps = checked;
    }

    fn set_transcript_text(&self, text: &str) {
        win::set_text(self.c.text, &win::to_crlf(text));
        win::send(self.c.text, EM_SETMODIFY, 0, 0);
        win::send(self.c.text, EM_SETSEL, 0, 0);
        win::send(self.c.text, EM_SCROLLCARET, 0, 0);
    }

    fn export(&self) {
        let text = win::from_crlf(&win::get_text(self.c.text));
        if text.trim().is_empty() {
            win::message_box(
                Some(self.hwnd),
                "Non c'è ancora testo da esportare. Avvia prima una trascrizione con F5.",
                MB_OK | MB_ICONINFORMATION,
            );
            return;
        }
        let selected = self.selected_format();
        let mut formats = vec![selected];
        formats.extend(ExportFormat::ALL.iter().copied().filter(|f| *f != selected));
        let filters: Vec<Filter> = formats
            .iter()
            .map(|f| Filter {
                name: f.label().to_string(),
                pattern: format!("*.{}", f.extension()),
            })
            .collect();

        let audio_path = self.audio_path.borrow().clone();
        let stem = audio_path
            .as_deref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "trascrizione".to_string());
        let folder = {
            let settings = self.settings.borrow();
            settings
                .last_export_dir
                .clone()
                .or_else(|| audio_path.as_deref().and_then(Path::parent).map(Path::to_path_buf))
        };

        let Some((path, filter_index)) = dialogs::save_file(
            self.hwnd,
            "Esporta la trascrizione",
            &filters,
            folder.as_deref(),
            &format!("{stem}.{}", selected.extension()),
            selected.extension(),
        ) else {
            return;
        };
        let format = ExportFormat::from_path(&path)
            .or_else(|| formats.get(filter_index).copied())
            .unwrap_or(selected);

        let document = {
            let current = self.current.borrow();
            let info = DocumentInfo {
                audio_path: audio_path.as_deref(),
                duration_ms: current
                    .as_ref()
                    .map(|c| c.transcript.audio_duration_ms)
                    .filter(|d| *d > 0),
                language: current.as_ref().and_then(|c| c.transcript.language.as_deref()),
                model_name: current.as_ref().map(|c| c.model_name.as_str()),
                translated: current.as_ref().is_some_and(|c| c.translated),
            };
            Document::from_text(&text, &info)
        };

        match export::export(&document, format, &path) {
            Ok(()) => {
                self.exported.set(true);
                win::send(self.c.text, EM_SETMODIFY, 0, 0);
                self.settings.borrow_mut().last_export_dir = path.parent().map(Path::to_path_buf);
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.set_status(&format!("Esportazione completata: {name}."), true);
            }
            Err(e) => {
                self.set_status("Esportazione non riuscita.", false);
                win::error_box(Some(self.hwnd), &format!("Esportazione non riuscita.\n{e:#}"));
            }
        }
    }

    fn copy_all(&self) {
        if unsafe { GetWindowTextLengthW(self.c.text) } == 0 {
            self.set_status("Non c'è testo da copiare.", true);
            return;
        }
        let mut start = 0u32;
        let mut end = 0u32;
        win::send(
            self.c.text,
            EM_GETSEL,
            &mut start as *mut u32 as usize,
            &mut end as *mut u32 as isize,
        );
        win::send(self.c.text, EM_SETSEL, 0, -1);
        win::send(self.c.text, WM_COPY, 0, 0);
        win::send(self.c.text, EM_SETSEL, start as usize, end as isize);
        self.set_status("Testo copiato negli appunti.", true);
    }

    fn zoom(&self, delta: i32) {
        let points = {
            let mut settings = self.settings.borrow_mut();
            let points = if delta == 0 {
                DEFAULT_FONT_POINTS
            } else {
                (settings.transcript_font_points as i32 + delta)
                    .clamp(MIN_FONT_POINTS as i32, MAX_FONT_POINTS as i32) as u32
            };
            settings.transcript_font_points = points;
            points
        };
        self.update_fonts();
        self.set_status(&format!("Dimensione del testo: {points} punti."), true);
    }

    fn show_about(&self) {
        let backends = device::compiled_gpu_backends();
        let gpu_support = if backends.is_empty() {
            "non incluso in questa versione (solo processore)".to_string()
        } else {
            backends.join(", ")
        };
        let gpus = device::list_gpus();
        let gpu_list = if gpus.is_empty() {
            "nessuna".to_string()
        } else {
            gpus.iter().map(|g| g.label()).collect::<Vec<_>>().join("\n")
        };
        let text = format!(
            "{} {}\n\nTrascrizione audio in locale con Whisper (whisper.cpp {}).\nNessun dato viene inviato in rete: solo i modelli vengono scaricati, su richiesta.\n\nSupporto schede video: {gpu_support}\nSchede video utilizzabili:\n{gpu_list}\n\nCartella dei modelli:\n{}",
            trascrittore_core::APP_NAME,
            trascrittore_core::APP_VERSION,
            transcriber::engine_version(),
            models::models_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_default()
        );
        win::message_box(Some(self.hwnd), &text, MB_OK | MB_ICONINFORMATION);
    }

    fn confirm_close(&self) -> bool {
        if self.busy.get()
            && !self.confirm("È in corso un'operazione. Vuoi interromperla e uscire?")
        {
            return false;
        }
        if self.has_unsaved_edits()
            && !self.confirm("Il testo trascritto non è stato esportato. Vuoi uscire comunque?")
        {
            return false;
        }
        self.cancel_flag.borrow().store(true, Ordering::Relaxed);
        true
    }

    fn save_settings(&self) {
        let _ = self.settings.borrow().save();
    }

    // ----- Stato «occupato» -----

    fn begin_busy(&self) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel_flag.borrow_mut() = Arc::clone(&cancel);
        self.busy.set(true);
        self.phase.set(None);
        self.last_milestone.set(0);
        self.started.set(Some(Instant::now()));
        win::send(self.c.progress, PBM_SETPOS, 0, 0);

        // Il focus va su «Annulla» prima di disabilitare il pulsante premuto,
        // altrimenti lo screen reader perderebbe la posizione.
        win::enable(self.c.cancel, true);
        self.focus(self.c.cancel);
        for hwnd in [
            self.c.browse,
            self.c.model,
            self.c.lang,
            self.c.device,
            self.c.translate,
            self.c.timestamps,
            self.c.start,
            self.c.export,
            self.c.format,
        ] {
            win::enable(hwnd, false);
        }
        self.update_menu();
        cancel
    }

    fn end_busy(&self, focus: HWND) {
        self.busy.set(false);
        self.phase.set(None);
        for hwnd in [
            self.c.browse,
            self.c.model,
            self.c.lang,
            self.c.device,
            self.c.translate,
            self.c.timestamps,
            self.c.start,
            self.c.export,
            self.c.format,
        ] {
            win::enable(hwnd, true);
        }
        self.focus(focus);
        win::enable(self.c.cancel, false);
        self.update_menu();
    }

    fn set_idle_state(&self) {
        win::enable(self.c.cancel, false);
        self.update_menu();
    }

    fn update_menu(&self) {
        let busy = self.busy.get();
        let state = |enabled: bool| if enabled { MF_ENABLED } else { MF_GRAYED };
        unsafe {
            for (id, enabled) in [
                (IDM_OPEN, !busy),
                (IDM_EXPORT, !busy),
                (IDM_START, !busy),
                (IDM_CANCEL, busy),
                (IDM_DOWNLOAD, !busy),
                (IDM_CUSTOM_MODEL, !busy),
            ] {
                let _ = EnableMenuItem(self.menu, u32::from(id), MF_BYCOMMAND | state(enabled));
            }
            let _ = DrawMenuBar(self.hwnd);
        }
    }

    // ----- Eventi del thread di lavoro -----

    fn drain_events(&self) {
        while let Ok(event) = self.events_rx.try_recv() {
            self.handle_event(event);
        }
    }

    fn handle_event(&self, event: WorkerEvent) {
        match event {
            WorkerEvent::Downloading(percent) => {
                self.enter_phase(Phase::Downloading);
                self.show_progress(Phase::Downloading, percent);
            }
            WorkerEvent::Pipeline(Progress::Decoding(fraction)) => {
                self.enter_phase(Phase::Decoding);
                self.show_progress(Phase::Decoding, (fraction * 100.0) as u32);
            }
            WorkerEvent::Pipeline(Progress::LoadingModel(device)) => {
                self.phase.set(Some(Phase::LoadingModel));
                win::send(self.c.progress, PBM_SETPOS, 0, 0);
                self.set_status(
                    &format!("Caricamento del modello su {}…", device.label()),
                    true,
                );
            }
            WorkerEvent::Pipeline(Progress::Transcribing(percent)) => {
                self.enter_phase(Phase::Transcribing);
                self.show_progress(Phase::Transcribing, percent.max(0) as u32);
            }
            WorkerEvent::Transcribed(outcome) => self.on_transcribed(outcome),
            WorkerEvent::DownloadFinished => {
                self.fill_model_combo();
                self.end_busy(self.c.model);
                self.set_status("Download del modello completato.", true);
            }
            WorkerEvent::Failed(message) => {
                self.end_busy(self.c.start);
                self.set_status("Operazione non riuscita.", false);
                win::error_box(Some(self.hwnd), &message);
            }
            WorkerEvent::Cancelled => {
                self.fill_model_combo();
                self.end_busy(self.c.start);
                self.set_status("Operazione annullata.", true);
            }
        }
    }

    fn enter_phase(&self, phase: Phase) {
        if self.phase.get() != Some(phase) {
            self.phase.set(Some(phase));
            self.last_milestone.set(0);
            self.set_status(&format!("{}…", phase.description()), true);
        }
    }

    fn show_progress(&self, phase: Phase, percent: u32) {
        let percent = percent.min(100);
        win::send(self.c.progress, PBM_SETPOS, percent as usize, 0);
        let text = format!("{}: {percent}%", phase.description());
        // Annuncio vocale solo ai traguardi del 25%, per non disturbare la lettura.
        let milestone = percent / 25 * 25;
        let announce = matches!(phase, Phase::Downloading | Phase::Transcribing)
            && milestone > self.last_milestone.get()
            && milestone < 100;
        if announce {
            self.last_milestone.set(milestone);
        }
        self.set_status(&text, announce);
    }

    fn on_transcribed(&self, outcome: Outcome) {
        let Outcome { transcript, device } = outcome;
        let elapsed = self
            .started
            .get()
            .map(|t| format_duration_words(t.elapsed().as_millis() as u64));
        let timestamps = win::is_checked(self.c.timestamps);
        let text = transcript.to_text(timestamps);
        let words = transcript.word_count();
        let duration = format_duration_words(transcript.audio_duration_ms);
        let language = transcript
            .language
            .as_deref()
            .map(languages::display_name);
        let (model_name, translated) = self.pending.borrow_mut().take().unwrap_or_default();
        *self.current.borrow_mut() = Some(CurrentTranscript {
            transcript,
            model_name,
            translated,
        });
        self.set_transcript_text(&text);
        self.exported.set(false);
        self.fill_model_combo();
        win::send(self.c.progress, PBM_SETPOS, 100, 0);

        let mut status = format!(
            "Trascrizione completata: {words} parole, durata dell'audio {duration}"
        );
        if let Some(language) = language {
            status.push_str(&format!(", lingua {language}"));
        }
        if let Some(elapsed) = elapsed {
            status.push_str(&format!(". Tempo impiegato: {elapsed} su {}", device.label()));
        }
        status.push('.');
        if words == 0 {
            status = "Trascrizione completata, ma non è stato riconosciuto alcun parlato.".into();
        }
        // Prima si sposta il focus sul testo, poi si annuncia l'esito.
        self.end_busy(self.c.text);
        self.set_status(&status, true);
    }

    // ----- Stato e annunci -----

    fn set_status(&self, text: &str, announce: bool) {
        *self.status.borrow_mut() = text.to_string();
        win::set_text(self.c.status, text);
        if announce {
            a11y::announce(self.c.status, text);
        }
    }

    fn announce_status(&self) {
        let status = self.status.borrow().clone();
        a11y::announce(self.c.status, &status);
    }

    // ----- Focus -----

    fn focus(&self, hwnd: HWND) {
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
        self.last_focus.set(hwnd);
    }

    fn is_own_control(&self, hwnd: HWND) -> bool {
        !hwnd.is_invalid()
            && (self.all_controls().contains(&hwnd) || unsafe { IsChild(self.hwnd, hwnd) }.as_bool())
    }

    fn restore_focus(&self) {
        let target = self.last_focus.get();
        let target = if !target.is_invalid() && unsafe { IsWindowEnabled(target) }.as_bool() {
            target
        } else if self.busy.get() {
            self.c.cancel
        } else {
            self.c.start
        };
        unsafe {
            let _ = SetFocus(Some(target));
        }
    }

    // ----- Caratteri e disposizione -----

    fn px(&self, value: i32) -> i32 {
        value * self.dpi.get() as i32 / 96
    }

    fn update_fonts(&self) {
        let dpi = self.dpi.get();
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        unsafe {
            let _ = SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                metrics.cbSize,
                Some(&mut metrics as *mut _ as *mut c_void),
                0,
                dpi,
            );
            let ui_font = CreateFontIndirectW(&metrics.lfMessageFont);
            let mut text_logfont = metrics.lfMessageFont;
            let points = self.settings.borrow().transcript_font_points.clamp(MIN_FONT_POINTS, MAX_FONT_POINTS);
            text_logfont.lfHeight = -((points * dpi) as i32 / 72);
            let text_font = CreateFontIndirectW(&text_logfont);

            for hwnd in self.all_controls() {
                let font = if hwnd == self.c.text { text_font } else { ui_font };
                win::send(hwnd, WM_SETFONT, font.0 as usize, 1);
            }
            let old_ui = self.ui_font.replace(ui_font);
            let old_text = self.text_font.replace(text_font);
            if !old_ui.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(old_ui.0));
            }
            if !old_text.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(old_text.0));
            }
        }
    }

    fn text_size(&self, text: &str) -> SIZE {
        let wide: Vec<u16> = text.encode_utf16().filter(|c| *c != u16::from(b'&')).collect();
        let mut size = SIZE::default();
        unsafe {
            let hdc = GetDC(Some(self.hwnd));
            let old = SelectObject(hdc, HGDIOBJ(self.ui_font.get().0));
            let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
            SelectObject(hdc, old);
            ReleaseDC(Some(self.hwnd), hdc);
        }
        size
    }

    fn layout(&self) {
        let mut client = RECT::default();
        unsafe {
            if GetClientRect(self.hwnd, &mut client).is_err() {
                return;
            }
        }
        let width = client.right;
        let height = client.bottom;
        if width <= 0 || height <= 0 {
            return;
        }

        let margin = self.px(12);
        let gap = self.px(8);
        let text_h = self.text_size("Ag").cy.max(self.px(14));
        let ctrl_h = text_h + self.px(10);
        let label_w = ["File audio:", "Modello:", "Lingua:", "Dispositivo:", "Avanzamento:", "Stato:"]
            .iter()
            .map(|l| self.text_size(l).cx)
            .max()
            .unwrap_or(0)
            + gap;
        let x1 = margin + label_w;
        let right = width - margin;
        let field_w = (right - x1).max(self.px(100));
        let button_w = |t: &str| self.text_size(t).cx + self.px(28);
        let check_w = |t: &str| self.text_size(t).cx + self.px(28);
        let label_y = |y: i32| y + (ctrl_h - text_h) / 2;
        let place = |hwnd: HWND, x: i32, y: i32, w: i32, h: i32| unsafe {
            let _ = MoveWindow(hwnd, x, y, w.max(1), h.max(1), true);
        };
        let c = &self.c;
        let mut y = margin;

        let browse_w = button_w("Sfoglia...");
        place(c.audio_label, margin, label_y(y), label_w - gap, text_h);
        place(c.audio_path, x1, y, field_w - browse_w - gap, ctrl_h);
        place(c.browse, right - browse_w, y, browse_w, ctrl_h);
        y += ctrl_h + gap;

        place(c.model_label, margin, label_y(y), label_w - gap, text_h);
        place(c.model, x1, y, field_w, ctrl_h * 12);
        y += ctrl_h + gap;

        place(c.lang_label, margin, label_y(y), label_w - gap, text_h);
        place(c.lang, x1, y, field_w.min(self.px(360)), ctrl_h * 14);
        y += ctrl_h + gap;

        place(c.device_label, margin, label_y(y), label_w - gap, text_h);
        place(c.device, x1, y, field_w, ctrl_h * 8);
        y += ctrl_h + gap;

        let translate_w = check_w("Traduci in inglese");
        place(c.translate, x1, y, translate_w, ctrl_h);
        place(c.timestamps, x1 + translate_w + self.px(16), y, check_w("Includi timestamp"), ctrl_h);
        y += ctrl_h + gap;

        let start_w = button_w("Avvia trascrizione");
        place(c.start, x1, y, start_w, ctrl_h);
        place(c.cancel, x1 + start_w + gap, y, button_w("Annulla"), ctrl_h);
        y += ctrl_h + gap * 2;

        place(c.progress_label, margin, label_y(y), label_w - gap, text_h);
        place(c.progress, x1, y + self.px(3), field_w, ctrl_h - self.px(6));
        y += ctrl_h + gap;

        place(c.status_label, margin, y, label_w - gap, text_h);
        place(c.status, x1, y, field_w, text_h * 2 + self.px(2));
        y += text_h * 2 + gap * 2;

        place(c.text_label, margin, y, right - margin, text_h);
        y += text_h + self.px(4);

        let bottom_y = height - margin - ctrl_h;
        place(c.text, margin, y, right - margin, (bottom_y - gap - y).max(self.px(60)));

        let format_label_w = self.text_size("Formato di esportazione:").cx + gap;
        let combo_w = self.px(260);
        place(c.format_label, margin, label_y(bottom_y), format_label_w, text_h);
        place(c.format, margin + format_label_w, bottom_y, combo_w, ctrl_h * 8);
        place(
            c.export,
            margin + format_label_w + combo_w + gap,
            bottom_y,
            button_w("Esporta..."),
            ctrl_h,
        );
    }
}

/// Scarica un modello inoltrando l'avanzamento; restituisce `false` in caso di errore o annullamento
/// (l'evento corrispondente è già stato inviato).
fn download_with_progress(info: &ModelInfo, notifier: &Notifier, cancel: &AtomicBool) -> bool {
    notifier.send(WorkerEvent::Downloading(0));
    let mut last = 0u32;
    let result = models::download(
        info,
        |done, total| {
            if let Some(total) = total.filter(|t| *t > 0) {
                let percent = (done * 100 / total) as u32;
                if percent != last {
                    last = percent;
                    notifier.send(WorkerEvent::Downloading(percent));
                }
            }
        },
        cancel,
    );
    match result {
        Ok(_) => true,
        Err(e) if is_cancelled(&e) => {
            notifier.send(WorkerEvent::Cancelled);
            false
        }
        Err(e) => {
            notifier.send(WorkerEvent::Failed(format!("{e:#}")));
            false
        }
    }
}
