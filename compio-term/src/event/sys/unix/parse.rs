// Portions of this parser are adapted from Crossterm 0.29.0.
// Copyright (c) 2019 Timon. Licensed under the MIT license.

use std::{collections::VecDeque, str::FromStr};

use crate::event::{
    Event, InternalEvent, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
    MediaKeyCode, ModifierKeyCode, MouseButton, MouseEvent, MouseEventKind,
};

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

pub(super) struct Parser {
    buffer: Vec<u8>,
    events: VecDeque<InternalEvent>,
    raw_mode: bool,
}

impl Parser {
    pub(super) fn new(raw_mode: bool) -> Self {
        Self {
            buffer: Vec::with_capacity(256),
            events: VecDeque::with_capacity(128),
            raw_mode,
        }
    }

    pub(super) fn advance(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.buffer.push(byte);
            match parse_event(&self.buffer, true, self.raw_mode) {
                Ok(Some(event)) => {
                    self.events.push_back(event);
                    self.buffer.clear();
                }
                Ok(None) => {}
                Err(()) => self.buffer.clear(),
            }
        }
    }

    pub(super) fn resolve_escape(&mut self) {
        match parse_event(&self.buffer, false, self.raw_mode) {
            Ok(Some(event)) => {
                self.events.push_back(event);
                self.buffer.clear();
            }
            Ok(None) => {}
            Err(()) => self.buffer.clear(),
        }
    }

    pub(super) fn has_ambiguous_escape(&self) -> bool {
        self.buffer == b"\x1b"
    }
}

impl Iterator for Parser {
    type Item = InternalEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.events.pop_front()
    }
}

fn parse_event(
    buffer: &[u8],
    input_available: bool,
    raw_mode: bool,
) -> Result<Option<InternalEvent>, ()> {
    let Some(&first) = buffer.first() else {
        return Ok(None);
    };

    match first {
        b'\x1b' if buffer.len() == 1 => {
            if input_available {
                Ok(None)
            } else {
                Ok(Some(key(KeyCode::Esc, KeyModifiers::NONE)))
            }
        }
        b'\x1b' => match buffer[1] {
            b'[' => parse_csi(buffer, raw_mode),
            b'O' => parse_ss3(buffer),
            b'\x1b' => Ok(Some(key(KeyCode::Esc, KeyModifiers::NONE))),
            _ => parse_event(&buffer[1..], input_available, raw_mode).map(|event| {
                event.map(|event| match event {
                    InternalEvent::Event(Event::Key(mut key)) => {
                        key.modifiers |= KeyModifiers::ALT;
                        InternalEvent::Event(Event::Key(key))
                    }
                    event => event,
                })
            }),
        },
        b'\r' => Ok(Some(key(KeyCode::Enter, KeyModifiers::NONE))),
        b'\n' if !raw_mode => Ok(Some(key(KeyCode::Enter, KeyModifiers::NONE))),
        b'\t' => Ok(Some(key(KeyCode::Tab, KeyModifiers::NONE))),
        b'\x7f' => Ok(Some(key(KeyCode::Backspace, KeyModifiers::NONE))),
        b'\0' => Ok(Some(key(KeyCode::Char(' '), KeyModifiers::CONTROL))),
        byte @ b'\x01'..=b'\x1a' => Ok(Some(key(
            KeyCode::Char(char::from(byte - 1 + b'a')),
            KeyModifiers::CONTROL,
        ))),
        byte @ b'\x1c'..=b'\x1f' => Ok(Some(key(
            KeyCode::Char(char::from(byte - b'\x1c' + b'4')),
            KeyModifiers::CONTROL,
        ))),
        _ => parse_utf8(buffer).map(|character| {
            character.map(|character| {
                let modifiers = if character.is_uppercase() {
                    KeyModifiers::SHIFT
                } else {
                    KeyModifiers::NONE
                };
                key(KeyCode::Char(character), modifiers)
            })
        }),
    }
}

fn parse_ss3(buffer: &[u8]) -> Result<Option<InternalEvent>, ()> {
    let Some(&byte) = buffer.get(2) else {
        return Ok(None);
    };
    let code = match byte {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'F' => KeyCode::End,
        b'H' => KeyCode::Home,
        b'P'..=b'S' => KeyCode::F(1 + byte - b'P'),
        _ => return Err(()),
    };
    Ok(Some(key(code, KeyModifiers::NONE)))
}

fn parse_csi(buffer: &[u8], raw_mode: bool) -> Result<Option<InternalEvent>, ()> {
    if buffer.starts_with(PASTE_START) {
        return parse_paste(buffer);
    }
    if buffer.starts_with(b"\x1b[M") {
        return parse_normal_mouse(buffer);
    }
    if buffer.len() == 2 {
        return Ok(None);
    }

    let final_byte = *buffer.last().ok_or(())?;
    if !(0x40..=0x7e).contains(&final_byte) {
        return Ok(None);
    }

    let body = &buffer[2..buffer.len() - 1];
    let event = match (body, final_byte) {
        (b"", b'A') => key(KeyCode::Up, KeyModifiers::NONE),
        (b"", b'B') => key(KeyCode::Down, KeyModifiers::NONE),
        (b"", b'C') => key(KeyCode::Right, KeyModifiers::NONE),
        (b"", b'D') => key(KeyCode::Left, KeyModifiers::NONE),
        (b"", b'F') => key(KeyCode::End, KeyModifiers::NONE),
        (b"", b'H') => key(KeyCode::Home, KeyModifiers::NONE),
        (b"", b'I') => InternalEvent::Event(Event::FocusGained),
        (b"", b'O') => InternalEvent::Event(Event::FocusLost),
        (b"", b'P') => key(KeyCode::F(1), KeyModifiers::NONE),
        (b"", b'Q') => key(KeyCode::F(2), KeyModifiers::NONE),
        (b"", b'S') => key(KeyCode::F(4), KeyModifiers::NONE),
        (b"", b'Z') => key(KeyCode::BackTab, KeyModifiers::SHIFT),
        (body, b'R') if body.contains(&b';') => InternalEvent::Ignored,
        (body, b'A' | b'B' | b'C' | b'D' | b'F' | b'H' | b'P' | b'Q' | b'R' | b'S') => {
            parse_modified_key(body, final_byte)?
        }
        (body, b'M' | b'm') if body.starts_with(b"<") => parse_sgr_mouse(body, final_byte)?,
        (body, b'M') if body.contains(&b';') => parse_rxvt_mouse(body)?,
        (body, b'~') => parse_special_key(body)?,
        (body, b'u') if body.starts_with(b"?") => InternalEvent::Ignored,
        (body, b'u') => parse_kitty_key(body, raw_mode)?,
        (body, b'c') if body.starts_with(b"?") => InternalEvent::Ignored,
        _ => return Err(()),
    };
    Ok(Some(event))
}

fn parse_modified_key(body: &[u8], final_byte: u8) -> Result<InternalEvent, ()> {
    let text = std::str::from_utf8(body).map_err(|_| ())?;
    let mut parameters = text.split(';');
    let _ = parameters.next();
    let (modifiers, kind, state) = parameters
        .next()
        .map(parse_modifier_parameter)
        .transpose()?
        .map(|(mask, kind)| {
            (
                parse_modifiers(mask),
                parse_kind(kind),
                parse_modifier_state(mask),
            )
        })
        .unwrap_or((KeyModifiers::NONE, KeyEventKind::Press, KeyEventState::NONE));
    let code = match final_byte {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'F' => KeyCode::End,
        b'H' => KeyCode::Home,
        b'P' => KeyCode::F(1),
        b'Q' => KeyCode::F(2),
        b'R' => KeyCode::F(3),
        b'S' => KeyCode::F(4),
        _ => return Err(()),
    };
    Ok(InternalEvent::Event(Event::Key(
        KeyEvent::new_with_kind_and_state(code, modifiers, kind, state),
    )))
}

fn parse_special_key(body: &[u8]) -> Result<InternalEvent, ()> {
    let text = std::str::from_utf8(body).map_err(|_| ())?;
    let mut parameters = text.split(';');
    let first = next_parsed::<u8>(&mut parameters)?;
    let (modifiers, kind, state) = parameters
        .next()
        .map(parse_modifier_parameter)
        .transpose()?
        .map(|(mask, kind)| {
            (
                parse_modifiers(mask),
                parse_kind(kind),
                parse_modifier_state(mask),
            )
        })
        .unwrap_or((KeyModifiers::NONE, KeyEventKind::Press, KeyEventState::NONE));
    let code = match first {
        1 | 7 => KeyCode::Home,
        2 => KeyCode::Insert,
        3 => KeyCode::Delete,
        4 | 8 => KeyCode::End,
        5 => KeyCode::PageUp,
        6 => KeyCode::PageDown,
        value @ 11..=15 => KeyCode::F(value - 10),
        value @ 17..=21 => KeyCode::F(value - 11),
        value @ 23..=26 => KeyCode::F(value - 12),
        value @ 28..=29 => KeyCode::F(value - 15),
        value @ 31..=34 => KeyCode::F(value - 17),
        _ => return Err(()),
    };
    Ok(InternalEvent::Event(Event::Key(
        KeyEvent::new_with_kind_and_state(code, modifiers, kind, state),
    )))
}

fn parse_kitty_key(body: &[u8], raw_mode: bool) -> Result<InternalEvent, ()> {
    let text = std::str::from_utf8(body).map_err(|_| ())?;
    let mut parameters = text.split(';');
    let mut codepoints = parameters.next().ok_or(())?.split(':');
    let codepoint = next_parsed::<u32>(&mut codepoints)?;
    let (mask, kind) = parameters
        .next()
        .map(parse_modifier_parameter)
        .transpose()?
        .unwrap_or((1, 1));
    let mut modifiers = parse_modifiers(mask);
    let mut state = parse_modifier_state(mask);
    let mut code = if let Some((code, code_state)) = functional_key(codepoint) {
        state |= code_state;
        code
    } else {
        let character = char::from_u32(codepoint).ok_or(())?;
        match character {
            '\x1b' => KeyCode::Esc,
            '\r' => KeyCode::Enter,
            '\n' if !raw_mode => KeyCode::Enter,
            '\t' if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
            '\t' => KeyCode::Tab,
            '\x7f' => KeyCode::Backspace,
            character => KeyCode::Char(character),
        }
    };

    if let KeyCode::Modifier(modifier) = code {
        let modifier = match modifier {
            ModifierKeyCode::LeftAlt | ModifierKeyCode::RightAlt => KeyModifiers::ALT,
            ModifierKeyCode::LeftControl | ModifierKeyCode::RightControl => KeyModifiers::CONTROL,
            ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift => KeyModifiers::SHIFT,
            ModifierKeyCode::LeftSuper | ModifierKeyCode::RightSuper => KeyModifiers::SUPER,
            ModifierKeyCode::LeftHyper | ModifierKeyCode::RightHyper => KeyModifiers::HYPER,
            ModifierKeyCode::LeftMeta | ModifierKeyCode::RightMeta => KeyModifiers::META,
            _ => KeyModifiers::NONE,
        };
        modifiers |= modifier;
    }

    if modifiers.contains(KeyModifiers::SHIFT)
        && let Some(character) = codepoints
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .and_then(char::from_u32)
    {
        code = KeyCode::Char(character);
        modifiers.remove(KeyModifiers::SHIFT);
    }

    Ok(InternalEvent::Event(Event::Key(
        KeyEvent::new_with_kind_and_state(code, modifiers, parse_kind(kind), state),
    )))
}

fn functional_key(codepoint: u32) -> Option<(KeyCode, KeyEventState)> {
    let keypad = match codepoint {
        57399..=57408 => KeyCode::Char(char::from(b'0' + (codepoint - 57399) as u8)),
        57409 => KeyCode::Char('.'),
        57410 => KeyCode::Char('/'),
        57411 => KeyCode::Char('*'),
        57412 => KeyCode::Char('-'),
        57413 => KeyCode::Char('+'),
        57414 => KeyCode::Enter,
        57415 => KeyCode::Char('='),
        57416 => KeyCode::Char(','),
        57417 => KeyCode::Left,
        57418 => KeyCode::Right,
        57419 => KeyCode::Up,
        57420 => KeyCode::Down,
        57421 => KeyCode::PageUp,
        57422 => KeyCode::PageDown,
        57423 => KeyCode::Home,
        57424 => KeyCode::End,
        57425 => KeyCode::Insert,
        57426 => KeyCode::Delete,
        57427 => KeyCode::KeypadBegin,
        _ => return non_keypad_functional_key(codepoint),
    };
    Some((keypad, KeyEventState::KEYPAD))
}

fn non_keypad_functional_key(codepoint: u32) -> Option<(KeyCode, KeyEventState)> {
    let code = match codepoint {
        57358 => KeyCode::CapsLock,
        57359 => KeyCode::ScrollLock,
        57360 => KeyCode::NumLock,
        57361 => KeyCode::PrintScreen,
        57362 => KeyCode::Pause,
        57363 => KeyCode::Menu,
        57376..=57398 => KeyCode::F((codepoint - 57363) as u8),
        57428 => KeyCode::Media(MediaKeyCode::Play),
        57429 => KeyCode::Media(MediaKeyCode::Pause),
        57430 => KeyCode::Media(MediaKeyCode::PlayPause),
        57431 => KeyCode::Media(MediaKeyCode::Reverse),
        57432 => KeyCode::Media(MediaKeyCode::Stop),
        57433 => KeyCode::Media(MediaKeyCode::FastForward),
        57434 => KeyCode::Media(MediaKeyCode::Rewind),
        57435 => KeyCode::Media(MediaKeyCode::TrackNext),
        57436 => KeyCode::Media(MediaKeyCode::TrackPrevious),
        57437 => KeyCode::Media(MediaKeyCode::Record),
        57438 => KeyCode::Media(MediaKeyCode::LowerVolume),
        57439 => KeyCode::Media(MediaKeyCode::RaiseVolume),
        57440 => KeyCode::Media(MediaKeyCode::MuteVolume),
        57441 => KeyCode::Modifier(ModifierKeyCode::LeftShift),
        57442 => KeyCode::Modifier(ModifierKeyCode::LeftControl),
        57443 => KeyCode::Modifier(ModifierKeyCode::LeftAlt),
        57444 => KeyCode::Modifier(ModifierKeyCode::LeftSuper),
        57445 => KeyCode::Modifier(ModifierKeyCode::LeftHyper),
        57446 => KeyCode::Modifier(ModifierKeyCode::LeftMeta),
        57447 => KeyCode::Modifier(ModifierKeyCode::RightShift),
        57448 => KeyCode::Modifier(ModifierKeyCode::RightControl),
        57449 => KeyCode::Modifier(ModifierKeyCode::RightAlt),
        57450 => KeyCode::Modifier(ModifierKeyCode::RightSuper),
        57451 => KeyCode::Modifier(ModifierKeyCode::RightHyper),
        57452 => KeyCode::Modifier(ModifierKeyCode::RightMeta),
        57453 => KeyCode::Modifier(ModifierKeyCode::IsoLevel3Shift),
        57454 => KeyCode::Modifier(ModifierKeyCode::IsoLevel5Shift),
        _ => return None,
    };
    Some((code, KeyEventState::NONE))
}

fn parse_rxvt_mouse(body: &[u8]) -> Result<InternalEvent, ()> {
    let text = std::str::from_utf8(body).map_err(|_| ())?;
    let mut values = text.split(';');
    let cb = next_parsed::<u8>(&mut values)?.checked_sub(32).ok_or(())?;
    let (kind, modifiers) = parse_cb(cb)?;
    let column = next_parsed::<u16>(&mut values)?.checked_sub(1).ok_or(())?;
    let row = next_parsed::<u16>(&mut values)?.checked_sub(1).ok_or(())?;
    Ok(mouse(kind, column, row, modifiers))
}

fn parse_normal_mouse(buffer: &[u8]) -> Result<Option<InternalEvent>, ()> {
    if buffer.len() < 6 {
        return Ok(None);
    }
    let cb = buffer[3].checked_sub(32).ok_or(())?;
    let (kind, modifiers) = parse_cb(cb)?;
    let column = u16::from(buffer[4].checked_sub(33).ok_or(())?);
    let row = u16::from(buffer[5].checked_sub(33).ok_or(())?);
    Ok(Some(mouse(kind, column, row, modifiers)))
}

fn parse_sgr_mouse(body: &[u8], final_byte: u8) -> Result<InternalEvent, ()> {
    let text = std::str::from_utf8(body.strip_prefix(b"<").ok_or(())?).map_err(|_| ())?;
    let mut values = text.split(';');
    let cb = next_parsed::<u8>(&mut values)?;
    let (mut kind, modifiers) = parse_cb(cb)?;
    let column = next_parsed::<u16>(&mut values)?.checked_sub(1).ok_or(())?;
    let row = next_parsed::<u16>(&mut values)?.checked_sub(1).ok_or(())?;
    if final_byte == b'm'
        && let MouseEventKind::Down(button) = kind
    {
        kind = MouseEventKind::Up(button);
    }
    Ok(mouse(kind, column, row, modifiers))
}

fn parse_cb(cb: u8) -> Result<(MouseEventKind, KeyModifiers), ()> {
    let button = (cb & 0b0000_0011) | ((cb & 0b1100_0000) >> 4);
    let dragging = cb & 0b0010_0000 != 0;
    let kind = match (button, dragging) {
        (0, false) => MouseEventKind::Down(MouseButton::Left),
        (1, false) => MouseEventKind::Down(MouseButton::Middle),
        (2, false) => MouseEventKind::Down(MouseButton::Right),
        (0, true) => MouseEventKind::Drag(MouseButton::Left),
        (1, true) => MouseEventKind::Drag(MouseButton::Middle),
        (2, true) => MouseEventKind::Drag(MouseButton::Right),
        (3, false) => MouseEventKind::Up(MouseButton::Left),
        (3..=5, true) => MouseEventKind::Moved,
        (4, false) => MouseEventKind::ScrollUp,
        (5, false) => MouseEventKind::ScrollDown,
        (6, false) => MouseEventKind::ScrollLeft,
        (7, false) => MouseEventKind::ScrollRight,
        _ => return Err(()),
    };
    let mut modifiers = KeyModifiers::NONE;
    modifiers.set(KeyModifiers::SHIFT, cb & 0b0000_0100 != 0);
    modifiers.set(KeyModifiers::ALT, cb & 0b0000_1000 != 0);
    modifiers.set(KeyModifiers::CONTROL, cb & 0b0001_0000 != 0);
    Ok((kind, modifiers))
}

fn parse_paste(buffer: &[u8]) -> Result<Option<InternalEvent>, ()> {
    if !buffer.ends_with(PASTE_END) {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&buffer[PASTE_START.len()..buffer.len() - PASTE_END.len()])
        .into_owned();
    Ok(Some(InternalEvent::Event(Event::Paste(text))))
}

fn parse_modifier_parameter(parameter: &str) -> Result<(u8, u8), ()> {
    let mut values = parameter.split(':');
    let mask = next_parsed::<u8>(&mut values)?;
    let kind = values
        .next()
        .map(|value| value.parse::<u8>().map_err(|_| ()))
        .transpose()?
        .unwrap_or(1);
    Ok((mask, kind))
}

fn parse_modifiers(mask: u8) -> KeyModifiers {
    let bits = mask.saturating_sub(1);
    let mut modifiers = KeyModifiers::NONE;
    modifiers.set(KeyModifiers::SHIFT, bits & 1 != 0);
    modifiers.set(KeyModifiers::ALT, bits & 2 != 0);
    modifiers.set(KeyModifiers::CONTROL, bits & 4 != 0);
    modifiers.set(KeyModifiers::SUPER, bits & 8 != 0);
    modifiers.set(KeyModifiers::HYPER, bits & 16 != 0);
    modifiers.set(KeyModifiers::META, bits & 32 != 0);
    modifiers
}

fn parse_modifier_state(mask: u8) -> KeyEventState {
    let bits = mask.saturating_sub(1);
    let mut state = KeyEventState::NONE;
    state.set(KeyEventState::CAPS_LOCK, bits & 64 != 0);
    state.set(KeyEventState::NUM_LOCK, bits & 128 != 0);
    state
}

fn parse_kind(kind: u8) -> KeyEventKind {
    match kind {
        2 => KeyEventKind::Repeat,
        3 => KeyEventKind::Release,
        _ => KeyEventKind::Press,
    }
}

fn parse_utf8(buffer: &[u8]) -> Result<Option<char>, ()> {
    match std::str::from_utf8(buffer) {
        Ok(text) => Ok(text.chars().next()),
        Err(_) => {
            let width = match buffer[0] {
                0x00..=0x7f => 1,
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => return Err(()),
            };
            if buffer[1..].iter().any(|byte| byte & 0xc0 != 0x80) {
                return Err(());
            }
            if buffer.len() < width {
                Ok(None)
            } else {
                Err(())
            }
        }
    }
}

fn next_parsed<'a, T: FromStr>(values: &mut impl Iterator<Item = &'a str>) -> Result<T, ()> {
    values.next().ok_or(())?.parse().map_err(|_| ())
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> InternalEvent {
    InternalEvent::Event(Event::Key(KeyEvent::new(code, modifiers)))
}

fn mouse(kind: MouseEventKind, column: u16, row: u16, modifiers: KeyModifiers) -> InternalEvent {
    InternalEvent::Event(Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(bytes: &[u8]) -> Vec<Event> {
        let mut parser = Parser::new(true);
        parser.advance(bytes);
        parser.resolve_escape();
        parser
            .filter_map(|event| match event {
                InternalEvent::Event(event) => Some(event),
                InternalEvent::Ignored => None,
            })
            .collect()
    }

    #[test]
    fn parses_plain_utf8_and_control_keys() {
        assert_eq!(
            events("aé\u{3}".as_bytes()),
            vec![
                Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
                Event::Key(KeyEvent::new(KeyCode::Char('é'), KeyModifiers::NONE)),
                Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL,)),
            ]
        );
    }

    #[test]
    fn resolves_escape_and_alt_keys() {
        assert_eq!(
            events(b"\x1b"),
            vec![Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,))]
        );
        assert_eq!(
            events(b"\x1bx"),
            vec![Event::Key(KeyEvent::new(
                KeyCode::Char('x'),
                KeyModifiers::ALT,
            ))]
        );
    }

    #[test]
    fn parses_navigation_modifiers_and_kind() {
        assert_eq!(
            events(b"\x1b[A\x1b[1;6:2C"),
            vec![
                Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Right,
                    KeyModifiers::SHIFT | KeyModifiers::CONTROL,
                    KeyEventKind::Repeat,
                )),
            ]
        );
    }

    #[test]
    fn parses_bracketed_paste_as_one_event() {
        assert_eq!(
            events(b"\x1b[200~hello\nworld\x1b[201~"),
            vec![Event::Paste("hello\nworld".into())]
        );
    }

    #[test]
    fn parses_sgr_mouse_coordinates() {
        assert_eq!(
            events(b"\x1b[<0;4;3M"),
            vec![Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 3,
                row: 2,
                modifiers: KeyModifiers::NONE,
            })]
        );
    }

    #[test]
    fn parses_kitty_release_event() {
        assert_eq!(
            events(b"\x1b[97;5:3u"),
            vec![Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('a'),
                KeyModifiers::CONTROL,
                KeyEventKind::Release,
            ))]
        );
    }
}
