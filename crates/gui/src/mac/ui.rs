//! Piccoli aiuti sopra AppKit: controlli con nome accessibile, annunci per VoiceOver,
//! finestre di messaggio e pannelli di apertura e salvataggio.

#![allow(unused_unsafe)]

use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityAnnouncementKey,
    NSAccessibilityAnnouncementRequestedNotification, NSAccessibilityPostNotificationWithUserInfo,
    NSAccessibilityPriorityKey, NSAccessibilityPriorityLevel, NSAccessibilityTextFieldRole,
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSAutoresizingMaskOptions, NSBackingStoreType,
    NSBezelStyle, NSBorderType, NSButton, NSCell, NSControl, NSControlStateValueOff,
    NSControlStateValueOn, NSFont, NSLineBreakMode, NSMenuItem, NSModalResponseOK, NSOpenPanel,
    NSPopUpButton, NSProgressIndicator, NSProgressIndicatorStyle, NSScrollView, NSTextAlignment,
    NSTextField, NSTextView, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSNumber, NSPoint, NSRange, NSRect, NSSize, NSString, NSURL,
};
use objc2_uniform_type_identifiers::UTType;

pub fn ns(text: &str) -> Retained<NSString> {
    NSString::from_str(text)
}

pub fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w.max(1.0), h.max(1.0)))
}

define_class!(
    /// Vista con l'origine in alto a sinistra: la disposizione procede dall'alto verso il
    /// basso, come l'ordine di lettura e di tabulazione.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "TrascrittoreVistaCapovolta"]
    pub struct FlippedView;

    impl FlippedView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl FlippedView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

/// Cella del controllo. Per campi di testo, menu a comparsa e pulsanti l'elemento che
/// VoiceOver legge è la cella, non la vista: le proprietà di accessibilità vanno impostate
/// anche lì (non tutti i controlli le inoltrano da soli).
fn cell(view: &NSView) -> Option<Retained<NSCell>> {
    view.downcast_ref::<NSControl>().and_then(|control| control.cell())
}

/// Imposta il nome letto da VoiceOver (senza i due punti dell'etichetta visibile).
pub fn set_name(view: &NSView, name: &str) {
    let name = ns(name);
    view.setAccessibilityLabel(Some(&name));
    if let Some(cell) = cell(view) {
        cell.setAccessibilityLabel(Some(&name));
    }
}

/// Imposta la descrizione aggiuntiva, letta da VoiceOver dopo una breve pausa e mostrata
/// anche come suggerimento al passaggio del mouse.
pub fn set_help(view: &NSView, help: &str) {
    let help = ns(help);
    view.setAccessibilityHelp(Some(&help));
    view.setToolTip(Some(&help));
    if let Some(cell) = cell(view) {
        cell.setAccessibilityHelp(Some(&help));
    }
}

/// Esclude una vista dall'albero di accessibilità (per esempio un'etichetta il cui testo è
/// già il nome del controllo accanto).
pub fn hide_from_voiceover(view: &NSView) {
    view.setAccessibilityElement(false);
    if let Some(cell) = cell(view) {
        cell.setAccessibilityElement(false);
    }
}

/// Etichetta visibile accanto a un controllo. Per VoiceOver il nome è già sul controllo,
/// quindi l'etichetta non viene letta una seconda volta come testo a sé.
pub fn label(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&ns(text), mtm);
    label.setAlignment(NSTextAlignment::Right);
    hide_from_voiceover(&label);
    label
}

/// Etichetta che VoiceOver legge come testo, per i valori che non hanno un controllo
/// accanto con il proprio nome (es. «Stato:»).
pub fn visible_label(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&ns(text), mtm);
    label.setAlignment(NSTextAlignment::Right);
    label
}

pub fn button(
    title: &str,
    target: &AnyObject,
    action: Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(&ns(title), Some(target), Some(action), mtm)
    };
    button.setBezelStyle(NSBezelStyle::Push);
    button
}

pub fn checkbox(
    title: &str,
    target: &AnyObject,
    action: Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    unsafe {
        NSButton::checkboxWithTitle_target_action(&ns(title), Some(target), Some(action), mtm)
    }
}

pub fn is_checked(button: &NSButton) -> bool {
    button.state() == NSControlStateValueOn
}

pub fn set_checked(button: &NSButton, checked: bool) {
    button.setState(if checked { NSControlStateValueOn } else { NSControlStateValueOff });
}

pub fn set_menu_checked(item: &NSMenuItem, checked: bool) {
    item.setState(if checked { NSControlStateValueOn } else { NSControlStateValueOff });
}

pub fn popup(
    name: &str,
    target: &AnyObject,
    action: Sel,
    mtm: MainThreadMarker,
) -> Retained<NSPopUpButton> {
    let popup =
        NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, false);
    unsafe {
        popup.setTarget(Some(target));
        popup.setAction(Some(action));
    }
    set_name(&popup, name);
    popup
}

pub fn popup_reset(popup: &NSPopUpButton) {
    popup.removeAllItems();
}

pub fn popup_add(popup: &NSPopUpButton, title: &str) {
    // Attraverso il menu: `addItemWithTitle:` scarterebbe le voci con titoli uguali.
    if let Some(menu) = popup.menu() {
        unsafe { menu.addItemWithTitle_action_keyEquivalent(&ns(title), None, &ns("")) };
    }
}

pub fn popup_selected(popup: &NSPopUpButton) -> Option<usize> {
    usize::try_from(popup.indexOfSelectedItem()).ok()
}

pub fn popup_select(popup: &NSPopUpButton, index: usize) {
    popup.selectItemAtIndex(index as isize);
}

/// Campo di sola lettura, raggiungibile con il Tab e selezionabile.
pub fn read_only_field(text: &str, name: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::textFieldWithString(&ns(text), mtm);
    field.setEditable(false);
    field.setSelectable(true);
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
    field.setUsesSingleLineMode(true);
    // Un campo non modificabile verrebbe letto come testo statico, con il nome al posto del
    // contenuto: come campo di testo VoiceOver legge sia il nome sia il percorso.
    if let Some(cell) = cell(&field) {
        cell.setAccessibilityRole(Some(unsafe { NSAccessibilityTextFieldRole }));
    }
    set_name(&field, name);
    field
}

/// Testo di stato: per VoiceOver il suo nome è il contenuto stesso.
pub fn status_text(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::wrappingLabelWithString(&ns(text), mtm);
    // Fuori dall'ordine di tabulazione, come su Windows: si legge con Comando+L o con VoiceOver.
    field.setSelectable(false);
    field
}

pub fn progress_bar(name: &str, mtm: MainThreadMarker) -> Retained<NSProgressIndicator> {
    let progress = NSProgressIndicator::new(mtm);
    progress.setStyle(NSProgressIndicatorStyle::Bar);
    progress.setIndeterminate(false);
    progress.setMinValue(0.0);
    progress.setMaxValue(100.0);
    progress.setDoubleValue(0.0);
    set_name(&progress, name);
    progress
}

/// Area di testo multiriga con barra di scorrimento.
pub fn text_area(
    name: &str,
    editable: bool,
    mtm: MainThreadMarker,
) -> (Retained<NSScrollView>, Retained<NSTextView>) {
    let scroll = NSTextView::scrollableTextView(mtm);
    scroll.setBorderType(NSBorderType::BezelBorder);
    let text = scroll
        .documentView()
        .and_then(|view| view.downcast::<NSTextView>().ok())
        .expect("NSTextView::scrollableTextView contiene sempre una NSTextView");
    text.setRichText(false);
    text.setImportsGraphics(false);
    text.setEditable(editable);
    text.setSelectable(true);
    text.setAllowsUndo(editable);
    text.setUsesFindBar(true);
    text.setAutomaticQuoteSubstitutionEnabled(false);
    text.setAutomaticDashSubstitutionEnabled(false);
    text.setAutomaticTextReplacementEnabled(false);
    set_name(&scroll, name);
    set_name(&text, name);
    (scroll, text)
}

pub fn text_of(text: &NSTextView) -> String {
    text.string().to_string()
}

pub fn set_text_view_font(text: &NSTextView, points: f64) {
    text.setFont(Some(&NSFont::systemFontOfSize(points)));
}

pub fn move_caret_to_start(text: &NSTextView) {
    let start = NSRange::new(0, 0);
    text.setSelectedRange(start);
    text.scrollRangeToVisible(start);
}

/// Fa pronunciare un messaggio a VoiceOver senza spostare il focus.
pub fn announce(element: &AnyObject, message: &str) {
    let message = ns(message);
    let priority = NSNumber::new_isize(NSAccessibilityPriorityLevel::High.0);
    unsafe {
        let keys = [NSAccessibilityAnnouncementKey, NSAccessibilityPriorityKey];
        let values: [&AnyObject; 2] = [&message, &priority];
        let info = NSDictionary::from_slices(&keys, &values);
        NSAccessibilityPostNotificationWithUserInfo(
            element,
            NSAccessibilityAnnouncementRequestedNotification,
            Some(&info),
        );
    }
}

/// Tipo di finestra di messaggio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warning,
    Error,
}

/// Finestra di messaggio standard, sempre letta da VoiceOver.
pub fn alert(level: Level, title: &str, text: &str, mtm: MainThreadMarker) {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(match level {
        Level::Info => NSAlertStyle::Informational,
        Level::Warning => NSAlertStyle::Warning,
        Level::Error => NSAlertStyle::Critical,
    });
    alert.setMessageText(&ns(title));
    alert.setInformativeText(&ns(text));
    alert.addButtonWithTitle(&ns("OK"));
    alert.runModal();
}

/// Chiede conferma con un pulsante che descrive l'azione e il pulsante «Annulla».
///
/// Con `safe_default` il pulsante predefinito (Invio) è «Annulla»: per le azioni che fanno
/// perdere dati; altrimenti è l'azione ed Esc annulla.
pub fn confirm(
    title: &str,
    text: &str,
    action: &str,
    safe_default: bool,
    mtm: MainThreadMarker,
) -> bool {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(if safe_default {
        NSAlertStyle::Warning
    } else {
        NSAlertStyle::Informational
    });
    alert.setMessageText(&ns(title));
    alert.setInformativeText(&ns(text));
    if safe_default {
        alert.addButtonWithTitle(&ns("Annulla"));
        alert.addButtonWithTitle(&ns(action));
        alert.runModal() != NSAlertFirstButtonReturn
    } else {
        alert.addButtonWithTitle(&ns(action));
        let cancel = alert.addButtonWithTitle(&ns("Annulla"));
        cancel.setKeyEquivalent(&ns("\u{1b}"));
        alert.runModal() == NSAlertFirstButtonReturn
    }
}

/// Domanda con più risposte: il primo pulsante è quello predefinito (Invio), l'ultimo si
/// attiva anche con Esc. Restituisce l'indice del pulsante scelto.
pub fn choose(title: &str, text: &str, buttons: &[&str], mtm: MainThreadMarker) -> usize {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Informational);
    alert.setMessageText(&ns(title));
    alert.setInformativeText(&ns(text));
    for (index, title) in buttons.iter().enumerate() {
        let button = alert.addButtonWithTitle(&ns(title));
        if index > 0 && index + 1 == buttons.len() {
            button.setKeyEquivalent(&ns("\u{1b}"));
        }
    }
    let response = alert.runModal() - NSAlertFirstButtonReturn;
    usize::try_from(response).unwrap_or(buttons.len() - 1).min(buttons.len() - 1)
}

pub fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&ns(&path.to_string_lossy()))
}

pub fn url_path(url: &NSURL) -> Option<PathBuf> {
    url.path().map(|p| PathBuf::from(p.to_string()))
}

/// Tipi di file per le estensioni indicate (quelle sconosciute al sistema vengono saltate).
pub fn content_types(extensions: &[&str]) -> Retained<NSArray<UTType>> {
    let types: Vec<Retained<UTType>> =
        extensions.iter().filter_map(|e| UTType::typeWithFilenameExtension(&ns(e))).collect();
    NSArray::from_retained_slice(&types)
}

/// Pannello di apertura di un singolo file.
pub fn open_file(
    title: &str,
    message: &str,
    extensions: &[&str],
    folder: Option<&Path>,
    mtm: MainThreadMarker,
) -> Option<PathBuf> {
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setTitle(Some(&ns(title)));
    panel.setMessage(Some(&ns(message)));
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    let types = content_types(extensions);
    if types.count() > 0 {
        panel.setAllowedContentTypes(&types);
    }
    if let Some(folder) = folder.filter(|f| f.is_dir()) {
        panel.setDirectoryURL(Some(&file_url(folder)));
    }
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    panel.URL().and_then(|url| url_path(&url))
}

/// Finestra con un testo di sola lettura (guida, informazioni), con il pulsante «Chiudi»
/// attivabile anche con Esc. Il focus va sul testo, che VoiceOver legge riga per riga.
pub fn text_window(title: &str, text: &str, mtm: MainThreadMarker) -> Retained<NSWindow> {
    let (width, height) = (620.0, 520.0);
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect(0.0, 0.0, width, height),
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setRestorable(false);
    window.setTitle(&ns(title));
    window.setContentMinSize(NSSize::new(360.0, 240.0));

    let content = NSView::new(mtm);
    let close = button("Chiudi", &window, sel!(performClose:), mtm);
    close.setKeyEquivalent(&ns("\u{1b}"));
    let close_size = close.intrinsicContentSize();
    let close_w = close_size.width.max(90.0);
    let margin = 20.0;
    close.setFrame(rect(width - margin - close_w, 14.0, close_w, close_size.height));
    close.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMaxYMargin,
    );

    let (scroll, view) = text_area(title, false, mtm);
    let top = 14.0 + close_size.height + 12.0;
    scroll.setFrame(rect(margin, top, width - 2.0 * margin, height - top - margin));
    scroll.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    view.setString(&ns(text));
    set_text_view_font(&view, 14.0);
    move_caret_to_start(&view);

    content.addSubview(&scroll);
    content.addSubview(&close);
    window.setContentView(Some(&content));
    window.setInitialFirstResponder(Some(&view));
    window.center();
    window.makeKeyAndOrderFront(None);
    window.makeFirstResponder(Some(&view));
    window
}
