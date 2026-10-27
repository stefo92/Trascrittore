//! Piccoli aiuti sopra l'API Win32.

use windows::core::HSTRING;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, MessageBoxW, SendMessageW,
    SetWindowTextW, BM_GETCHECK, BM_SETCHECK, CB_ADDSTRING, CB_ERR, CB_GETCURSEL, CB_RESETCONTENT,
    CB_SETCURSEL, GWL_STYLE, MB_ICONERROR, MB_OK, MESSAGEBOX_RESULT, MESSAGEBOX_STYLE, WS_VISIBLE,
};

/// Titolo delle finestre di messaggio.
pub const APP_TITLE: &str = "Trascrittore";

pub fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))).0 }
}

pub fn set_text(hwnd: HWND, text: &str) {
    unsafe {
        let _ = SetWindowTextW(hwnd, &HSTRING::from(text));
    }
}

pub fn get_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut buffer = vec![0u16; len + 1];
        let copied = GetWindowTextW(hwnd, &mut buffer).max(0) as usize;
        String::from_utf16_lossy(&buffer[..copied])
    }
}

/// `true` se il controllo stesso ha lo stile WS_VISIBLE. A differenza di `IsWindowVisible`
/// non dipende dalla finestra principale, che durante l'avvio non è ancora mostrata.
pub fn is_shown(hwnd: HWND) -> bool {
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    style & WS_VISIBLE.0 != 0
}

pub fn enable(hwnd: HWND, enabled: bool) {
    unsafe {
        let _ = EnableWindow(hwnd, enabled);
    }
}

pub fn is_checked(hwnd: HWND) -> bool {
    send(hwnd, BM_GETCHECK, 0, 0) == BST_CHECKED.0 as isize
}

pub fn set_checked(hwnd: HWND, checked: bool) {
    let state = if checked { BST_CHECKED } else { BST_UNCHECKED };
    send(hwnd, BM_SETCHECK, state.0 as usize, 0);
}

pub fn combo_reset(hwnd: HWND) {
    send(hwnd, CB_RESETCONTENT, 0, 0);
}

pub fn combo_add(hwnd: HWND, text: &str) {
    let wide = HSTRING::from(text);
    send(hwnd, CB_ADDSTRING, 0, wide.as_ptr() as isize);
}

pub fn combo_selected(hwnd: HWND) -> Option<usize> {
    let index = send(hwnd, CB_GETCURSEL, 0, 0);
    (index != CB_ERR as isize && index >= 0).then_some(index as usize)
}

pub fn combo_select(hwnd: HWND, index: usize) {
    send(hwnd, CB_SETCURSEL, index, 0);
}

pub fn message_box(owner: Option<HWND>, text: &str, style: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    unsafe {
        MessageBoxW(
            owner,
            &HSTRING::from(text),
            &HSTRING::from(APP_TITLE),
            style,
        )
    }
}

pub fn error_box(owner: Option<HWND>, text: &str) {
    message_box(owner, text, MB_OK | MB_ICONERROR);
}

/// Converte gli a capo in CRLF, richiesto dalle caselle di testo multiriga.
pub fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// Converte gli a capo CRLF in `\n`.
pub fn from_crlf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// Estrae la parola bassa di un parametro di messaggio.
pub fn loword(value: usize) -> u32 {
    (value & 0xFFFF) as u32
}

/// Estrae la parola alta di un parametro di messaggio.
pub fn hiword(value: usize) -> u32 {
    ((value >> 16) & 0xFFFF) as u32
}
