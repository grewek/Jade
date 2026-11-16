//! Turns protocol input commands into real GPUI platform events, dispatched
//! through `Window::dispatch_event` so hit-testing, focus and key bindings run
//! exactly as for a user. No repaint is forced: the window redraws itself
//! when the event invalidates it, as it would for real input.

use gpui_kit::{
    KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, PlatformInput, Point, ScrollDelta, ScrollWheelEvent, Window, point, px,
};
use jade_debug_protocol as proto;
use jade_debug_protocol::{Command, Location};

const MAX_CLICK_COUNT: u8 = 3;

pub(super) fn is_input(command: &Command) -> bool {
    matches!(
        command,
        Command::Type { .. }
            | Command::Key { .. }
            | Command::MouseMove { .. }
            | Command::MouseDown { .. }
            | Command::MouseUp { .. }
            | Command::Click { .. }
            | Command::Scroll { .. }
    )
}

pub(super) fn apply(
    command: &Command,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) -> Result<(), String> {
    match command {
        Command::Type { text } => text.chars().for_each(|c| type_char(c, window, cx)),
        Command::Key { key, modifiers } => press_key(key, to_modifiers(*modifiers), window, cx)?,
        Command::MouseMove { at, modifiers } => mouse_move(*at, *modifiers, window, cx)?,
        Command::MouseDown {
            at,
            button,
            modifiers,
        } => mouse_down(*at, *button, *modifiers, 1, window, cx)?,
        Command::MouseUp {
            at,
            button,
            modifiers,
        } => mouse_up(*at, *button, *modifiers, 1, window, cx)?,
        Command::Click {
            at,
            button,
            count,
            modifiers,
        } => click(at, *button, *count, *modifiers, window, cx)?,
        Command::Scroll {
            at,
            dx,
            dy,
            modifiers,
        } => scroll(*at, *dx, *dy, *modifiers, window, cx)?,
        other => return Err(format!("not an input command: {other:?}")),
    }
    Ok(())
}

fn to_modifiers(m: proto::Modifiers) -> Modifiers {
    Modifiers {
        control: m.ctrl,
        alt: m.alt,
        shift: m.shift,
        platform: m.meta,
        function: false,
    }
}

fn to_button(button: proto::Button) -> MouseButton {
    match button {
        proto::Button::Left => MouseButton::Left,
        proto::Button::Right => MouseButton::Right,
        proto::Button::Middle => MouseButton::Middle,
    }
}

fn to_point(at: proto::Point) -> Result<Point<Pixels>, String> {
    if !at.x.is_finite() || !at.y.is_finite() {
        return Err(format!("non-finite coordinates ({}, {})", at.x, at.y));
    }
    Ok(point(px(at.x), px(at.y)))
}

/// A bare key-down and key-up, as the real Linux platform sends them.
fn press_key(
    key: &str,
    modifiers: Modifiers,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) -> Result<(), String> {
    if key.is_empty() || key.len() > 32 {
        return Err(format!("invalid key name {key:?}"));
    }
    let keystroke = Keystroke {
        modifiers,
        key: key.to_owned(),
        key_char: None,
    };
    let down = KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    };
    window.dispatch_event(PlatformInput::KeyDown(down), cx);
    window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke }), cx);
    Ok(())
}

/// Enter and Tab go bare (the editor handles them as keys, and
/// `dispatch_keystroke` would also insert text); everything else is a
/// keystroke carrying its character, like the headless tests do.
fn type_char(c: char, window: &mut Window, cx: &mut gpui_kit::App) {
    match c {
        '\n' => drop(press_key("enter", Modifiers::default(), window, cx)),
        '\t' => drop(press_key("tab", Modifiers::default(), window, cx)),
        _ => {
            let key = if c == ' ' {
                "space".to_owned()
            } else {
                c.to_lowercase().to_string()
            };
            let modifiers = Modifiers {
                shift: c.is_uppercase(),
                ..Default::default()
            };
            let keystroke = Keystroke {
                modifiers,
                key,
                key_char: Some(c.to_string()),
            };
            window.dispatch_keystroke(keystroke, cx);
        }
    }
}

fn mouse_move(
    at: proto::Point,
    modifiers: proto::Modifiers,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) -> Result<(), String> {
    let event = MouseMoveEvent {
        position: to_point(at)?,
        pressed_button: None,
        modifiers: to_modifiers(modifiers),
    };
    window.dispatch_event(PlatformInput::MouseMove(event), cx);
    Ok(())
}

fn mouse_down(
    at: proto::Point,
    button: proto::Button,
    modifiers: proto::Modifiers,
    click_count: usize,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) -> Result<(), String> {
    let event = MouseDownEvent {
        button: to_button(button),
        position: to_point(at)?,
        modifiers: to_modifiers(modifiers),
        click_count,
        first_mouse: false,
    };
    window.dispatch_event(PlatformInput::MouseDown(event), cx);
    Ok(())
}

fn mouse_up(
    at: proto::Point,
    button: proto::Button,
    modifiers: proto::Modifiers,
    click_count: usize,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) -> Result<(), String> {
    let event = MouseUpEvent {
        button: to_button(button),
        position: to_point(at)?,
        modifiers: to_modifiers(modifiers),
        click_count,
    };
    window.dispatch_event(PlatformInput::MouseUp(event), cx);
    Ok(())
}

/// Move, then down/up per click with a rising click count (a double click is
/// `click_count` 1 then 2, as the platform reports it).
fn click(
    at: &Location,
    button: proto::Button,
    count: u8,
    modifiers: proto::Modifiers,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) -> Result<(), String> {
    let Location::Point(at) = at else {
        return Err("clicking a named target is not available yet".into());
    };
    if !(1..=MAX_CLICK_COUNT).contains(&count) {
        return Err(format!(
            "click count must be 1..={MAX_CLICK_COUNT}, got {count}"
        ));
    }
    mouse_move(*at, modifiers, window, cx)?;
    for click_count in 1..=usize::from(count) {
        mouse_down(*at, button, modifiers, click_count, window, cx)?;
        mouse_up(*at, button, modifiers, click_count, window, cx)?;
    }
    Ok(())
}

fn scroll(
    at: proto::Point,
    dx: f32,
    dy: f32,
    modifiers: proto::Modifiers,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) -> Result<(), String> {
    if !dx.is_finite() || !dy.is_finite() {
        return Err(format!("non-finite scroll delta ({dx}, {dy})"));
    }
    let event = ScrollWheelEvent {
        position: to_point(at)?,
        delta: ScrollDelta::Pixels(point(px(dx), px(dy))),
        modifiers: to_modifiers(modifiers),
        ..Default::default()
    };
    window.dispatch_event(PlatformInput::ScrollWheel(event), cx);
    Ok(())
}
