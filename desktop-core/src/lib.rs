use std::{
    thread,
    time::{Duration, Instant},
};

use enigo::{
    Button,
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Mouse, Settings,
};
use serde::{Deserialize, Serialize};
use xcap::{image::RgbaImage, Monitor};

pub type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub fn monitor_for_region(region: Region) -> Result<Monitor> {
    let monitor = Monitor::from_point(region.x, region.y).map_err(|e| e.to_string())?;
    let x = monitor.x().map_err(|e| e.to_string())?;
    let y = monitor.y().map_err(|e| e.to_string())?;
    let width = monitor.width().map_err(|e| e.to_string())?;
    let height = monitor.height().map_err(|e| e.to_string())?;
    if region.width < 8
        || region.height < 8
        || region.x < x
        || region.y < y
        || region.x as i64 + region.width as i64 > x as i64 + width as i64
        || region.y as i64 + region.height as i64 > y as i64 + height as i64
    {
        return Err("Region must be at least 8x8 and entirely inside one monitor".into());
    }
    Ok(monitor)
}

pub fn capture_region(region: Region) -> Result<RgbaImage> {
    let monitor = monitor_for_region(region)?;
    monitor
        .capture_region(
            (region.x - monitor.x().map_err(|e| e.to_string())?) as u32,
            (region.y - monitor.y().map_err(|e| e.to_string())?) as u32,
            region.width,
            region.height,
        )
        .map_err(|e| e.to_string())
}

pub fn screen_point(region: Region, x: i32, y: i32) -> (i32, i32) {
    (
        region.x + ((region.width.saturating_sub(1) as f64) * x as f64 / 1000.0).round() as i32,
        region.y + ((region.height.saturating_sub(1) as f64) * y as f64 / 1000.0).round() as i32,
    )
}

pub fn check_cancel(cancelled: &impl Fn() -> bool) -> Result<()> {
    if cancelled() {
        Err("Operation cancelled; some input may already have been sent".into())
    } else {
        Ok(())
    }
}

pub fn wait(ms: u64, cancelled: &impl Fn() -> bool) -> Result<()> {
    let start = Instant::now();
    let duration = Duration::from_millis(ms);
    loop {
        check_cancel(cancelled)?;
        let Some(remaining) = duration.checked_sub(start.elapsed()) else {
            return Ok(());
        };
        thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

pub fn initialize_dpi() -> Result<()> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn foreground_window() -> usize {
    #[cfg(windows)]
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow().0 as usize
    }
    #[cfg(not(windows))]
    {
        0
    }
}

pub fn keyboard_emergency_stop_pressed() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_F8};
        // Check the held state, not the unreliable process-shared "pressed since last call" bit.
        GetAsyncKeyState(VK_F8.0 as i32) < 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub fn move_mouse(enigo: &mut Enigo, x: i32, y: i32) -> Result<()> {
    #[cfg(windows)]
    {
        // Enigo 0.6 maps absolute coordinates against the primary monitor only.
        // VIRTUALDESK also handles secondary monitors with negative origins.
        use windows::Win32::UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*};
        let _ = enigo;
        unsafe {
            let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            if width <= 0
                || height <= 0
                || x < left
                || y < top
                || x as i64 >= left as i64 + width as i64
                || y as i64 >= top as i64 + height as i64
            {
                return Err("Mouse position is outside the virtual desktop".into());
            }
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: (((x as i64 - left as i64) * 65536 + 32768) / width as i64) as i32,
                        dy: (((y as i64 - top as i64) * 65536 + 32768) / height as i64) as i32,
                        dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                        ..Default::default()
                    },
                },
            };
            if SendInput(&[input], std::mem::size_of::<INPUT>() as i32) != 1 {
                return Err(
                    "Mouse input was blocked; check desktop access and target privilege level"
                        .into(),
                );
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    enigo
        .move_mouse(x, y, enigo::Coordinate::Abs)
        .map_err(|e| e.to_string())
}

pub fn new_input() -> Result<Enigo> {
    Enigo::new(&Settings::default()).map_err(|e| e.to_string())
}

pub fn click(
    enigo: &mut Enigo,
    point: (i32, i32),
    button: Button,
    clicks: u8,
    cancelled: &impl Fn() -> bool,
) -> Result<()> {
    if !(1..=2).contains(&clicks) {
        return Err("clicks must be 1 or 2".into());
    }
    check_cancel(cancelled)?;
    move_mouse(enigo, point.0, point.1)?;
    wait(70, cancelled)?;
    for index in 0..clicks {
        check_cancel(cancelled)?;
        enigo.button(button, Click).map_err(|e| e.to_string())?;
        if index + 1 < clicks {
            wait(90, cancelled)?;
        }
    }
    Ok(())
}

pub fn drag(
    enigo: &mut Enigo,
    from: (i32, i32),
    to: (i32, i32),
    button: Button,
    duration_ms: u64,
    cancelled: &impl Fn() -> bool,
) -> Result<()> {
    if !(1..=10000).contains(&duration_ms) {
        return Err("duration_ms must be 1..10000".into());
    }
    check_cancel(cancelled)?;
    move_mouse(enigo, from.0, from.1)?;
    enigo.button(button, Press).map_err(|e| e.to_string())?;
    let result = (|| {
        let start = Instant::now();
        while start.elapsed().as_millis() < duration_ms as u128 {
            check_cancel(cancelled)?;
            let p = (start.elapsed().as_secs_f64() * 1000.0 / duration_ms as f64).min(1.0);
            move_mouse(
                enigo,
                (from.0 as f64 + (to.0 as f64 - from.0 as f64) * p).round() as i32,
                (from.1 as f64 + (to.1 as f64 - from.1 as f64) * p).round() as i32,
            )?;
            wait(12, cancelled)?;
        }
        check_cancel(cancelled)?;
        move_mouse(enigo, to.0, to.1)
    })();
    let release = enigo.button(button, Release).map_err(|e| e.to_string());
    result.and(release)
}

pub fn parse_key(name: &str) -> Result<Key> {
    let name = name.to_ascii_lowercase();
    let key = match name.as_str() {
        "ctrl" | "control" => Key::Control,
        "alt" => Key::Alt, "shift" => Key::Shift,
        "win" | "meta" | "super" => Key::Meta,
        "enter" | "return" => Key::Return, "esc" | "escape" => Key::Escape,
        "tab" => Key::Tab, "space" => Key::Space,
        "backspace" => Key::Backspace, "delete" => Key::Delete,
        "insert" => Key::Insert, "home" => Key::Home, "end" => Key::End,
        "pageup" => Key::PageUp, "pagedown" => Key::PageDown,
        "up" | "arrowup" => Key::UpArrow, "down" | "arrowdown" => Key::DownArrow,
        "left" | "arrowleft" => Key::LeftArrow, "right" | "arrowright" => Key::RightArrow,
        "capslock" => Key::CapsLock,
        #[cfg(windows)]
        _ if name.len() == 1 && name.as_bytes()[0].is_ascii_alphanumeric() => Key::Other(name.as_bytes()[0].to_ascii_uppercase() as u32),
        #[cfg(windows)]
        _ if name.strip_prefix('f').and_then(|v| v.parse::<u32>().ok()).is_some_and(|n| (1..=24).contains(&n)) =>
            Key::Other(0x70 + name[1..].parse::<u32>().unwrap() - 1),
        _ => return Err(format!("Unsupported key {name:?}; use A-Z, 0-9, F1-F24, Enter, Tab, Escape, Space, navigation keys or Ctrl/Alt/Shift/Win. Use keyboard_type for text.")),
    };
    Ok(key)
}

pub fn press_keys(
    enigo: &mut impl Keyboard,
    keys: &[Key],
    hold_ms: u64,
    cancelled: &impl Fn() -> bool,
) -> Result<()> {
    if keys.is_empty() || keys.len() > 8 || hold_ms > 10000 {
        return Err("Use 1..8 keys and hold_ms <= 10000".into());
    }
    let mut held = Vec::new();
    let result = (|| {
        for key in keys {
            check_cancel(cancelled)?;
            enigo.key(*key, Press).map_err(|e| e.to_string())?;
            held.push(*key);
        }
        wait(hold_ms, cancelled)
    })();
    let mut release_error = None;
    for key in held.iter().rev() {
        if let Err(error) = enigo.key(*key, Release) {
            release_error = Some(error.to_string());
        }
    }
    result.and(release_error.map_or(Ok(()), Err))
}

pub fn type_text(
    enigo: &mut impl Keyboard,
    text: &str,
    interval_ms: u64,
    cancelled: &impl Fn() -> bool,
) -> Result<usize> {
    let count = text.chars().count();
    if count > 10000 || interval_ms > 1000 || count as u64 * interval_ms > 30000 {
        return Err(
            "Text limit is 10000 characters; total typing delay must be <= 30000 ms".into(),
        );
    }
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
    {
        return Err("Text contains unsupported control characters".into());
    }
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    for c in text.chars() {
        check_cancel(cancelled)?;
        match c {
            '\n' => enigo.key(Key::Return, Click),
            '\t' => enigo.key(Key::Tab, Click),
            // Avoid Enigo's newline/tab path, which also queues a Unicode control character.
            _ => enigo.text(c.encode_utf8(&mut [0; 4])),
        }
        .map_err(|e| e.to_string())?;
        if interval_ms > 0 {
            wait(interval_ms, cancelled)?;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    struct RecordingKeyboard {
        keys: Vec<(Key, enigo::Direction)>,
        text: String,
    }

    impl Keyboard for RecordingKeyboard {
        fn fast_text(&mut self, text: &str) -> enigo::InputResult<Option<()>> {
            self.text.push_str(text);
            Ok(Some(()))
        }

        fn key(&mut self, key: Key, direction: enigo::Direction) -> enigo::InputResult<()> {
            self.keys.push((key, direction));
            Ok(())
        }

        fn raw(&mut self, _: u16, _: enigo::Direction) -> enigo::InputResult<()> {
            unreachable!()
        }
    }

    #[test]
    fn cancelled_hold_releases_all_keys_in_reverse_order() {
        let mut input = RecordingKeyboard::default();
        let checks = Cell::new(0);
        let result = press_keys(&mut input, &[Key::Control, Key::Shift], 10000, &|| {
            checks.set(checks.get() + 1);
            checks.get() > 2
        });
        assert!(result.is_err());
        assert_eq!(
            input.keys,
            vec![
                (Key::Control, Press),
                (Key::Shift, Press),
                (Key::Shift, Release),
                (Key::Control, Release),
            ]
        );
    }

    #[test]
    fn cancellation_between_keys_releases_partial_chord() {
        let mut input = RecordingKeyboard::default();
        let checks = Cell::new(0);
        assert!(
            press_keys(&mut input, &[Key::Control, Key::Shift], 10000, &|| {
                checks.set(checks.get() + 1);
                checks.get() > 1
            })
            .is_err()
        );
        assert_eq!(
            input.keys,
            vec![(Key::Control, Press), (Key::Control, Release)]
        );
    }

    #[test]
    fn cancelled_typing_sends_no_remaining_characters() {
        let mut input = RecordingKeyboard::default();
        let checks = Cell::new(0);
        assert!(type_text(&mut input, "abc", 1000, &|| {
            checks.set(checks.get() + 1);
            checks.get() > 1
        })
        .is_err());
        assert_eq!(input.text, "a");
        assert!(input.keys.is_empty());
    }

    #[test]
    fn pre_cancelled_keyboard_actions_send_nothing() {
        let mut input = RecordingKeyboard::default();
        assert!(press_keys(&mut input, &[Key::Return], 50, &|| true).is_err());
        assert!(type_text(&mut input, "abc", 0, &|| true).is_err());
        assert!(input.keys.is_empty());
        assert!(input.text.is_empty());
    }

    #[test]
    fn normalized_edges_and_negative_origin() {
        let region = Region {
            x: -2560,
            y: 180,
            width: 2560,
            height: 1440,
        };
        assert_eq!(screen_point(region, 0, 0), (-2560, 180));
        assert_eq!(screen_point(region, 1000, 1000), (-1, 1619));
        assert_eq!(screen_point(region, 500, 500), (-1280, 900));
    }
    #[test]
    fn key_names_and_cancellation() {
        assert_eq!(parse_key("CTRL"), Ok(Key::Control));
        assert_eq!(parse_key("a"), parse_key("A"));
        assert_eq!(parse_key("F12"), Ok(Key::Other(0x7b)));
        assert!(parse_key("Ctrl+A").is_err());
        assert!(parse_key("F25").is_err());
        assert!(wait(10000, &|| true).is_err());
    }
}
