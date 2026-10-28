//! Finestra principale per macOS, stato dell'applicazione e comunicazione con il thread di lavoro.
//!
//! Stessa disposizione e stesse regole della versione per Windows: controlli nativi in ordine
//! di lettura, pulsanti non disponibili disattivati invece che nascosti, annunci a VoiceOver
//! senza spostare il focus, focus sul testo al termine della trascrizione.

#![allow(unused_unsafe)]

use std::cell::{Cell, OnceCell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationDelegate, NSApplicationTerminateReply, NSBackingStoreType,
    NSButton, NSControl, NSMenu, NSMenuItem, NSMenuItemValidation, NSModalResponseOK, NSPasteboard,
    NSPasteboardTypeString, NSPopUpButton, NSProgressIndicator, NSSavePanel, NSScrollView,
    NSTextAlignment, NSTextDelegate, NSTextField, NSTextView, NSTextViewDelegate, NSView, NSWindow,
    NSWindowDelegate, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSArray, NSNotification, NSSize, NSURL};

use trascrittore_core::device::{self, Device, ResolvedDevice};
use trascrittore_core::document::{Document, DocumentInfo};
use trascrittore_core::export::{self, ExportFormat};
use trascrittore_core::models::{self, ModelInfo};
use trascrittore_core::pipeline::{self, Job, Outcome, Progress, ProgressFn};
use trascrittore_core::settings::Settings;
use trascrittore_core::transcriber::{self, EngineOptions, TranscribeOptions, Transcriber};
use trascrittore_core::transcript::{format_duration_words, Transcript};
use trascrittore_core::{audio, is_cancelled, languages, paths};

use super::check::{self, CheckOptions};
use super::ui::{self, ns, rect, FlippedView, Level};
use super::{menu, Options};

const MIN_FONT_POINTS: u32 = 8;
const MAX_FONT_POINTS: u32 = 40;
const DEFAULT_FONT_POINTS: u32 = 12;

const HELP_TEXT: &str = "Uso di Trascrittore

1. Premi Comando+O (oppure il pulsante Scegli) e scegli un file audio. Puoi anche trascinare il file sull'icona di Trascrittore nel Dock o usare «Apri con» nel Finder.
2. Scegli modello e lingua. Il modello consigliato è «Large v3 Turbo compresso»; i modelli «compressi» più piccoli sono più veloci ma meno precisi.
   «Usa la scheda video (GPU)» usa la scheda video del Mac tramite Metal: è molto più veloce del processore. «Modalità rapida» accelera ancora, con qualche errore in più. Flash Attention (menu Trascrizione) è disattivata: su alcune schede video accelera, su altre rallenta.
3. Premi Comando+R per avviare la trascrizione. Esc o Comando+punto la interrompono.
4. Al termine il cursore si sposta sul testo trascritto: puoi leggerlo e correggerlo.
5. Scegli il formato e premi Comando+S per esportare.

Tasti rapidi
Comando+O: apri file audio
Comando+R (oppure F5): avvia la trascrizione
Esc oppure Comando+punto: interrompi l'operazione in corso
Comando+S: esporta
Comando+Maiuscole+C: copia tutto il testo trascritto
Comando+L (oppure F2): leggi lo stato corrente
Comando+più, Comando+meno, Comando+0: dimensione del testo trascritto
Comando+1: vai al file audio
Comando+2: vai al modello
Comando+3: vai alla lingua
Comando+4: vai al testo trascritto
Comando+5: vai al formato di esportazione
Comando+punto interrogativo (oppure F1): questa guida

Navigazione con la tastiera e con VoiceOver
Con VoiceOver ti sposti tra i controlli con Tab oppure con VO+freccia destra e sinistra. Lo stato viene annunciato da solo durante il lavoro, senza spostare il cursore.
Senza VoiceOver il tasto Tab raggiunge pulsanti, caselle e menu a comparsa solo se è attiva «Navigazione da tastiera» in Impostazioni di Sistema, Tastiera. In ogni caso i comandi del menu Vai portano direttamente ai campi principali.
Nel testo trascritto Tab passa al controllo successivo e Maiuscole+Tab a quello precedente.";

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

/// Inoltra gli eventi all'interfaccia e la «sveglia» sul thread principale.
#[derive(Clone)]
struct Notifier {
    tx: Sender<WorkerEvent>,
    /// Indirizzo del controller, che resta in vita per tutta l'esecuzione.
    target: usize,
}

impl Notifier {
    fn send(&self, event: WorkerEvent) {
        if self.tx.send(event).is_ok() {
            // `performSelectorOnMainThread:` si può chiamare da qualsiasi thread.
            let target = self.target as *const AnyObject;
            unsafe {
                let _: () = msg_send![
                    &*target,
                    performSelectorOnMainThread: sel!(workerEvents:),
                    withObject: None::<&AnyObject>,
                    waitUntilDone: false
                ];
            }
        }
    }
}

/// Modello scelto nel menu a comparsa.
#[derive(Debug, Clone)]
enum ModelChoice {
    Catalog(&'static ModelInfo),
    Custom(PathBuf),
}

impl ModelChoice {
    fn display_name(&self) -> String {
        match self {
            ModelChoice::Catalog(info) => info.id.to_string(),
            ModelChoice::Custom(path) => file_name(path),
        }
    }
}

/// Trascrizione corrente con le informazioni necessarie all'esportazione.
struct CurrentTranscript {
    transcript: Transcript,
    model_name: String,
    translated: bool,
}

/// Controlli della finestra principale, nell'ordine di lettura e di tabulazione.
pub(super) struct Ui {
    pub window: Retained<NSWindow>,
    pub content: Retained<FlippedView>,
    audio_label: Retained<NSTextField>,
    pub audio_path: Retained<NSTextField>,
    browse: Retained<NSButton>,
    model_label: Retained<NSTextField>,
    pub model: Retained<NSPopUpButton>,
    lang_label: Retained<NSTextField>,
    pub lang: Retained<NSPopUpButton>,
    use_gpu: Retained<NSButton>,
    device_label: Retained<NSTextField>,
    device: Retained<NSPopUpButton>,
    translate: Retained<NSButton>,
    timestamps: Retained<NSButton>,
    fast: Retained<NSButton>,
    start: Retained<NSButton>,
    cancel: Retained<NSButton>,
    progress_label: Retained<NSTextField>,
    progress: Retained<NSProgressIndicator>,
    status_label: Retained<NSTextField>,
    status: Retained<NSTextField>,
    text_label: Retained<NSTextField>,
    text_scroll: Retained<NSScrollView>,
    pub text: Retained<NSTextView>,
    format_label: Retained<NSTextField>,
    pub format: Retained<NSPopUpButton>,
    export: Retained<NSButton>,
}

pub struct State {
    ui: OnceCell<Ui>,
    settings: RefCell<Settings>,
    audio_path: RefCell<Option<PathBuf>>,
    /// File da aprire appena la finestra è pronta (argomento o «Apri con» all'avvio).
    initial_file: RefCell<Option<PathBuf>>,
    current: RefCell<Option<CurrentTranscript>>,
    /// Modello e traduzione della trascrizione in corso, applicati solo se riesce.
    pending: RefCell<Option<(String, bool)>>,
    exported: Cell<bool>,
    /// Il testo è stato modificato a mano dopo l'ultima trascrizione o esportazione.
    modified: Cell<bool>,
    busy: Cell<bool>,
    launched: Cell<bool>,
    quit_confirmed: Cell<bool>,
    phase: Cell<Option<Phase>>,
    last_milestone: Cell<u32>,
    started: Cell<Option<Instant>>,
    gpu_warning_shown: Cell<bool>,
    status: RefCell<String>,
    cancel_flag: RefCell<Arc<AtomicBool>>,
    events_tx: Sender<WorkerEvent>,
    events_rx: Receiver<WorkerEvent>,
    cache: Arc<Mutex<Option<Transcriber>>>,
    /// Pannello di esportazione aperto, per cambiarne il formato dal menu a comparsa.
    save_panel: RefCell<Option<Retained<NSSavePanel>>>,
    /// Finestre della guida e delle informazioni.
    text_windows: RefCell<Vec<Retained<NSWindow>>>,
    check: Option<CheckOptions>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TrascrittoreController"]
    #[ivars = State]
    pub struct Controller;

    unsafe impl NSObjectProtocol for Controller {}

    unsafe impl NSApplicationDelegate for Controller {
        #[unsafe(method(applicationWillFinishLaunching:))]
        fn will_finish_launching(&self, _notification: &NSNotification) {
            menu::install(&NSApplication::sharedApplication(self.mtm()), self.target(), self.mtm());
            self.build();
            self.initialize();
        }

        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            self.show();
        }

        #[unsafe(method(application:openURLs:))]
        fn open_urls(&self, _application: &NSApplication, urls: &NSArray<NSURL>) {
            let path = urls.iter().find_map(|url| ui::url_path(&url)).filter(|p| p.is_file());
            if let Some(path) = path {
                self.open_from_finder(path);
            }
        }

        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _application: &NSApplication) -> NSApplicationTerminateReply {
            if self.ivars().quit_confirmed.get() || self.confirm_close() {
                NSApplicationTerminateReply::TerminateNow
            } else {
                NSApplicationTerminateReply::TerminateCancel
            }
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn terminate_after_last_window(&self, _application: &NSApplication) -> bool {
            true
        }

        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _notification: &NSNotification) {
            self.ivars().cancel_flag.borrow().store(true, Ordering::Relaxed);
            self.save_settings();
            self.release_engine();
        }

        #[unsafe(method(applicationSupportsSecureRestorableState:))]
        fn secure_restorable_state(&self, _application: &NSApplication) -> bool {
            true
        }
    }

    unsafe impl NSWindowDelegate for Controller {
        /// Chiudere la finestra principale chiude il programma, anche se sono aperte la guida o
        /// le informazioni.
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _window: &NSWindow) -> bool {
            if self.confirm_close() {
                self.ivars().quit_confirmed.set(true);
                NSApplication::sharedApplication(self.mtm()).terminate(None);
            }
            false
        }

        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notification: &NSNotification) {
            self.layout();
        }
    }

    unsafe impl NSTextDelegate for Controller {
        #[unsafe(method(textDidChange:))]
        fn text_did_change(&self, _notification: &NSNotification) {
            self.ivars().modified.set(true);
            self.update_edited();
        }
    }

    unsafe impl NSTextViewDelegate for Controller {
        /// Tab ed Esc nel testo trascritto: Tab passa al controllo successivo invece di
        /// inserire una tabulazione; Esc interrompe il lavoro in corso, senza completamento
        /// automatico delle parole.
        #[unsafe(method(textView:doCommandBySelector:))]
        fn text_view_command(&self, _text_view: &NSTextView, command: Sel) -> bool {
            let window = &self.ui().window;
            if command == sel!(insertTab:) {
                window.selectNextKeyView(None);
                true
            } else if command == sel!(insertBacktab:) {
                window.selectPreviousKeyView(None);
                true
            } else if command == sel!(cancelOperation:) {
                self.cancel_job();
                true
            } else {
                false
            }
        }
    }

    unsafe impl NSMenuItemValidation for Controller {
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            self.validate(item)
        }
    }

    /// Comandi di menu, pulsanti e controlli.
    impl Controller {
        #[unsafe(method(chooseAudio:))]
        fn choose_audio_action(&self, _sender: Option<&AnyObject>) {
            if !self.busy() {
                self.choose_audio();
            }
        }

        #[unsafe(method(startTranscription:))]
        fn start_action(&self, _sender: Option<&AnyObject>) {
            if !self.busy() {
                self.start_transcription();
            }
        }

        #[unsafe(method(cancelJob:))]
        fn cancel_action(&self, _sender: Option<&AnyObject>) {
            self.cancel_job();
        }

        #[unsafe(method(export:))]
        fn export_action(&self, _sender: Option<&AnyObject>) {
            if !self.busy() {
                self.export();
            }
        }

        #[unsafe(method(copyAll:))]
        fn copy_all_action(&self, _sender: Option<&AnyObject>) {
            self.copy_all();
        }

        #[unsafe(method(zoomIn:))]
        fn zoom_in(&self, _sender: Option<&AnyObject>) {
            self.zoom(2);
        }

        #[unsafe(method(zoomOut:))]
        fn zoom_out(&self, _sender: Option<&AnyObject>) {
            self.zoom(-2);
        }

        #[unsafe(method(zoomReset:))]
        fn zoom_reset(&self, _sender: Option<&AnyObject>) {
            self.zoom(0);
        }

        #[unsafe(method(downloadModel:))]
        fn download_action(&self, _sender: Option<&AnyObject>) {
            if !self.busy() {
                self.download_selected_model();
            }
        }

        #[unsafe(method(chooseCustomModel:))]
        fn custom_model_action(&self, _sender: Option<&AnyObject>) {
            if !self.busy() {
                self.choose_custom_model();
            }
        }

        #[unsafe(method(openModelsDir:))]
        fn models_dir_action(&self, _sender: Option<&AnyObject>) {
            self.open_models_dir();
        }

        #[unsafe(method(toggleFlashAttention:))]
        fn flash_action(&self, _sender: Option<&AnyObject>) {
            if !self.busy() {
                self.toggle_flash_attention();
            }
        }

        #[unsafe(method(announceStatus:))]
        fn status_action(&self, _sender: Option<&AnyObject>) {
            let status = self.ivars().status.borrow().clone();
            self.announce(&status);
        }

        #[unsafe(method(focusAudio:))]
        fn focus_audio(&self, _sender: Option<&AnyObject>) {
            self.focus(&self.ui().audio_path);
        }

        #[unsafe(method(focusModel:))]
        fn focus_model(&self, _sender: Option<&AnyObject>) {
            self.focus_popup(&self.ui().model);
        }

        #[unsafe(method(focusLanguage:))]
        fn focus_language(&self, _sender: Option<&AnyObject>) {
            self.focus_popup(&self.ui().lang);
        }

        #[unsafe(method(focusText:))]
        fn focus_text(&self, _sender: Option<&AnyObject>) {
            self.focus(&self.ui().text);
        }

        #[unsafe(method(focusFormat:))]
        fn focus_format(&self, _sender: Option<&AnyObject>) {
            self.focus_popup(&self.ui().format);
        }

        #[unsafe(method(showHelp:))]
        fn help_action(&self, _sender: Option<&AnyObject>) {
            self.show_text_window("Guida rapida di Trascrittore", HELP_TEXT);
        }

        #[unsafe(method(showAbout:))]
        fn about_action(&self, _sender: Option<&AnyObject>) {
            let text = self.about_text();
            self.show_text_window("Informazioni su Trascrittore", &text);
        }

        #[unsafe(method(modelChanged:))]
        fn model_changed(&self, _sender: Option<&AnyObject>) {
            self.remember_model_choice();
        }

        #[unsafe(method(languageChanged:))]
        fn language_changed(&self, _sender: Option<&AnyObject>) {
            self.ivars().settings.borrow_mut().language = self.selected_language();
        }

        #[unsafe(method(useGpuToggled:))]
        fn use_gpu_toggled(&self, _sender: Option<&AnyObject>) {
            self.on_use_gpu_toggled();
        }

        #[unsafe(method(deviceChanged:))]
        fn device_changed(&self, _sender: Option<&AnyObject>) {
            self.ivars().settings.borrow_mut().device = self.selected_device();
        }

        #[unsafe(method(translateToggled:))]
        fn translate_toggled(&self, _sender: Option<&AnyObject>) {
            self.ivars().settings.borrow_mut().translate = ui::is_checked(&self.ui().translate);
        }

        #[unsafe(method(timestampsToggled:))]
        fn timestamps_toggled(&self, _sender: Option<&AnyObject>) {
            self.on_timestamps_toggled();
        }

        #[unsafe(method(fastToggled:))]
        fn fast_toggled(&self, _sender: Option<&AnyObject>) {
            self.ivars().settings.borrow_mut().fast_mode = ui::is_checked(&self.ui().fast);
        }

        #[unsafe(method(formatChanged:))]
        fn format_changed(&self, _sender: Option<&AnyObject>) {
            self.ivars().settings.borrow_mut().export_format = self.selected_format();
        }

        #[unsafe(method(saveFormatChanged:))]
        fn save_format_changed(&self, sender: Option<&AnyObject>) {
            if let Some(popup) = sender.and_then(|s| s.downcast_ref::<NSPopUpButton>()) {
                self.on_save_format_changed(popup);
            }
        }

        #[unsafe(method(workerEvents:))]
        fn worker_events(&self, _sender: Option<&AnyObject>) {
            self.drain_events();
        }

        #[unsafe(method(runCheck:))]
        fn run_check_action(&self, _sender: Option<&AnyObject>) {
            self.run_check();
        }

        #[unsafe(method(checkTranscribed:))]
        fn check_transcribed_action(&self, _sender: Option<&AnyObject>) {
            self.finish_check_after_transcription();
        }
    }
);

impl Controller {
    pub fn new(mtm: MainThreadMarker, options: Options) -> Retained<Self> {
        let (events_tx, events_rx) = mpsc::channel();
        let mut settings = Settings::load();
        // Modello della verifica: un identificativo del catalogo oppure un file .bin.
        if let Some(model) = options.check.as_ref().and_then(|c| c.model.as_deref()) {
            if Path::new(model).is_file() {
                settings.model_id.clear();
                settings.custom_model_path = Some(PathBuf::from(model));
            } else {
                settings.model_id = model.to_string();
                settings.custom_model_path = None;
            }
        }
        let this = Self::alloc(mtm).set_ivars(State {
            ui: OnceCell::new(),
            settings: RefCell::new(settings),
            audio_path: RefCell::new(None),
            initial_file: RefCell::new(options.audio_file),
            current: RefCell::new(None),
            pending: RefCell::new(None),
            exported: Cell::new(true),
            modified: Cell::new(false),
            busy: Cell::new(false),
            launched: Cell::new(false),
            quit_confirmed: Cell::new(false),
            phase: Cell::new(None),
            last_milestone: Cell::new(0),
            started: Cell::new(None),
            gpu_warning_shown: Cell::new(false),
            status: RefCell::new("Pronto.".to_string()),
            cancel_flag: RefCell::new(Arc::new(AtomicBool::new(false))),
            events_tx,
            events_rx,
            cache: Arc::new(Mutex::new(None)),
            save_panel: RefCell::new(None),
            text_windows: RefCell::new(Vec::new()),
            check: options.check,
        });
        unsafe { msg_send![super(this), init] }
    }

    fn target(&self) -> &AnyObject {
        self
    }

    pub(super) fn ui(&self) -> &Ui {
        self.ivars().ui.get().expect("interfaccia non ancora creata")
    }

    fn busy(&self) -> bool {
        self.ivars().busy.get()
    }

    fn checking(&self) -> bool {
        self.ivars().check.is_some()
    }

    // ----- Creazione della finestra -----

    /// Crea i controlli. L'ordine di tabulazione segue la posizione sullo schermo, dall'alto
    /// in basso e da sinistra a destra, come l'ordine di lettura di VoiceOver.
    fn build(&self) {
        let mtm = self.mtm();
        let target = self.target();

        let audio_label = ui::label("File audio:", mtm);
        let audio_path = ui::read_only_field("Nessun file selezionato", "File audio", mtm);
        let browse = ui::button("Scegli…", target, sel!(chooseAudio:), mtm);
        ui::set_help(&browse, "Sceglie il file audio da trascrivere. Comando+O.");
        let model_label = ui::label("Modello:", mtm);
        let model = ui::popup("Modello", target, sel!(modelChanged:), mtm);
        let lang_label = ui::label("Lingua:", mtm);
        let lang = ui::popup("Lingua", target, sel!(languageChanged:), mtm);
        let use_gpu = ui::checkbox("Usa la scheda video (GPU)", target, sel!(useGpuToggled:), mtm);
        ui::set_help(&use_gpu, &gpu_availability_text());
        let device_label = ui::label("Scheda video:", mtm);
        let device = ui::popup("Scheda video", target, sel!(deviceChanged:), mtm);
        let translate = ui::checkbox("Traduci in inglese", target, sel!(translateToggled:), mtm);
        let timestamps = ui::checkbox("Includi timestamp", target, sel!(timestampsToggled:), mtm);
        ui::set_help(
            &timestamps,
            "Aggiunge all'inizio di ogni frase il momento dell'audio in cui viene detta.",
        );
        let fast = ui::checkbox("Modalità rapida (meno precisa)", target, sel!(fastToggled:), mtm);
        ui::set_help(
            &fast,
            "Sulla scheda video è fino a due volte più veloce, con qualche errore in più nel testo; sul processore il guadagno è minimo.",
        );
        let start = ui::button("Avvia trascrizione", target, sel!(startTranscription:), mtm);
        ui::set_help(&start, "Comando+R");
        let cancel = ui::button("Interrompi", target, sel!(cancelJob:), mtm);
        // Esc funziona ovunque nella finestra finché il pulsante è attivo.
        cancel.setKeyEquivalent(&ns("\u{1b}"));
        ui::set_help(&cancel, "Esc oppure Comando+punto");
        let progress_label = ui::label("Avanzamento:", mtm);
        let progress = ui::progress_bar("Avanzamento", mtm);
        let status_label = ui::visible_label("Stato:", mtm);
        let status = ui::status_text("Pronto.", mtm);
        let text_label = ui::label("Testo trascritto:", mtm);
        text_label.setAlignment(NSTextAlignment::Left);
        let (text_scroll, text) = ui::text_area("Testo trascritto", true, mtm);
        ui::set_help(&text, "Puoi leggere e correggere il testo prima di esportarlo.");
        text.setDelegate(Some(ProtocolObject::from_ref(self)));
        let format_label = ui::label("Formato di esportazione:", mtm);
        let format = ui::popup("Formato di esportazione", target, sel!(formatChanged:), mtm);
        let export = ui::button("Esporta…", target, sel!(export:), mtm);
        ui::set_help(&export, "Comando+S");

        let content = FlippedView::new(mtm);
        let views: [&NSView; 24] = [
            &audio_label,
            &audio_path,
            &browse,
            &model_label,
            &model,
            &lang_label,
            &lang,
            &use_gpu,
            &device_label,
            &device,
            &translate,
            &timestamps,
            &fast,
            &start,
            &cancel,
            &progress_label,
            &progress,
            &status_label,
            &status,
            &text_label,
            &text_scroll,
            &format_label,
            &format,
            &export,
        ];
        for view in views {
            content.addSubview(view);
        }

        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect(0.0, 0.0, 860.0, 740.0),
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // La finestra appartiene a `Retained`: AppKit non deve liberarla alla chiusura.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setRestorable(false);
        window.setTitle(&ns("Trascrittore"));
        window.setContentMinSize(NSSize::new(640.0, 600.0));
        window.setContentView(Some(&content));
        window.setDelegate(Some(ProtocolObject::from_ref(self)));
        window.setAutorecalculatesKeyViewLoop(true);
        window.setInitialFirstResponder(Some(&browse));

        let built = Ui {
            window,
            content,
            audio_label,
            audio_path,
            browse,
            model_label,
            model,
            lang_label,
            lang,
            use_gpu,
            device_label,
            device,
            translate,
            timestamps,
            fast,
            start,
            cancel,
            progress_label,
            progress,
            status_label,
            status,
            text_label,
            text_scroll,
            text,
            format_label,
            format,
            export,
        };
        if self.ivars().ui.set(built).is_err() {
            unreachable!("interfaccia creata due volte");
        }
    }

    /// Popola i controlli con le preferenze salvate.
    fn initialize(&self) {
        let ui = self.ui();
        let settings = self.ivars().settings.borrow().clone();

        self.fill_model_popup();

        ui::popup_add(&ui.lang, "Rilevamento automatico");
        for lang in languages::LANGUAGES {
            ui::popup_add(&ui.lang, lang.name);
        }
        let lang_index = settings
            .language
            .as_deref()
            .and_then(|code| languages::LANGUAGES.iter().position(|l| l.code == code))
            .map_or(0, |i| i + 1);
        ui::popup_select(&ui.lang, lang_index);

        let gpus = device::list_gpus();
        ui::set_checked(&ui.use_gpu, settings.device != Device::Cpu && !gpus.is_empty());
        self.fill_device_popup(settings.device);
        self.update_device_enabled();

        for format in ExportFormat::ALL {
            ui::popup_add(&ui.format, format.label());
        }
        let format_index =
            ExportFormat::ALL.iter().position(|f| *f == settings.export_format).unwrap_or(0);
        ui::popup_select(&ui.format, format_index);

        ui::set_checked(&ui.translate, settings.translate);
        ui::set_checked(&ui.timestamps, settings.timestamps);
        ui::set_checked(&ui.fast, settings.fast_mode);
        self.update_text_font();

        // Stato iniziale: dice subito se si userà la scheda video (leggibile con Comando+L).
        self.set_status(&format!("Pronto. {}", self.device_summary()), false);
        ui.cancel.setEnabled(false);
        self.layout();
    }

    /// Mostra la finestra e apre il file indicato all'avvio.
    fn show(&self) {
        let ui = self.ui();
        ui.window.center();
        ui.window.makeKeyAndOrderFront(None);
        let app = NSApplication::sharedApplication(self.mtm());
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        self.ivars().launched.set(true);

        let initial = self.ivars().initial_file.borrow_mut().take();
        if let Some(path) = initial {
            self.set_audio_file(path, false);
            self.focus(&ui.start);
        } else {
            self.focus(&ui.browse);
        }

        if self.checking() {
            unsafe {
                let _: () = msg_send![
                    self,
                    performSelector: sel!(runCheck:),
                    withObject: None::<&AnyObject>,
                    afterDelay: 0.5f64
                ];
            }
        }
    }

    /// File aperto dal Finder («Apri con», trascinamento sull'icona).
    fn open_from_finder(&self, path: PathBuf) {
        if !self.ivars().launched.get() {
            *self.ivars().initial_file.borrow_mut() = Some(path);
            return;
        }
        if self.busy() {
            self.set_status("È in corso un'operazione: apri il file quando è terminata.", true);
            return;
        }
        self.set_audio_file(path, true);
        self.focus(&self.ui().start);
    }

    // ----- Modelli, lingue, dispositivi -----

    fn fill_model_popup(&self) {
        let ui = self.ui();
        let settings = self.ivars().settings.borrow();
        ui::popup_reset(&ui.model);
        for info in models::MODELS {
            let state = if info.is_downloaded() { "scaricato" } else { "da scaricare" };
            ui::popup_add(
                &ui.model,
                &format!(
                    "{} ({}, {state})",
                    info.description,
                    models::format_size(info.size_bytes)
                ),
            );
        }
        if let Some(custom) = &settings.custom_model_path {
            ui::popup_add(&ui.model, &format!("File personalizzato: {}", file_name(custom)));
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
        ui::popup_select(&ui.model, index);
    }

    fn selected_model(&self) -> Option<ModelChoice> {
        let index = ui::popup_selected(&self.ui().model)?;
        if let Some(info) = models::MODELS.get(index) {
            return Some(ModelChoice::Catalog(info));
        }
        self.ivars().settings.borrow().custom_model_path.clone().map(ModelChoice::Custom)
    }

    fn remember_model_choice(&self) {
        let choice = self.selected_model();
        let mut settings = self.ivars().settings.borrow_mut();
        match choice {
            Some(ModelChoice::Catalog(info)) => settings.model_id = info.id.to_string(),
            Some(ModelChoice::Custom(_)) => settings.model_id.clear(),
            None => {}
        }
    }

    /// Elenca le schede video; la preselezionata è quella salvata o, in mancanza,
    /// quella che sceglierebbe «Automatico».
    fn fill_device_popup(&self, selected: Device) {
        let popup = &self.ui().device;
        let gpus = device::list_gpus();
        ui::popup_reset(popup);
        if gpus.is_empty() {
            ui::popup_add(popup, "Nessuna scheda video utilizzabile");
            ui::popup_select(popup, 0);
            return;
        }
        for gpu in gpus {
            ui::popup_add(popup, &gpu.short_label());
        }
        let preferred = match selected {
            Device::Gpu(i) => Some(i),
            _ => match Device::Auto.resolve(gpus) {
                Ok(ResolvedDevice::Gpu(gpu)) => Some(gpu.index),
                _ => None,
            },
        };
        let index = preferred.and_then(|i| gpus.iter().position(|g| g.index == i)).unwrap_or(0);
        ui::popup_select(popup, index);
    }

    /// Dispositivo scelto: la scheda video selezionata se l'opzione è attiva, altrimenti la CPU.
    fn selected_device(&self) -> Device {
        let ui = self.ui();
        if !ui::is_checked(&ui.use_gpu) {
            return Device::Cpu;
        }
        ui::popup_selected(&ui.device)
            .and_then(|i| device::list_gpus().get(i))
            .map_or(Device::Auto, |g| Device::Gpu(g.index))
    }

    /// La scelta della scheda ha senso solo con la GPU attiva e almeno una scheda presente.
    fn update_device_enabled(&self) {
        let ui = self.ui();
        let enabled = ui::is_checked(&ui.use_gpu) && !device::list_gpus().is_empty();
        ui.device.setEnabled(enabled && !self.busy());
    }

    /// Frase che dice su quale dispositivo avverrà la trascrizione.
    fn device_summary(&self) -> String {
        match self.selected_device().resolve(device::list_gpus()) {
            Ok(ResolvedDevice::Gpu(gpu)) => {
                format!("La trascrizione userà la scheda video {}.", gpu.description)
            }
            _ if ui::is_checked(&self.ui().use_gpu) || device::list_gpus().is_empty() => {
                format!("La trascrizione userà il processore: {}", gpu_availability_text())
            }
            _ => "La trascrizione userà il processore (scheda video disattivata).".to_string(),
        }
    }

    fn on_use_gpu_toggled(&self) {
        let ui = self.ui();
        if ui::is_checked(&ui.use_gpu) && device::list_gpus().is_empty() {
            ui::set_checked(&ui.use_gpu, false);
            self.alert(Level::Info, "Scheda video non disponibile", &gpu_availability_text());
        }
        self.ivars().settings.borrow_mut().device = self.selected_device();
        self.update_device_enabled();
        self.set_status(&self.device_summary(), true);
    }

    fn toggle_flash_attention(&self) {
        let enabled = {
            let mut settings = self.ivars().settings.borrow_mut();
            settings.flash_attention = !settings.flash_attention;
            settings.flash_attention
        };
        let message = if enabled {
            "Accelerazione Flash Attention attivata: su alcune schede video accelera, su altre e sul processore rallenta. Confronta il tempo impiegato."
        } else {
            "Accelerazione Flash Attention disattivata."
        };
        self.set_status(message, true);
    }

    fn selected_language(&self) -> Option<String> {
        let index = ui::popup_selected(&self.ui().lang)?;
        index.checked_sub(1).and_then(|i| languages::LANGUAGES.get(i)).map(|l| l.code.to_string())
    }

    fn selected_format(&self) -> ExportFormat {
        ui::popup_selected(&self.ui().format)
            .and_then(|i| ExportFormat::ALL.get(i).copied())
            .unwrap_or(ExportFormat::Docx)
    }

    /// Abilita le voci di menu secondo lo stato; la voce Flash Attention mostra il segno di
    /// spunta quando è attiva.
    fn validate(&self, item: &NSMenuItem) -> bool {
        let Some(action) = item.action() else {
            return true;
        };
        let busy = self.busy();
        if action == sel!(toggleFlashAttention:) {
            ui::set_menu_checked(item, self.ivars().settings.borrow().flash_attention);
            return !busy;
        }
        if action == sel!(cancelJob:) {
            return busy;
        }
        let idle_only = [
            sel!(chooseAudio:),
            sel!(export:),
            sel!(startTranscription:),
            sel!(downloadModel:),
            sel!(chooseCustomModel:),
        ];
        !(busy && idle_only.contains(&action))
    }

    // ----- Azioni -----

    fn choose_audio(&self) {
        let folder = self.ivars().settings.borrow().last_audio_dir.clone();
        if let Some(path) = ui::open_file(
            "Apri file audio",
            "Scegli il file audio da trascrivere.",
            audio::SUPPORTED_EXTENSIONS,
            folder.as_deref(),
            self.mtm(),
        ) {
            self.set_audio_file(path, true);
        }
    }

    fn set_audio_file(&self, path: PathBuf, announce: bool) {
        let ui = self.ui();
        ui.audio_path.setStringValue(&ns(&path.display().to_string()));
        let name = file_name(&path);
        ui.window.setTitle(&ns(&format!("{name} – Trascrittore")));
        ui.window.setRepresentedURL(Some(&ui::file_url(&path)));
        self.ivars().settings.borrow_mut().last_audio_dir = path.parent().map(Path::to_path_buf);
        *self.ivars().audio_path.borrow_mut() = Some(path);
        self.set_status(
            &format!(
                "File audio selezionato: {name}. Premi Comando+R per avviare la trascrizione."
            ),
            announce,
        );
    }

    fn choose_custom_model(&self) {
        let folder = models::models_dir().ok();
        let Some(path) = ui::open_file(
            "Scegli un file modello Whisper",
            "Scegli un modello Whisper in formato GGML (file .bin).",
            &["bin"],
            folder.as_deref(),
            self.mtm(),
        ) else {
            return;
        };
        if let Err(e) = models::validate_model_file(&path) {
            self.alert(Level::Error, "Modello non valido", &format!("{e:#}"));
            return;
        }
        {
            let mut settings = self.ivars().settings.borrow_mut();
            settings.custom_model_path = Some(path.clone());
            settings.model_id.clear();
        }
        self.fill_model_popup();
        self.set_status(
            &format!("Modello personalizzato selezionato: {}.", file_name(&path)),
            true,
        );
        self.focus(&self.ui().model);
    }

    fn open_models_dir(&self) {
        match models::models_dir() {
            Ok(dir) => {
                NSWorkspace::sharedWorkspace().openURL(&ui::file_url(&dir));
            }
            Err(e) => {
                self.alert(Level::Error, "Cartella dei modelli non disponibile", &format!("{e:#}"))
            }
        }
    }

    fn has_unsaved_edits(&self) -> bool {
        let has_text = !self.ui().text.string().is_empty();
        has_text && (self.ivars().modified.get() || !self.ivars().exported.get())
    }

    /// Il pallino nel pulsante di chiusura indica un testo non ancora esportato.
    fn update_edited(&self) {
        self.ui().window.setDocumentEdited(self.has_unsaved_edits());
    }

    fn start_transcription(&self) {
        let Some(audio_path) = self.ivars().audio_path.borrow().clone() else {
            self.alert(
                Level::Info,
                "Nessun file audio",
                "Seleziona prima un file audio con Comando+O o con il pulsante Scegli.",
            );
            self.focus(&self.ui().browse);
            return;
        };
        if !audio_path.is_file() {
            self.alert(
                Level::Error,
                "File non trovato",
                &format!("Il file «{}» non esiste più.", audio_path.display()),
            );
            return;
        }
        if self.has_unsaved_edits()
            && !self.confirm(
                "Sostituire il testo trascritto?",
                "Il testo attuale non è stato esportato e verrà sostituito dalla nuova trascrizione.",
                "Sostituisci",
                true,
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
                    self.alert(
                        Level::Error,
                        "Cartella dei modelli non disponibile",
                        "Impossibile creare o leggere la cartella dei modelli.",
                    );
                    return;
                };
                if info.is_downloaded() {
                    (path, None)
                } else if self.confirm(
                    &format!("Scaricare il modello «{}»?", info.description),
                    &format!(
                        "Il modello non è ancora presente sul Mac. Dimensione: circa {}. Il download avviene una sola volta.",
                        models::format_size(info.size_bytes)
                    ),
                    "Scarica",
                    false,
                ) {
                    (path, Some(*info))
                } else {
                    return;
                }
            }
            ModelChoice::Custom(path) => (path.clone(), None),
        };

        let ui = self.ui();
        let translated = ui::is_checked(&ui.translate);
        let options = TranscribeOptions {
            language: self.selected_language(),
            translate: translated,
            threads: None,
            initial_prompt: None,
            fast: ui::is_checked(&ui.fast),
            openvino: None,
        };
        let job = Job {
            audio_path,
            model_path,
            device: self.selected_device(),
            engine: EngineOptions {
                flash_attention: self.ivars().settings.borrow().flash_attention,
            },
            options,
        };

        let cancel = self.begin_busy();
        let notifier = self.notifier();
        let cache = Arc::clone(&self.ivars().cache);
        *self.ivars().pending.borrow_mut() = Some((model.display_name(), translated));

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
            self.alert(
                Level::Info,
                "Nessun modello da scaricare",
                "Seleziona un modello dell'elenco da scaricare.",
            );
            return;
        };
        if info.is_downloaded() {
            self.set_status(&format!("Il modello «{}» è già scaricato.", info.description), true);
            return;
        }
        if !self.confirm(
            &format!("Scaricare il modello «{}»?", info.description),
            &format!("Dimensione: circa {}.", models::format_size(info.size_bytes)),
            "Scarica",
            false,
        ) {
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
        // Il pulsante resta attivo: disattivarlo mentre ha il focus farebbe perdere la
        // posizione a VoiceOver. Le pressioni ripetute sono ignorate.
        if self.busy() && !self.ivars().cancel_flag.borrow().swap(true, Ordering::Relaxed) {
            // whisper.cpp si ferma solo al termine del blocco di audio in elaborazione.
            self.set_status(
                "Interruzione in corso: attendi qualche secondo, termina il blocco di audio in elaborazione.",
                true,
            );
        }
    }

    fn notifier(&self) -> Notifier {
        Notifier { tx: self.ivars().events_tx.clone(), target: self as *const Self as usize }
    }

    fn on_timestamps_toggled(&self) {
        let ui = self.ui();
        let checked = ui::is_checked(&ui.timestamps);
        let text = {
            let current = self.ivars().current.borrow();
            match current.as_ref() {
                Some(c) if !c.transcript.segments.is_empty() => Some(c.transcript.to_text(checked)),
                _ => None,
            }
        };
        if let Some(text) = text {
            if self.ivars().modified.get()
                && !self.confirm(
                    "Perdere le correzioni?",
                    "Le correzioni fatte a mano nel testo andranno perse.",
                    "Continua",
                    true,
                )
            {
                ui::set_checked(&ui.timestamps, !checked);
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
        self.ivars().settings.borrow_mut().timestamps = checked;
    }

    fn set_transcript_text(&self, text: &str) {
        let view = &self.ui().text;
        view.setString(&ns(text));
        // Le modifiche precedenti si riferiscono a un altro testo.
        if let Some(undo) = view.undoManager() {
            undo.removeAllActions();
        }
        ui::move_caret_to_start(view);
        self.ivars().modified.set(false);
        self.update_edited();
    }

    fn export(&self) {
        let ui = self.ui();
        let text = ui::text_of(&ui.text);
        if text.trim().is_empty() {
            self.alert(
                Level::Info,
                "Niente da esportare",
                "Non c'è ancora testo da esportare. Avvia prima una trascrizione con Comando+R.",
            );
            return;
        }
        let selected = self.selected_format();
        let audio_path = self.ivars().audio_path.borrow().clone();
        let stem = audio_path
            .as_deref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "trascrizione".to_string());
        let folder = self
            .ivars()
            .settings
            .borrow()
            .last_export_dir
            .clone()
            .or_else(|| audio_path.as_deref().and_then(Path::parent).map(Path::to_path_buf));

        let mtm = self.mtm();
        let panel = NSSavePanel::savePanel(mtm);
        panel.setTitle(Some(&ns("Esporta la trascrizione")));
        panel.setPrompt(Some(&ns("Esporta")));
        panel.setNameFieldLabel(Some(&ns("Esporta come:")));
        panel.setNameFieldStringValue(&ns(&format!("{stem}.{}", selected.extension())));
        panel.setCanCreateDirectories(true);
        panel.setExtensionHidden(false);
        panel.setAllowedContentTypes(&ui::content_types(&[selected.extension()]));
        if let Some(folder) = folder.filter(|f| f.is_dir()) {
            panel.setDirectoryURL(Some(&ui::file_url(&folder)));
        }

        // Scelta del formato dentro il pannello, come l'elenco dei tipi di file su Windows.
        let accessory = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 440.0, 44.0));
        let label = ui::label("Formato:", mtm);
        let popup = ui::popup("Formato", self.target(), sel!(saveFormatChanged:), mtm);
        for format in ExportFormat::ALL {
            ui::popup_add(&popup, format.label());
        }
        ui::popup_select(
            &popup,
            ExportFormat::ALL.iter().position(|f| *f == selected).unwrap_or(0),
        );
        let label_size = label.intrinsicContentSize();
        let popup_h = popup.intrinsicContentSize().height;
        label.setFrame(rect(
            20.0,
            (44.0 - label_size.height) / 2.0,
            label_size.width,
            label_size.height,
        ));
        popup.setFrame(rect(28.0 + label_size.width, (44.0 - popup_h) / 2.0, 280.0, popup_h));
        accessory.addSubview(&label);
        accessory.addSubview(&popup);
        panel.setAccessoryView(Some(&accessory));

        *self.ivars().save_panel.borrow_mut() = Some(panel.clone());
        let response = panel.runModal();
        self.ivars().save_panel.borrow_mut().take();
        if response != NSModalResponseOK {
            return;
        }
        let Some(path) = panel.URL().and_then(|url| ui::url_path(&url)) else {
            return;
        };
        let chosen = ui::popup_selected(&popup)
            .and_then(|i| ExportFormat::ALL.get(i).copied())
            .unwrap_or(selected);
        let format = ExportFormat::from_path(&path).unwrap_or(chosen);
        if format != selected {
            if let Some(index) = ExportFormat::ALL.iter().position(|f| *f == format) {
                ui::popup_select(&ui.format, index);
            }
            self.ivars().settings.borrow_mut().export_format = format;
        }

        let document = {
            let current = self.ivars().current.borrow();
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
                self.ivars().exported.set(true);
                self.ivars().modified.set(false);
                self.update_edited();
                self.ivars().settings.borrow_mut().last_export_dir =
                    path.parent().map(Path::to_path_buf);
                self.set_status(&format!("Esportazione completata: {}.", file_name(&path)), true);
            }
            Err(e) => {
                self.set_status("Esportazione non riuscita.", false);
                self.alert(Level::Error, "Esportazione non riuscita", &format!("{e:#}"));
            }
        }
    }

    /// Nel pannello di esportazione il formato scelto cambia anche l'estensione del nome.
    fn on_save_format_changed(&self, popup: &NSPopUpButton) {
        let Some(panel) = self.ivars().save_panel.borrow().clone() else {
            return;
        };
        let Some(format) =
            ui::popup_selected(popup).and_then(|i| ExportFormat::ALL.get(i).copied())
        else {
            return;
        };
        panel.setAllowedContentTypes(&ui::content_types(&[format.extension()]));
        let name = panel.nameFieldStringValue().to_string();
        let stem =
            Path::new(&name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name);
        panel.setNameFieldStringValue(&ns(&format!("{stem}.{}", format.extension())));
    }

    fn copy_all(&self) {
        let text = ui::text_of(&self.ui().text);
        if text.is_empty() {
            self.set_status("Non c'è testo da copiare.", true);
            return;
        }
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        unsafe { pasteboard.setString_forType(&ns(&text), NSPasteboardTypeString) };
        self.set_status("Testo copiato negli appunti.", true);
    }

    fn zoom(&self, delta: i32) {
        let points = {
            let mut settings = self.ivars().settings.borrow_mut();
            let points = if delta == 0 {
                DEFAULT_FONT_POINTS
            } else {
                (settings.transcript_font_points as i32 + delta)
                    .clamp(MIN_FONT_POINTS as i32, MAX_FONT_POINTS as i32) as u32
            };
            settings.transcript_font_points = points;
            points
        };
        self.update_text_font();
        self.set_status(&format!("Dimensione del testo: {points} punti."), true);
    }

    fn update_text_font(&self) {
        let points = self
            .ivars()
            .settings
            .borrow()
            .transcript_font_points
            .clamp(MIN_FONT_POINTS, MAX_FONT_POINTS);
        ui::set_text_view_font(&self.ui().text, f64::from(points));
    }

    fn about_text(&self) -> String {
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
        let mut text = format!(
            "{} {}\n\nTrascrizione audio in locale con Whisper (whisper.cpp {}).\nNessun dato viene inviato in rete: solo i modelli vengono scaricati, su richiesta.\n\nSupporto schede video: {gpu_support}\nSchede video utilizzabili:\n{gpu_list}\n{summary}\nFlash Attention: {flash}\n\nCartella dei modelli:\n{}",
            trascrittore_core::APP_NAME,
            trascrittore_core::APP_VERSION,
            transcriber::engine_version(),
            models::models_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_default(),
            summary = self.device_summary(),
            flash = if self.ivars().settings.borrow().flash_attention {
                "attiva"
            } else {
                "disattivata"
            },
        );
        if let Some(data) = paths::portable_data_dir() {
            text.push_str(&format!(
                "\n\nModalità portable: impostazioni e modelli nella cartella\n{}",
                data.display()
            ));
        }
        text.push_str("\n\n");
        text.push_str(trascrittore_core::LEGAL_NOTICE);
        text
    }

    /// Mostra (o riporta in primo piano, aggiornata) la finestra di testo con questo titolo.
    fn show_text_window(&self, title: &str, text: &str) {
        let mut windows = self.ivars().text_windows.borrow_mut();
        windows.retain(|w| {
            let same = w.title().to_string() == title;
            if same {
                w.close();
            }
            !same
        });
        windows.push(ui::text_window(title, text, self.mtm()));
    }

    fn confirm_close(&self) -> bool {
        if self.busy()
            && !self.confirm(
                "Interrompere l'operazione e uscire?",
                "È in corso un'operazione.",
                "Esci",
                true,
            )
        {
            return false;
        }
        if self.has_unsaved_edits()
            && !self.confirm(
                "Uscire senza esportare?",
                "Il testo trascritto non è stato esportato.",
                "Esci",
                true,
            )
        {
            return false;
        }
        self.ivars().cancel_flag.borrow().store(true, Ordering::Relaxed);
        true
    }

    fn save_settings(&self) {
        // La verifica automatica non deve cambiare le preferenze dell'utente.
        if !self.checking() {
            let _ = self.ivars().settings.borrow().save();
        }
    }

    // ----- Finestre di messaggio -----

    fn alert(&self, level: Level, title: &str, text: &str) {
        if self.checking() {
            println!("Finestra di messaggio: {title}. {text}");
            return;
        }
        ui::alert(level, title, text, self.mtm());
    }

    fn confirm(&self, title: &str, text: &str, action: &str, safe_default: bool) -> bool {
        if self.checking() {
            println!("Conferma accettata: {title} {text}");
            return true;
        }
        ui::confirm(title, text, action, safe_default, self.mtm())
    }

    // ----- Stato «occupato» -----

    /// Controlli disattivati durante un'operazione.
    fn idle_controls(&self) -> [&NSControl; 11] {
        let ui = self.ui();
        [
            &ui.browse,
            &ui.model,
            &ui.lang,
            &ui.use_gpu,
            &ui.device,
            &ui.translate,
            &ui.timestamps,
            &ui.fast,
            &ui.start,
            &ui.format,
            &ui.export,
        ]
    }

    fn begin_busy(&self) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        *self.ivars().cancel_flag.borrow_mut() = Arc::clone(&cancel);
        let state = self.ivars();
        state.busy.set(true);
        state.phase.set(None);
        state.last_milestone.set(0);
        state.started.set(Some(Instant::now()));
        let ui = self.ui();
        ui.progress.setDoubleValue(0.0);

        // Il focus va su «Interrompi» prima di disattivare il pulsante premuto,
        // altrimenti VoiceOver perderebbe la posizione.
        ui.cancel.setEnabled(true);
        if !self.focus_is_text() {
            self.focus(&ui.cancel);
        }
        for control in self.idle_controls() {
            control.setEnabled(false);
        }
        cancel
    }

    fn end_busy(&self, focus: &NSView) {
        let state = self.ivars();
        state.busy.set(false);
        state.phase.set(None);
        for control in self.idle_controls() {
            control.setEnabled(true);
        }
        self.update_device_enabled();
        self.focus(focus);
        self.ui().cancel.setEnabled(false);
    }

    // ----- Eventi del thread di lavoro -----

    fn drain_events(&self) {
        while let Ok(event) = self.ivars().events_rx.try_recv() {
            self.handle_event(event);
        }
    }

    fn handle_event(&self, event: WorkerEvent) {
        let ui = self.ui();
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
                self.ivars().phase.set(Some(Phase::LoadingModel));
                ui.progress.setDoubleValue(0.0);
                self.set_status(&format!("Caricamento del modello su {}…", device.label()), true);
            }
            WorkerEvent::Pipeline(Progress::Transcribing(percent)) => {
                self.enter_phase(Phase::Transcribing);
                self.show_progress(Phase::Transcribing, percent.max(0) as u32);
            }
            WorkerEvent::Transcribed(outcome) => self.on_transcribed(outcome),
            WorkerEvent::DownloadFinished => {
                self.fill_model_popup();
                self.end_busy(&ui.model);
                self.set_status("Download del modello completato.", true);
            }
            WorkerEvent::Failed(message) => {
                self.end_busy(&ui.start);
                self.set_status("Operazione non riuscita.", false);
                self.alert(Level::Error, "Operazione non riuscita", &message);
                if self.checking() {
                    self.finish_check(&[format!("operazione non riuscita: {message}")]);
                }
            }
            WorkerEvent::Cancelled => {
                self.fill_model_popup();
                self.end_busy(&ui.start);
                self.set_status("Operazione interrotta.", true);
            }
        }
    }

    fn enter_phase(&self, phase: Phase) {
        if self.ivars().phase.get() != Some(phase) {
            self.ivars().phase.set(Some(phase));
            self.ivars().last_milestone.set(0);
            self.set_status(&format!("{}…", phase.description()), true);
        }
    }

    fn show_progress(&self, phase: Phase, percent: u32) {
        let percent = percent.min(100);
        self.ui().progress.setDoubleValue(f64::from(percent));
        let text = format!("{}: {percent}%", phase.description());
        // Annuncio vocale solo ai traguardi del 25%, per non disturbare la lettura.
        let milestone = percent / 25 * 25;
        let announce = matches!(phase, Phase::Downloading | Phase::Transcribing)
            && milestone > self.ivars().last_milestone.get()
            && milestone < 100;
        if announce {
            self.ivars().last_milestone.set(milestone);
        }
        self.set_status(&text, announce);
    }

    fn on_transcribed(&self, outcome: Outcome) {
        let Outcome { transcript, device, gpu_warning, openvino: _ } = outcome;
        let ui = self.ui();
        let elapsed = self
            .ivars()
            .started
            .get()
            .map(|t| format_duration_words(t.elapsed().as_millis() as u64));
        let text = transcript.to_text(ui::is_checked(&ui.timestamps));
        let words = transcript.word_count();
        let duration = format_duration_words(transcript.audio_duration_ms);
        let language = transcript.language.as_deref().map(languages::display_name);
        let (model_name, translated) = self.ivars().pending.borrow_mut().take().unwrap_or_default();
        *self.ivars().current.borrow_mut() =
            Some(CurrentTranscript { transcript, model_name, translated });
        self.set_transcript_text(&text);
        self.ivars().exported.set(false);
        self.update_edited();
        self.fill_model_popup();
        ui.progress.setDoubleValue(100.0);

        let mut status =
            format!("Trascrizione completata: {words} parole, durata dell'audio {duration}");
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
        if let Some(warning) = &gpu_warning {
            status.push(' ');
            status.push_str(warning);
        }
        // Prima si sposta il focus sul testo, poi si annuncia l'esito.
        self.end_busy(&ui.text);
        self.set_status(&status, true);
        if let Some(warning) = gpu_warning {
            if !self.ivars().gpu_warning_shown.replace(true) {
                self.alert(Level::Warning, "Scheda video non usata", &warning);
            }
        }
        if self.checking() {
            // Dopo un breve intervallo, quando la finestra è stata ridisegnata.
            unsafe {
                let _: () = msg_send![
                    self,
                    performSelector: sel!(checkTranscribed:),
                    withObject: None::<&AnyObject>,
                    afterDelay: 0.5f64
                ];
            }
        }
    }

    // ----- Stato e annunci -----

    fn set_status(&self, text: &str, announce: bool) {
        *self.ivars().status.borrow_mut() = text.to_string();
        if let Some(ui) = self.ivars().ui.get() {
            ui.status.setStringValue(&ns(text));
            self.layout();
        }
        if announce {
            self.announce(text);
        }
    }

    fn announce(&self, text: &str) {
        if self.checking() {
            println!("Annuncio: {text}");
        }
        ui::announce(&self.ui().window, text);
    }

    // ----- Focus -----

    fn focus(&self, view: &NSView) -> bool {
        self.ui().window.makeFirstResponder(Some(view))
    }

    /// Porta il focus su un menu a comparsa; se la navigazione da tastiera completa è
    /// disattivata (il controllo non può ricevere il focus) apre direttamente il menu.
    fn focus_popup(&self, popup: &NSPopUpButton) {
        if !self.focus(popup) && popup.isEnabled() && !self.checking() {
            unsafe { popup.performClick(None) };
        }
    }

    fn focus_is_text(&self) -> bool {
        let ui = self.ui();
        ui.window.firstResponder().is_some_and(|responder| {
            std::ptr::eq(
                Retained::as_ptr(&responder).cast::<AnyObject>(),
                Retained::as_ptr(&ui.text).cast::<AnyObject>(),
            )
        })
    }

    // ----- Disposizione -----

    /// Dispone i controlli dall'alto verso il basso; il testo trascritto occupa lo spazio
    /// rimanente. Richiamata a ogni ridimensionamento della finestra.
    fn layout(&self) {
        let Some(ui) = self.ivars().ui.get() else {
            return;
        };
        let size = ui.content.frame().size;
        let (width, height) = (size.width, size.height);
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let natural = |view: &NSView| view.intrinsicContentSize();
        let margin = 20.0;
        let gap = 10.0;
        let label_w = [
            &ui.audio_label,
            &ui.model_label,
            &ui.lang_label,
            &ui.device_label,
            &ui.progress_label,
            &ui.status_label,
        ]
        .iter()
        .map(|l| natural(l).width)
        .fold(0.0, f64::max);
        let x1 = margin + label_w + 8.0;
        let right = width - margin;
        let field_w = (right - x1).max(160.0);
        let row_h =
            [natural(&ui.model).height, natural(&ui.browse).height, natural(&ui.audio_path).height]
                .into_iter()
                .fold(22.0, f64::max);
        let place = |view: &NSView, x: f64, y: f64, w: f64, h: f64| view.setFrame(rect(x, y, w, h));
        // Controllo centrato verticalmente nella riga.
        let place_row = |view: &NSView, x: f64, y: f64, w: f64| {
            let h = natural(view).height;
            let h = if h > 0.0 { h.min(row_h) } else { row_h };
            place(view, x, y + (row_h - h) / 2.0, w, h);
        };
        let place_label = |label: &NSTextField, y: f64| place_row(label, margin, y, label_w);
        let mut y = margin;

        let browse_w = natural(&ui.browse).width.max(96.0);
        place_label(&ui.audio_label, y);
        place_row(&ui.audio_path, x1, y, field_w - browse_w - gap);
        place_row(&ui.browse, right - browse_w, y, browse_w);
        y += row_h + gap;

        place_label(&ui.model_label, y);
        place_row(&ui.model, x1, y, field_w);
        y += row_h + gap;

        place_label(&ui.lang_label, y);
        place_row(&ui.lang, x1, y, field_w.min(360.0));
        y += row_h + gap;

        place_row(&ui.use_gpu, x1, y, natural(&ui.use_gpu).width);
        y += row_h + gap;

        place_label(&ui.device_label, y);
        place_row(&ui.device, x1, y, field_w);
        y += row_h + gap;

        // Opzioni sulla stessa riga, che va a capo se la finestra è stretta.
        let mut x = x1;
        for check in [&ui.translate, &ui.timestamps, &ui.fast] {
            let w = natural(check).width;
            if x > x1 && x + w > right {
                x = x1;
                y += row_h + gap / 2.0;
            }
            place_row(check, x, y, w);
            x += w + 24.0;
        }
        y += row_h + gap;

        let start_w = natural(&ui.start).width.max(140.0);
        place_row(&ui.start, x1, y, start_w);
        place_row(&ui.cancel, x1 + start_w + gap, y, natural(&ui.cancel).width.max(96.0));
        y += row_h + gap * 1.5;

        place_label(&ui.progress_label, y);
        place_row(&ui.progress, x1, y, field_w);
        y += row_h + gap;

        ui.status.setPreferredMaxLayoutWidth(field_w);
        let status_h = natural(&ui.status).height.clamp(row_h, 96.0);
        let status_label_h = natural(&ui.status_label).height;
        place(&ui.status_label, margin, y, label_w, status_label_h);
        place(&ui.status, x1, y, field_w, status_h);
        y += status_h + gap;

        let text_label_h = natural(&ui.text_label).height;
        place(&ui.text_label, margin, y, right - margin, text_label_h);
        y += text_label_h + 4.0;

        let bottom_y = height - margin - row_h;
        place(&ui.text_scroll, margin, y, right - margin, (bottom_y - gap - y).max(80.0));

        let format_label_w = natural(&ui.format_label).width;
        let format_w = 260.0;
        place_row(&ui.format_label, margin, bottom_y, format_label_w);
        place_row(&ui.format, margin + format_label_w + 8.0, bottom_y, format_w);
        place_row(
            &ui.export,
            margin + format_label_w + 8.0 + format_w + gap,
            bottom_y,
            natural(&ui.export).width.max(110.0),
        );
    }

    // ----- Verifica automatica -----

    fn run_check(&self) {
        let Some(options) = &self.ivars().check else {
            return;
        };
        let ui = self.ui();
        let mut problems = check::verify(ui);
        for (title, text) in [
            ("Guida rapida di Trascrittore", HELP_TEXT.to_string()),
            ("Informazioni su Trascrittore", self.about_text()),
        ] {
            self.show_text_window(title, &text);
            if let Some(window) = self.ivars().text_windows.borrow().last() {
                problems.extend(check::verify_text_window(window, title));
                window.close();
            }
        }
        ui.window.makeKeyAndOrderFront(None);
        if let Some(path) = &options.snapshot {
            match check::snapshot(&ui.window, path) {
                Ok(()) => println!("Istantanea della finestra salvata in {}", path.display()),
                Err(e) => problems.push(format!("istantanea non salvata: {e}")),
            }
        }
        if !problems.is_empty() || self.ivars().audio_path.borrow().is_none() {
            self.finish_check(&problems);
        }
        // Con un file audio la verifica prosegue con una trascrizione vera.
        println!("=== Trascrizione dalla finestra");
        self.start_transcription();
        if !self.busy() {
            self.finish_check(&["la trascrizione non è partita".to_string()]);
        }
    }

    fn finish_check_after_transcription(&self) {
        let text = ui::text_of(&self.ui().text);
        let mut problems = Vec::new();
        println!("Testo trascritto: {}", text.trim());
        if let Some(path) = self.ivars().check.as_ref().and_then(|c| c.snapshot.as_ref()) {
            let path = path.with_file_name(format!(
                "{}-trascritto.png",
                path.file_stem().unwrap_or_default().to_string_lossy()
            ));
            match check::snapshot(&self.ui().window, &path) {
                Ok(()) => println!("Istantanea dopo la trascrizione salvata in {}", path.display()),
                Err(e) => problems.push(format!("istantanea non salvata: {e}")),
            }
        }
        if text.trim().is_empty() {
            problems.push("nessun testo trascritto".to_string());
        }
        if self.focus_is_text() {
            println!("Al termine il focus è sul testo trascritto.");
        } else {
            problems.push("al termine il focus non è sul testo trascritto".to_string());
        }

        // Comandi di menu: di nuovo disponibili, tranne «Interrompi».
        let app = NSApplication::sharedApplication(self.mtm());
        for (action, expected) in [
            (sel!(startTranscription:), true),
            (sel!(export:), true),
            (sel!(chooseAudio:), true),
            (sel!(cancelJob:), false),
        ] {
            let item = app.mainMenu().and_then(|bar| find_menu_item(&bar, action));
            match item {
                Some(item) if self.validate(&item) == expected => {}
                Some(item) => problems.push(format!(
                    "la voce di menu «{}» è {}",
                    item.title(),
                    if expected { "disattivata" } else { "attiva" }
                )),
                None => problems.push(format!("manca la voce di menu per {action:?}")),
            }
        }

        // Timestamp inseriti e tolti dal testo senza ritrascrivere.
        let ui = self.ui();
        ui::set_checked(&ui.timestamps, true);
        self.on_timestamps_toggled();
        let with_timestamps = ui::text_of(&ui.text);
        println!("Con i timestamp: {}", with_timestamps.trim());
        if !with_timestamps.starts_with('[') {
            problems.push("i timestamp non sono stati inseriti nel testo".to_string());
        }
        ui::set_checked(&ui.timestamps, false);
        self.on_timestamps_toggled();
        if ui::text_of(&ui.text) != text {
            problems.push("togliendo i timestamp il testo non torna quello originale".to_string());
        }
        self.finish_check(&problems);
    }

    /// Termina la verifica con il codice di uscita dell'esito.
    fn finish_check(&self, problems: &[String]) -> ! {
        let code = check::report(problems);
        self.release_engine();
        std::process::exit(code);
    }

    /// Libera il modello caricato prima dell'uscita: con Metal i distruttori globali di ggml
    /// si interrompono (GGML_ASSERT) se trovano ancora buffer del modello sulla scheda video.
    ///
    /// Se una trascrizione sta ancora finendo il blocco in corso, il modello non si può liberare
    /// in tempo: il programma esce subito senza eseguire i distruttori globali (le preferenze
    /// sono già salvate).
    fn release_engine(&self) {
        let cache = &self.ivars().cache;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match cache.try_lock() {
                Ok(mut engine) => {
                    *engine = None;
                    return;
                }
                Err(TryLockError::Poisoned(e)) => {
                    *e.into_inner() = None;
                    return;
                }
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(TryLockError::WouldBlock) => {
                    let _ = std::io::stdout().flush();
                    // SAFETY: `_exit` termina il processo senza eseguire codice di uscita.
                    unsafe { _exit(0) }
                }
            }
        }
    }
}

extern "C" {
    fn _exit(status: i32) -> !;
}

/// Prima voce di menu (anche nei sottomenu) con l'azione indicata.
fn find_menu_item(menu: &NSMenu, action: Sel) -> Option<Retained<NSMenuItem>> {
    menu.itemArray().iter().find_map(|item| {
        if item.action() == Some(action) {
            Some(item)
        } else {
            item.submenu().and_then(|sub| find_menu_item(&sub, action))
        }
    })
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Spiega se e perché la scheda video è disponibile.
fn gpu_availability_text() -> String {
    let backends = device::compiled_gpu_backends();
    let gpus = device::list_gpus();
    if backends.is_empty() {
        "Questa versione di Trascrittore è stata compilata senza supporto per la scheda video e usa solo il processore. Usa la versione con Metal, creata da scripts/build-macos.sh."
            .to_string()
    } else if gpus.is_empty() {
        format!(
            "Nessuna scheda video compatibile trovata ({}): {}. Nel frattempo si usa il processore.",
            backends.join(", "),
            device::gpu_driver_hint()
        )
    } else {
        let names: Vec<&str> = gpus.iter().map(|g| g.description.as_str()).collect();
        format!("Schede video disponibili: {}. Molto più veloci del processore.", names.join(", "))
    }
}

/// Inoltra l'avanzamento di un download a ogni punto percentuale.
fn download_progress(notifier: &Notifier) -> impl FnMut(u64, Option<u64>) + '_ {
    notifier.send(WorkerEvent::Downloading(0));
    let mut last = 0u32;
    move |done, total| {
        if let Some(total) = total.filter(|t| *t > 0) {
            let percent = (done * 100 / total) as u32;
            if percent != last {
                last = percent;
                notifier.send(WorkerEvent::Downloading(percent));
            }
        }
    }
}

/// Scarica un modello inoltrando l'avanzamento; restituisce `false` in caso di errore o
/// annullamento (l'evento corrispondente è già stato inviato).
fn download_with_progress(info: &ModelInfo, notifier: &Notifier, cancel: &AtomicBool) -> bool {
    match models::download(info, download_progress(notifier), cancel) {
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
