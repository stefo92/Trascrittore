//! Verifica automatica dell'interfaccia così come la vede VoiceOver.
//!
//! Con `trascrittore --verifica-accessibilita` il programma apre la finestra, legge l'albero
//! di accessibilità (gli stessi ruoli, nomi e valori che AppKit fornisce a VoiceOver), controlla
//! che ogni controllo abbia un nome, che le etichette non vengano lette due volte e che l'ordine
//! di tabulazione segua quello di lettura, poi termina con codice 0 se non ci sono problemi.
//! Con un file audio esegue anche una trascrizione dalla finestra e controlla che al termine il
//! focus passi al testo. Le finestre di messaggio vengono stampate e le conferme accettate.

#![allow(unused_unsafe)]

use std::io::Write;
use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{msg_send, sel, Message};
use objc2_app_kit::{
    NSBitmapImageFileType, NSControl, NSGraphicsContext, NSTextView, NSView, NSWindow,
};
use objc2_foundation::{NSArray, NSDictionary, NSString};

use super::app::Ui;

/// Opzioni della verifica.
#[derive(Debug, Default)]
pub struct CheckOptions {
    /// File PNG in cui salvare l'aspetto della finestra.
    pub snapshot: Option<PathBuf>,
    /// Modello da usare per la trascrizione di verifica (es. `tiny-q5_1`).
    pub model: Option<String>,
}

/// Ruoli dei controlli con cui si interagisce: devono avere un nome.
const INTERACTIVE_ROLES: &[&str] = &[
    "AXButton",
    "AXCheckBox",
    "AXPopUpButton",
    "AXTextField",
    "AXTextArea",
    "AXProgressIndicator",
];

/// Ruoli per cui VoiceOver annuncia «disattivato» (abbassato).
const TOGGLED_ROLES: &[&str] = &["AXButton", "AXCheckBox", "AXPopUpButton"];

/// Pulsanti della barra del titolo, forniti e nominati da macOS.
const WINDOW_BUTTONS: &[&str] =
    &["AXCloseButton", "AXMinimizeButton", "AXFullScreenButton", "AXZoomButton"];

/// Controlli attesi (ruolo e nome), nell'ordine di lettura.
const EXPECTED: &[(&str, &str)] = &[
    ("AXTextField", "File audio"),
    ("AXButton", "Scegli…"),
    ("AXPopUpButton", "Modello"),
    ("AXPopUpButton", "Lingua"),
    ("AXCheckBox", "Usa la scheda video (GPU)"),
    ("AXPopUpButton", "Scheda video"),
    (NEURAL_ENGINE.0, NEURAL_ENGINE.1),
    ("AXCheckBox", "Traduci in inglese"),
    ("AXCheckBox", "Includi timestamp"),
    ("AXCheckBox", "Modalità rapida (meno precisa)"),
    ("AXButton", "Avvia trascrizione"),
    ("AXButton", "Interrompi"),
    ("AXProgressIndicator", "Avanzamento"),
    ("AXTextArea", "Testo trascritto"),
    ("AXPopUpButton", "Formato di esportazione"),
    ("AXButton", "Esporta…"),
];

/// Casella del Neural Engine, presente solo nelle versioni con Core ML.
const NEURAL_ENGINE: (&str, &str) = ("AXCheckBox", "Usa il Neural Engine (Core ML)");

/// Controlli attesi in questa versione del programma.
fn expected() -> Vec<(&'static str, &'static str)> {
    EXPECTED
        .iter()
        .copied()
        .filter(|c| *c != NEURAL_ENGINE || trascrittore_core::coreml::compiled())
        .collect()
}

/// Elemento dell'albero di accessibilità.
struct Element {
    depth: usize,
    class: String,
    role: String,
    subrole: String,
    role_description: String,
    name: String,
    value: String,
    help: String,
    enabled: bool,
}

fn text(value: Option<Retained<NSString>>) -> String {
    value.map(|s| s.to_string()).unwrap_or_default()
}

fn responds(object: &AnyObject, selector: objc2::runtime::Sel) -> bool {
    unsafe { msg_send![object, respondsToSelector: selector] }
}

fn describe(object: &AnyObject, depth: usize) -> Element {
    unsafe {
        let role: Option<Retained<NSString>> = msg_send![object, accessibilityRole];
        let subrole: Option<Retained<NSString>> = msg_send![object, accessibilitySubrole];
        let role_description: Option<Retained<NSString>> =
            msg_send![object, accessibilityRoleDescription];
        let label: Option<Retained<NSString>> = msg_send![object, accessibilityLabel];
        let title: Option<Retained<NSString>> = msg_send![object, accessibilityTitle];
        let help: Option<Retained<NSString>> = msg_send![object, accessibilityHelp];
        let value: Option<Retained<AnyObject>> = msg_send![object, accessibilityValue];
        let enabled: bool = msg_send![object, isAccessibilityEnabled];
        let value = value
            .map(|v| {
                let description: Retained<NSString> = msg_send![&*v, description];
                description.to_string()
            })
            .unwrap_or_default();
        // VoiceOver legge l'etichetta se c'è, altrimenti il titolo.
        let name = Some(text(label)).filter(|l| !l.is_empty()).unwrap_or_else(|| text(title));
        Element {
            depth,
            class: object.class().name().to_string_lossy().into_owned(),
            role: text(role),
            subrole: text(subrole),
            role_description: text(role_description),
            name,
            value,
            help: text(help),
            enabled,
        }
    }
}

fn collect(object: &AnyObject, depth: usize, out: &mut Vec<Element>) {
    if !responds(object, sel!(accessibilityRole)) {
        return;
    }
    out.push(describe(object, depth));
    let children: Option<Retained<NSArray<AnyObject>>> =
        unsafe { msg_send![object, accessibilityChildren] };
    for child in children.map(|c| c.to_vec()).unwrap_or_default() {
        collect(&child, depth + 1, out);
    }
}

fn shorten(text: &str, max: usize) -> String {
    let text = text.replace('\n', " ");
    if text.chars().count() <= max {
        text
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}

/// Stampa l'albero e l'ordine di tabulazione; restituisce i problemi trovati.
pub(super) fn verify(ui: &Ui) -> Vec<String> {
    let mut problems = Vec::new();
    let mut elements = Vec::new();
    collect(&ui.window, 0, &mut elements);

    println!("=== Albero di accessibilità della finestra principale");
    for e in &elements {
        let mut line =
            format!("{}{} ({}) [{}]", "  ".repeat(e.depth), e.role, e.role_description, e.class);
        if !e.name.is_empty() {
            line.push_str(&format!(" «{}»", e.name));
        }
        if !e.value.is_empty() {
            line.push_str(&format!(" valore «{}»", shorten(&e.value, 70)));
        }
        if !e.enabled && TOGGLED_ROLES.contains(&e.role.as_str()) {
            line.push_str(" [disattivato]");
        }
        if !e.help.is_empty() {
            line.push_str(&format!(" aiuto «{}»", shorten(&e.help, 50)));
        }
        println!("{line}");
    }

    // Ogni controllo ha un nome (i pulsanti della barra del titolo sono di AppKit).
    for e in &elements {
        if INTERACTIVE_ROLES.contains(&e.role.as_str())
            && e.name.is_empty()
            && !WINDOW_BUTTONS.contains(&e.subrole.as_str())
        {
            problems.push(format!("controllo senza nome: {} ({})", e.role, e.role_description));
        }
    }
    // Ogni controllo atteso è presente, nell'ordine di lettura.
    let mut last = None;
    for (role, name) in expected() {
        match elements.iter().position(|e| e.role == role && e.name == name) {
            None => problems.push(format!("manca il controllo {role} «{name}»")),
            Some(position) => {
                if last.is_some_and(|l| position < l) {
                    problems.push(format!("«{name}» è fuori dall'ordine di lettura"));
                }
                last = Some(position);
            }
        }
    }
    // Le etichette visibili non vengono lette come testo a sé: il nome è sul controllo.
    for label in ["File audio:", "Modello:", "Lingua:", "Scheda video:", "Formato di esportazione:"]
    {
        if elements
            .iter()
            .any(|e| e.role == "AXStaticText" && (e.value == label || e.name == label))
        {
            problems.push(format!("l'etichetta «{label}» viene letta due volte"));
        }
    }
    // Lo stato si legge subito dopo la sua etichetta, come testo.
    let status = elements
        .iter()
        .position(|e| e.role == "AXStaticText" && e.value == "Stato:")
        .and_then(|i| elements.get(i + 1));
    if !status.is_some_and(|e| e.role == "AXStaticText" && !e.value.is_empty()) {
        problems.push("il testo di stato non è leggibile dopo «Stato:»".to_string());
    }

    println!("=== Ordine di tabulazione");
    ui.window.recalculateKeyViewLoop();
    let first: &NSView = &ui.audio_path;
    let first = first.retain();
    let mut order = Vec::new();
    let mut view = Some(first.clone());
    let mut steps = 0;
    while let Some(current) = view {
        // Per i controlli VoiceOver legge la cella (vedi `ui::set_name`).
        let cell = current.downcast_ref::<NSControl>().and_then(|c| c.cell());
        let element = match &cell {
            Some(cell) => describe(cell, 0),
            None => describe(&current, 0),
        };
        // Le etichette fanno parte della catena ma non ricevono mai il focus.
        if !element.name.is_empty() && order.last() != Some(&element.name) {
            order.push(element.name.clone());
            println!("{}. {} «{}»", order.len(), element.role, element.name);
        }
        view = unsafe { current.nextKeyView() };
        steps += 1;
        if steps > 200
            || view.as_ref().is_some_and(|v| Retained::as_ptr(v) == Retained::as_ptr(&first))
        {
            break;
        }
    }
    let expected_order: Vec<&str> = expected()
        .into_iter()
        .filter(|(role, _)| *role != "AXProgressIndicator")
        .map(|(_, name)| name)
        .collect();
    let actual: Vec<&str> =
        order.iter().map(String::as_str).filter(|name| expected_order.contains(name)).collect();
    if actual != expected_order {
        problems.push(format!("ordine di tabulazione inatteso: {}", actual.join(", ")));
    }
    problems
}

/// Controlla una finestra di testo (guida, informazioni): il focus è sul testo, che ha come
/// nome il titolo della finestra, e il pulsante «Chiudi» si attiva con Esc.
pub fn verify_text_window(window: &NSWindow, title: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let text = window.firstResponder().and_then(|r| r.downcast::<NSTextView>().ok());
    match text {
        Some(text) => {
            let element = describe(&text, 0);
            let length = text.string().length();
            println!(
                "Finestra «{title}»: focus su {} «{}», {length} caratteri",
                element.role, element.name
            );
            if element.name != title {
                problems.push(format!("il testo della finestra «{title}» non ha il nome giusto"));
            }
            if length == 0 {
                problems.push(format!("la finestra «{title}» è vuota"));
            }
        }
        None => problems.push(format!("nella finestra «{title}» il focus non è sul testo")),
    }
    let mut elements = Vec::new();
    collect(window, 0, &mut elements);
    if !elements.iter().any(|e| e.role == "AXButton" && e.name == "Chiudi") {
        problems.push(format!("nella finestra «{title}» manca il pulsante Chiudi"));
    }
    problems
}

/// Salva l'aspetto della finestra (barra del titolo compresa) in un file PNG.
///
/// Si disegna l'albero dei livelli di Core Animation: `cacheDisplayInRect:` non riproduce i
/// controlli moderni, e la cattura dall'esterno (`screencapture`) richiede il permesso di
/// registrazione dello schermo.
pub fn snapshot(window: &NSWindow, path: &Path) -> Result<(), String> {
    let content = window.contentView().ok_or("finestra senza contenuto")?;
    let frame = unsafe { content.superview() }.unwrap_or(content);
    frame.displayIfNeeded();
    let bounds = frame.bounds();
    let rep = frame
        .bitmapImageRepForCachingDisplayInRect(bounds)
        .ok_or("impossibile creare l'immagine")?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)
        .ok_or("impossibile disegnare nell'immagine")?;
    let layer = frame.layer().ok_or("la finestra non usa i livelli di Core Animation")?;
    let cg = context.CGContext();
    unsafe {
        let _: () = msg_send![&*layer, renderInContext: &*cg];
    }
    context.flushGraphics();
    let properties = NSDictionary::new();
    let data =
        unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties) }
            .ok_or("impossibile codificare il PNG")?;
    std::fs::write(path, data.to_vec()).map_err(|e| e.to_string())
}

/// Stampa l'esito; restituisce il codice di uscita (0 senza problemi).
pub fn report(problems: &[String]) -> i32 {
    println!();
    if problems.is_empty() {
        println!("Nessun problema di accessibilità trovato.");
    } else {
        for problem in problems {
            println!("PROBLEMA: {problem}");
        }
        println!("{} problemi trovati.", problems.len());
    }
    let _ = std::io::stdout().flush();
    i32::from(!problems.is_empty())
}
