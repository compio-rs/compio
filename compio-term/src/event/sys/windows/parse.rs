// Portions of this parser are adapted from Crossterm 0.29.0.
// Copyright (c) 2019 Timon. Licensed under the MIT license.

use std::{char, io};

use windows_sys::Win32::{
    Foundation::{INVALID_HANDLE_VALUE, TRUE},
    System::Console::{
        CAPSLOCK_ON, CONSOLE_SCREEN_BUFFER_INFO, DOUBLE_CLICK, FOCUS_EVENT,
        FROM_LEFT_1ST_BUTTON_PRESSED, FROM_LEFT_2ND_BUTTON_PRESSED, GetConsoleScreenBufferInfo,
        GetStdHandle, INPUT_RECORD, KEY_EVENT, KEY_EVENT_RECORD, LEFT_ALT_PRESSED,
        LEFT_CTRL_PRESSED, MOUSE_EVENT, MOUSE_EVENT_RECORD, MOUSE_HWHEELED, MOUSE_MOVED,
        MOUSE_WHEELED, NUMLOCK_ON, RIGHT_ALT_PRESSED, RIGHT_CTRL_PRESSED, RIGHTMOST_BUTTON_PRESSED,
        SHIFT_PRESSED, STD_OUTPUT_HANDLE, WINDOW_BUFFER_SIZE_EVENT,
    },
    UI::{
        Input::KeyboardAndMouse::{
            GetKeyboardLayout, ToUnicodeEx, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END,
            VK_ESCAPE, VK_F1, VK_F24, VK_HOME, VK_INSERT, VK_LEFT, VK_MENU, VK_NEXT, VK_NUMPAD0,
            VK_NUMPAD9, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP,
        },
        WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
    },
};

use crate::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

#[derive(Default)]
pub(super) struct Parser {
    surrogate: Option<u16>,
    buttons: MouseButtons,
}

impl Parser {
    pub(super) fn parse(&mut self, record: INPUT_RECORD) -> io::Result<Option<Event>> {
        let event = match u32::from(record.EventType) {
            KEY_EVENT => {
                // The event tag determines the active INPUT_RECORD union field.
                let record = unsafe { record.Event.KeyEvent };
                self.parse_key(record)
            }
            MOUSE_EVENT => {
                // The event tag determines the active INPUT_RECORD union field.
                let record = unsafe { record.Event.MouseEvent };
                let event = parse_mouse(record, self.buttons)?;
                self.buttons = MouseButtons::from_state(record.dwButtonState);
                event.map(Event::Mouse)
            }
            WINDOW_BUFFER_SIZE_EVENT => {
                // The event tag determines the active INPUT_RECORD union field.
                let size = unsafe { record.Event.WindowBufferSizeEvent }.dwSize;
                Some(Event::Resize(size.X.max(0) as u16, size.Y.max(0) as u16))
            }
            FOCUS_EVENT => {
                // The event tag determines the active INPUT_RECORD union field.
                let focus = unsafe { record.Event.FocusEvent };
                Some(if focus.bSetFocus == TRUE {
                    Event::FocusGained
                } else {
                    Event::FocusLost
                })
            }
            _ => None,
        };
        Ok(event)
    }

    fn parse_key(&mut self, record: KEY_EVENT_RECORD) -> Option<Event> {
        let event = parse_key(record)?;
        match event {
            WindowsKeyEvent::Key(event) => {
                self.surrogate = None;
                Some(Event::Key(event))
            }
            WindowsKeyEvent::Surrogate(surrogate) => {
                let first = self.surrogate.replace(surrogate)?;
                self.surrogate = None;
                let character = char::decode_utf16([first, surrogate]).next()?.ok()?;
                Some(Event::Key(KeyEvent::new(
                    KeyCode::Char(character),
                    modifiers(record.dwControlKeyState),
                )))
            }
        }
    }
}

enum WindowsKeyEvent {
    Key(KeyEvent),
    Surrogate(u16),
}

fn parse_key(record: KEY_EVENT_RECORD) -> Option<WindowsKeyEvent> {
    let modifiers = modifiers(record.dwControlKeyState);
    let state = key_state(record.dwControlKeyState);
    let virtual_key = record.wVirtualKeyCode;
    // The KEY_EVENT_RECORD layout makes UnicodeChar the active union field for W
    // APIs.
    let utf16 = unsafe { record.uChar.UnicodeChar };

    let alt_code = virtual_key == VK_MENU && record.bKeyDown == 0 && utf16 != 0;
    if alt_code {
        return unicode_key(utf16, modifiers, state, KeyEventKind::Release);
    }

    let numpad = (VK_NUMPAD0..=VK_NUMPAD9).contains(&virtual_key);
    let only_alt = modifiers.contains(KeyModifiers::ALT)
        && !modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL);
    if numpad && only_alt {
        return None;
    }

    let code = match virtual_key {
        VK_SHIFT | VK_CONTROL | VK_MENU => return None,
        VK_BACK => Some(KeyCode::Backspace),
        VK_ESCAPE => Some(KeyCode::Esc),
        VK_RETURN => Some(KeyCode::Enter),
        VK_F1..=VK_F24 => Some(KeyCode::F((virtual_key - VK_F1 + 1) as u8)),
        VK_LEFT => Some(KeyCode::Left),
        VK_UP => Some(KeyCode::Up),
        VK_RIGHT => Some(KeyCode::Right),
        VK_DOWN => Some(KeyCode::Down),
        VK_PRIOR => Some(KeyCode::PageUp),
        VK_NEXT => Some(KeyCode::PageDown),
        VK_HOME => Some(KeyCode::Home),
        VK_END => Some(KeyCode::End),
        VK_DELETE => Some(KeyCode::Delete),
        VK_INSERT => Some(KeyCode::Insert),
        VK_TAB if modifiers.contains(KeyModifiers::SHIFT) => Some(KeyCode::BackTab),
        VK_TAB => Some(KeyCode::Tab),
        _ if utf16 <= 0x1f => character_for_key(record).map(KeyCode::Char),
        _ if (0xd800..=0xdfff).contains(&utf16) => {
            return Some(WindowsKeyEvent::Surrogate(utf16));
        }
        _ => char::from_u32(u32::from(utf16)).map(KeyCode::Char),
    }?;
    let kind = if record.bKeyDown == 0 {
        KeyEventKind::Release
    } else {
        KeyEventKind::Press
    };
    Some(WindowsKeyEvent::Key(KeyEvent::new_with_kind_and_state(
        code, modifiers, kind, state,
    )))
}

fn unicode_key(
    utf16: u16,
    modifiers: KeyModifiers,
    state: KeyEventState,
    kind: KeyEventKind,
) -> Option<WindowsKeyEvent> {
    if (0xd800..=0xdfff).contains(&utf16) {
        return Some(WindowsKeyEvent::Surrogate(utf16));
    }
    let character = char::from_u32(u32::from(utf16))?;
    Some(WindowsKeyEvent::Key(KeyEvent::new_with_kind_and_state(
        KeyCode::Char(character),
        modifiers,
        kind,
        state,
    )))
}

fn character_for_key(record: KEY_EVENT_RECORD) -> Option<char> {
    let keyboard_state = [0_u8; 256];
    let mut utf16 = [0_u16; 16];
    let layout = unsafe {
        let window = GetForegroundWindow();
        let thread = GetWindowThreadProcessId(window, std::ptr::null_mut());
        GetKeyboardLayout(thread)
    };
    let count = unsafe {
        ToUnicodeEx(
            u32::from(record.wVirtualKeyCode),
            u32::from(record.wVirtualScanCode),
            keyboard_state.as_ptr(),
            utf16.as_mut_ptr(),
            utf16.len() as i32,
            4,
            layout,
        )
    };
    if count < 1 {
        return None;
    }
    let mut characters = char::decode_utf16(utf16.into_iter().take(count as usize));
    let mut character = characters.next()?.ok()?;
    if characters.next().is_some() {
        return None;
    }

    let uppercase = (record.dwControlKeyState & SHIFT_PRESSED != 0)
        ^ (record.dwControlKeyState & CAPSLOCK_ON != 0);
    if uppercase && character.is_lowercase() {
        let mut converted = character.to_uppercase();
        let first = converted.next()?;
        if converted.next().is_none() {
            character = first;
        }
    } else if !uppercase && character.is_uppercase() {
        let mut converted = character.to_lowercase();
        let first = converted.next()?;
        if converted.next().is_none() {
            character = first;
        }
    }
    Some(character)
}

fn modifiers(state: u32) -> KeyModifiers {
    let mut modifiers = KeyModifiers::NONE;
    modifiers.set(KeyModifiers::SHIFT, state & SHIFT_PRESSED != 0);
    modifiers.set(
        KeyModifiers::CONTROL,
        state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0,
    );
    modifiers.set(
        KeyModifiers::ALT,
        state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0,
    );
    modifiers
}

fn key_state(state: u32) -> KeyEventState {
    let mut event_state = KeyEventState::NONE;
    event_state.set(KeyEventState::CAPS_LOCK, state & CAPSLOCK_ON != 0);
    event_state.set(KeyEventState::NUM_LOCK, state & NUMLOCK_ON != 0);
    event_state
}

#[derive(Clone, Copy, Default)]
struct MouseButtons {
    left: bool,
    right: bool,
    middle: bool,
}

impl MouseButtons {
    fn from_state(state: u32) -> Self {
        Self {
            left: state & FROM_LEFT_1ST_BUTTON_PRESSED != 0,
            right: state & RIGHTMOST_BUTTON_PRESSED != 0,
            middle: state & FROM_LEFT_2ND_BUTTON_PRESSED != 0,
        }
    }
}

fn parse_mouse(
    record: MOUSE_EVENT_RECORD,
    pressed: MouseButtons,
) -> io::Result<Option<MouseEvent>> {
    let state = record.dwButtonState;
    let kind = match record.dwEventFlags {
        0 | DOUBLE_CLICK => {
            if state & FROM_LEFT_1ST_BUTTON_PRESSED != 0 && !pressed.left {
                Some(MouseEventKind::Down(MouseButton::Left))
            } else if state & FROM_LEFT_1ST_BUTTON_PRESSED == 0 && pressed.left {
                Some(MouseEventKind::Up(MouseButton::Left))
            } else if state & RIGHTMOST_BUTTON_PRESSED != 0 && !pressed.right {
                Some(MouseEventKind::Down(MouseButton::Right))
            } else if state & RIGHTMOST_BUTTON_PRESSED == 0 && pressed.right {
                Some(MouseEventKind::Up(MouseButton::Right))
            } else if state & FROM_LEFT_2ND_BUTTON_PRESSED != 0 && !pressed.middle {
                Some(MouseEventKind::Down(MouseButton::Middle))
            } else if state & FROM_LEFT_2ND_BUTTON_PRESSED == 0 && pressed.middle {
                Some(MouseEventKind::Up(MouseButton::Middle))
            } else {
                None
            }
        }
        MOUSE_MOVED => {
            let button = if state & RIGHTMOST_BUTTON_PRESSED != 0 {
                MouseButton::Right
            } else if state & FROM_LEFT_2ND_BUTTON_PRESSED != 0 {
                MouseButton::Middle
            } else {
                MouseButton::Left
            };
            if state
                & (FROM_LEFT_1ST_BUTTON_PRESSED
                    | RIGHTMOST_BUTTON_PRESSED
                    | FROM_LEFT_2ND_BUTTON_PRESSED)
                == 0
            {
                Some(MouseEventKind::Moved)
            } else {
                Some(MouseEventKind::Drag(button))
            }
        }
        MOUSE_WHEELED => match wheel_delta(state).cmp(&0) {
            std::cmp::Ordering::Less => Some(MouseEventKind::ScrollDown),
            std::cmp::Ordering::Greater => Some(MouseEventKind::ScrollUp),
            std::cmp::Ordering::Equal => None,
        },
        MOUSE_HWHEELED => match wheel_delta(state).cmp(&0) {
            std::cmp::Ordering::Less => Some(MouseEventKind::ScrollLeft),
            std::cmp::Ordering::Greater => Some(MouseEventKind::ScrollRight),
            std::cmp::Ordering::Equal => None,
        },
        _ => None,
    };
    let Some(kind) = kind else {
        return Ok(None);
    };

    let column = record.dwMousePosition.X.max(0) as u16;
    let row = relative_row(record.dwMousePosition.Y)?;
    Ok(Some(MouseEvent {
        kind,
        column,
        row,
        modifiers: modifiers(record.dwControlKeyState),
    }))
}

fn relative_row(row: i16) -> io::Result<u16> {
    let output = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if output.is_null() || output == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
    if unsafe { GetConsoleScreenBufferInfo(output, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(row.saturating_sub(info.srWindow.Top).max(0) as u16)
}

fn wheel_delta(state: u32) -> i16 {
    (state >> 16) as i16
}
