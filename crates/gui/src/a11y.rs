//! Supporto agli screen reader: nomi accessibili espliciti e annunci vocali.

use windows::core::{BSTR, HSTRING};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Accessibility::{
    CAccPropServices, IAccPropServices, NotificationKind_Other,
    NotificationProcessing_ImportantMostRecent, UiaClientsAreListening, UiaHostProviderFromHwnd,
    UiaRaiseNotificationEvent, PROPID_ACC_DESCRIPTION, PROPID_ACC_NAME,
};
use windows::Win32::UI::WindowsAndMessaging::{CHILDID_SELF, OBJID_CLIENT};

/// Assegna nomi e descrizioni accessibili ai controlli («Direct Annotation» di MSAA/UIA).
pub struct Annotator {
    services: Option<IAccPropServices>,
}

impl Annotator {
    pub fn new() -> Self {
        let services =
            unsafe { CoCreateInstance(&CAccPropServices, None, CLSCTX_INPROC_SERVER).ok() };
        Self { services }
    }

    /// Imposta il nome letto dallo screen reader (senza «&» e senza due punti).
    pub fn set_name(&self, hwnd: HWND, name: &str) {
        if let Some(services) = &self.services {
            unsafe {
                let _ = services.SetHwndPropStr(
                    hwnd,
                    OBJID_CLIENT.0 as u32,
                    CHILDID_SELF,
                    PROPID_ACC_NAME,
                    &HSTRING::from(name),
                );
            }
        }
    }

    /// Imposta una descrizione aggiuntiva, letta dopo nome e ruolo.
    pub fn set_description(&self, hwnd: HWND, description: &str) {
        if let Some(services) = &self.services {
            unsafe {
                let _ = services.SetHwndPropStr(
                    hwnd,
                    OBJID_CLIENT.0 as u32,
                    CHILDID_SELF,
                    PROPID_ACC_DESCRIPTION,
                    &HSTRING::from(description),
                );
            }
        }
    }
}

/// Fa pronunciare un messaggio allo screen reader senza spostare il focus
/// (UI Automation Notification Event, supportato da Assistente vocale, NVDA e JAWS).
pub fn announce(source: HWND, message: &str) {
    unsafe {
        if !UiaClientsAreListening().as_bool() {
            return;
        }
        let Ok(provider) = UiaHostProviderFromHwnd(source) else {
            return;
        };
        let _ = UiaRaiseNotificationEvent(
            &provider,
            NotificationKind_Other,
            NotificationProcessing_ImportantMostRecent,
            &BSTR::from(message),
            &BSTR::from("Trascrittore.Stato"),
        );
    }
}
