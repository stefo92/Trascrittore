//! Barra dei menu con le scorciatoie da tastiera.
//!
//! Le scorciatoie seguono le convenzioni del Mac (Comando+O, Comando+S, Comando+punto…). Le
//! voci nascoste con i tasti funzione (F1, F2, F5, come su Windows) restano attive anche se
//! non compaiono nel menu.

#![allow(unused_unsafe)]

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};

use super::ui::ns;

const F1: char = '\u{F704}';
const F2: char = '\u{F705}';
const F5: char = '\u{F708}';

struct Builder<'a> {
    mtm: MainThreadMarker,
    target: &'a AnyObject,
}

impl Builder<'_> {
    fn menu(&self, title: &str) -> Retained<NSMenu> {
        NSMenu::initWithTitle(NSMenu::alloc(self.mtm), &ns(title))
    }

    /// Voce con un comando del controller (`own`) o della catena dei responder (es. `copy:`).
    fn item(
        &self,
        menu: &NSMenu,
        title: &str,
        action: Sel,
        key: &str,
        modifiers: NSEventModifierFlags,
        own: bool,
    ) -> Retained<NSMenuItem> {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(self.mtm),
                &ns(title),
                Some(action),
                &ns(key),
            )
        };
        item.setKeyEquivalentModifierMask(modifiers);
        if own {
            unsafe { item.setTarget(Some(self.target)) };
        }
        menu.addItem(&item);
        item
    }

    fn cmd(&self, menu: &NSMenu, title: &str, action: Sel, key: &str) -> Retained<NSMenuItem> {
        self.item(menu, title, action, key, NSEventModifierFlags::Command, true)
    }

    /// Scorciatoia aggiuntiva per un comando, invisibile nel menu.
    fn hidden(&self, menu: &NSMenu, action: Sel, key: char, modifiers: NSEventModifierFlags) {
        let item = self.item(menu, "", action, &key.to_string(), modifiers, true);
        item.setHidden(true);
        item.setAllowsKeyEquivalentWhenHidden(true);
    }

    fn separator(&self, menu: &NSMenu) {
        menu.addItem(&NSMenuItem::separatorItem(self.mtm));
    }

    fn submenu(&self, bar: &NSMenu, menu: &NSMenu) {
        let item = NSMenuItem::new(self.mtm);
        item.setSubmenu(Some(menu));
        bar.addItem(&item);
    }
}

/// Crea la barra dei menu; `target` riceve i comandi propri del programma e ne decide lo
/// stato con `validateMenuItem:`.
pub fn install(app: &NSApplication, target: &AnyObject, mtm: MainThreadMarker) {
    let b = Builder { mtm, target };
    let cmd = NSEventModifierFlags::Command;
    let cmd_shift = NSEventModifierFlags::Command | NSEventModifierFlags::Shift;
    let cmd_option = NSEventModifierFlags::Command | NSEventModifierFlags::Option;
    let none = NSEventModifierFlags::empty();
    let bar = b.menu("");

    // Menu dell'applicazione: il titolo mostrato è il nome del programma.
    let app_menu = b.menu("Trascrittore");
    b.cmd(&app_menu, "Informazioni su Trascrittore", sel!(showAbout:), "");
    b.separator(&app_menu);
    let services = b.menu("Servizi");
    let services_item = NSMenuItem::new(mtm);
    services_item.setTitle(&ns("Servizi"));
    services_item.setSubmenu(Some(&services));
    app_menu.addItem(&services_item);
    app.setServicesMenu(Some(&services));
    b.separator(&app_menu);
    b.item(&app_menu, "Nascondi Trascrittore", sel!(hide:), "h", cmd, false);
    b.item(&app_menu, "Nascondi altre", sel!(hideOtherApplications:), "h", cmd_option, false);
    b.item(&app_menu, "Mostra tutte", sel!(unhideAllApplications:), "", none, false);
    b.separator(&app_menu);
    b.item(&app_menu, "Esci da Trascrittore", sel!(terminate:), "q", cmd, false);
    b.submenu(&bar, &app_menu);

    let file = b.menu("File");
    b.cmd(&file, "Apri file audio…", sel!(chooseAudio:), "o");
    b.cmd(&file, "Esporta…", sel!(export:), "s");
    b.separator(&file);
    b.item(&file, "Chiudi finestra", sel!(performClose:), "w", cmd, false);
    b.submenu(&bar, &file);

    let edit = b.menu("Modifica");
    b.item(&edit, "Annulla", sel!(undo:), "z", cmd, false);
    b.item(&edit, "Ripristina", sel!(redo:), "z", cmd_shift, false);
    b.separator(&edit);
    b.item(&edit, "Taglia", sel!(cut:), "x", cmd, false);
    b.item(&edit, "Copia", sel!(copy:), "c", cmd, false);
    b.item(&edit, "Incolla", sel!(paste:), "v", cmd, false);
    b.item(&edit, "Seleziona tutto", sel!(selectAll:), "a", cmd, false);
    b.separator(&edit);
    b.item(&edit, "Copia tutto il testo trascritto", sel!(copyAll:), "c", cmd_shift, true);
    b.separator(&edit);
    // Tag 1 = NSFindPanelActionShowFindPanel: apre la barra di ricerca del testo.
    let find = b.item(&edit, "Trova…", sel!(performFindPanelAction:), "f", cmd, false);
    find.setTag(1);
    let find_next =
        b.item(&edit, "Trova successivo", sel!(performFindPanelAction:), "g", cmd, false);
    find_next.setTag(2);
    let find_previous =
        b.item(&edit, "Trova precedente", sel!(performFindPanelAction:), "g", cmd_shift, false);
    find_previous.setTag(3);
    b.separator(&edit);
    b.cmd(&edit, "Ingrandisci il testo", sel!(zoomIn:), "+");
    b.hidden(&edit, sel!(zoomIn:), '=', cmd);
    b.cmd(&edit, "Riduci il testo", sel!(zoomOut:), "-");
    b.cmd(&edit, "Dimensione normale del testo", sel!(zoomReset:), "0");
    b.submenu(&bar, &edit);

    let job = b.menu("Trascrizione");
    b.cmd(&job, "Avvia la trascrizione", sel!(startTranscription:), "r");
    b.hidden(&job, sel!(startTranscription:), F5, none);
    b.cmd(&job, "Interrompi", sel!(cancelJob:), ".");
    b.separator(&job);
    b.cmd(&job, "Scarica il modello selezionato", sel!(downloadModel:), "");
    b.cmd(&job, "Usa un file modello personalizzato…", sel!(chooseCustomModel:), "");
    b.cmd(&job, "Mostra la cartella dei modelli nel Finder", sel!(openModelsDir:), "");
    b.separator(&job);
    b.cmd(&job, "Accelerazione Flash Attention", sel!(toggleFlashAttention:), "");
    b.separator(&job);
    b.cmd(&job, "Leggi lo stato", sel!(announceStatus:), "l");
    b.hidden(&job, sel!(announceStatus:), F2, none);
    b.submenu(&bar, &job);

    let go = b.menu("Vai");
    for (index, (title, action)) in [
        ("File audio", sel!(focusAudio:)),
        ("Modello", sel!(focusModel:)),
        ("Lingua", sel!(focusLanguage:)),
        ("Testo trascritto", sel!(focusText:)),
        ("Formato di esportazione", sel!(focusFormat:)),
    ]
    .into_iter()
    .enumerate()
    {
        b.cmd(&go, title, action, &(index + 1).to_string());
    }
    b.submenu(&bar, &go);

    let window = b.menu("Finestra");
    b.item(&window, "Riduci a icona", sel!(performMiniaturize:), "m", cmd, false);
    b.item(&window, "Zoom", sel!(performZoom:), "", none, false);
    b.submenu(&bar, &window);
    app.setWindowsMenu(Some(&window));

    let help = b.menu("Aiuto");
    b.cmd(&help, "Guida rapida di Trascrittore", sel!(showHelp:), "?");
    b.hidden(&help, sel!(showHelp:), F1, none);
    b.submenu(&bar, &help);
    app.setHelpMenu(Some(&help));

    app.setMainMenu(Some(&bar));
}
