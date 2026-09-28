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

pub mod observe;

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

#[cfg(windows)]
pub struct CaptureExclusion {
    window: usize,
    previous_affinity: u32,
}

#[cfg(windows)]
impl CaptureExclusion {
    pub fn new(window: usize) -> Result<Self> {
        use windows::{
            Wdk::System::SystemServices::RtlGetVersion,
            Win32::{
                Foundation::HWND,
                Graphics::Dwm::DwmFlush,
                System::SystemInformation::OSVERSIONINFOW,
                UI::WindowsAndMessaging::{
                    GetWindowDisplayAffinity, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
                },
            },
        };
        unsafe {
            let mut version = OSVERSIONINFOW {
                dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
                ..Default::default()
            };
            RtlGetVersion(&mut version)
                .ok()
                .map_err(|e| e.to_string())?;
            // Older Windows accepts this flag as WDA_MONITOR (a black box), not exclusion.
            if version.dwBuildNumber < 19041 {
                return Err("Capture exclusion requires Windows 10 2004 or later".into());
            }
            let hwnd = HWND(window as *mut std::ffi::c_void);
            let mut previous_affinity = 0;
            GetWindowDisplayAffinity(hwnd, &mut previous_affinity).map_err(|e| e.to_string())?;
            SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).map_err(|e| e.to_string())?;
            let guard = Self {
                window,
                previous_affinity,
            };
            DwmFlush().map_err(|e| e.to_string())?;
            Ok(guard)
        }
    }
}

#[cfg(windows)]
impl Drop for CaptureExclusion {
    fn drop(&mut self) {
        use windows::Win32::{
            Foundation::HWND,
            UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WINDOW_DISPLAY_AFFINITY},
        };
        unsafe {
            let _ = SetWindowDisplayAffinity(
                HWND(self.window as *mut std::ffi::c_void),
                WINDOW_DISPLAY_AFFINITY(self.previous_affinity),
            );
        }
    }
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

#[cfg(windows)]
pub fn raise_restored_window(window: usize) -> Result<()> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            SetForegroundWindow, SetWindowPos, HWND_TOP, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
        },
    };
    unsafe {
        let hwnd = HWND(window as *mut std::ffi::c_void);
        // Raise without permanently making the app topmost, even if Windows denies activation.
        SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        )
        .map_err(|error| error.to_string())?;
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(())
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

fn verify_pointer(enigo: &impl Mouse, expected: (i32, i32)) -> Result<()> {
    let actual = enigo.location().map_err(|error| error.to_string())?;
    if (actual.0 as i64 - expected.0 as i64).abs() > 2
        || (actual.1 as i64 - expected.1 as i64).abs() > 2
    {
        return Err(format!(
            "Mouse position mismatch: expected {expected:?}, actual {actual:?}"
        ));
    }
    Ok(())
}

fn with_mouse_button<T: Mouse>(
    input: &mut T,
    button: Button,
    action: impl FnOnce(&mut T) -> Result<()>,
) -> Result<()> {
    input
        .button(button, Press)
        .map_err(|error| error.to_string())?;
    let result = action(input);
    let release = input
        .button(button, Release)
        .map_err(|error| error.to_string());
    result.and(release)
}

pub fn click(
    enigo: &mut Enigo,
    point: (i32, i32),
    button: Button,
    clicks: u8,
    cancelled: &impl Fn() -> bool,
) -> Result<()> {
    click_with_move(enigo, point, button, clicks, cancelled, move_mouse)
}

fn click_with_move<T: Mouse>(
    input: &mut T,
    point: (i32, i32),
    button: Button,
    clicks: u8,
    cancelled: &impl Fn() -> bool,
    mut move_to: impl FnMut(&mut T, i32, i32) -> Result<()>,
) -> Result<()> {
    if !(1..=2).contains(&clicks) {
        return Err("clicks must be 1 or 2".into());
    }
    check_cancel(cancelled)?;
    move_to(input, point.0, point.1)?;
    wait(70, cancelled)?;
    let mut repositioned = false;
    for index in 0..clicks {
        check_cancel(cancelled)?;
        let actual = input.location().map_err(|error| error.to_string())?;
        if (actual.0 as i64 - point.0 as i64).abs() > 2
            || (actual.1 as i64 - point.1 as i64).abs() > 2
        {
            if repositioned {
                return Err(format!(
                    "Mouse position mismatch: expected {point:?}, actual {actual:?}"
                ));
            }
            // Correct one transient displacement before pressing, never replay sent clicks.
            check_cancel(cancelled)?;
            repositioned = true;
            move_to(input, point.0, point.1)?;
            wait(70, cancelled)?;
            verify_pointer(input, point)?;
        }
        check_cancel(cancelled)?;
        // Frame-polled games can miss a down/up pair dispatched in a single tick.
        with_mouse_button(input, button, |_| wait(60, cancelled))?;
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
    wait(100, cancelled)?;
    verify_pointer(enigo, from)?;
    with_mouse_button(enigo, button, |enigo| {
        // Allow hover hit-testing and pickup to run before the first drag movement.
        wait(100, cancelled)?;
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
        move_mouse(enigo, to.0, to.1)?;
        wait(80, cancelled)?;
        verify_pointer(enigo, to)
    })
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
    struct RecordingMouse {
        buttons: Vec<(Button, enigo::Direction)>,
        position: (i32, i32),
        drift_on_release: bool,
    }

    impl Mouse for RecordingMouse {
        fn button(
            &mut self,
            button: Button,
            direction: enigo::Direction,
        ) -> enigo::InputResult<()> {
            self.buttons.push((button, direction));
            if direction == Release && self.drift_on_release {
                self.position.0 += 10;
            }
            Ok(())
        }
        fn move_mouse(&mut self, _: i32, _: i32, _: enigo::Coordinate) -> enigo::InputResult<()> {
            unreachable!()
        }
        fn scroll(&mut self, _: i32, _: enigo::Axis) -> enigo::InputResult<()> {
            unreachable!()
        }
        fn main_display(&self) -> enigo::InputResult<(i32, i32)> {
            unreachable!()
        }
        fn location(&self) -> enigo::InputResult<(i32, i32)> {
            Ok(self.position)
        }
    }

    #[test]
    fn click_repositions_once_without_replaying_a_sent_click() {
        for drift_on_release in [false, true] {
            let mut input = RecordingMouse {
                drift_on_release,
                ..Default::default()
            };
            let mut moves = 0;
            click_with_move(
                &mut input,
                (100, 200),
                Button::Left,
                2,
                &|| false,
                |input, x, y| {
                    moves += 1;
                    input.position = if moves == 1 && !drift_on_release {
                        (x + 10, y)
                    } else {
                        (x, y)
                    };
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(moves, 2);
            assert_eq!(
                input.buttons,
                vec![
                    (Button::Left, Press),
                    (Button::Left, Release),
                    (Button::Left, Press),
                    (Button::Left, Release)
                ]
            );
        }
    }

    #[test]
    fn click_keeps_tolerance_and_does_not_reposition_when_aligned() {
        let mut input = RecordingMouse::default();
        let mut moves = 0;
        click_with_move(
            &mut input,
            (100, 200),
            Button::Left,
            2,
            &|| false,
            |input, x, y| {
                moves += 1;
                input.position = (x + 2, y - 2);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(moves, 1);
        assert_eq!(input.buttons.len(), 4);
    }

    #[test]
    fn click_stops_on_persistent_or_repeated_displacement() {
        for recover_first in [false, true] {
            let mut input = RecordingMouse {
                drift_on_release: recover_first,
                ..Default::default()
            };
            let mut moves = 0;
            let result = click_with_move(
                &mut input,
                (100, 200),
                Button::Left,
                2,
                &|| false,
                |input, x, y| {
                    moves += 1;
                    input.position = if moves == 2 && recover_first {
                        (x, y)
                    } else {
                        (x + 10, y)
                    };
                    Ok(())
                },
            );
            assert!(result.unwrap_err().starts_with("Mouse position mismatch"));
            assert_eq!(moves, 2);
            assert_eq!(input.buttons.len(), if recover_first { 2 } else { 0 });
        }
    }

    #[test]
    fn click_reposition_honors_cancellation_and_move_errors() {
        for cancel in [false, true] {
            let mut input = RecordingMouse::default();
            let stopped = Cell::new(false);
            let mut moves = 0;
            let result = click_with_move(
                &mut input,
                (100, 200),
                Button::Left,
                1,
                &|| stopped.get(),
                |input, x, y| {
                    moves += 1;
                    input.position = (x + 10, y);
                    if moves == 2 {
                        if !cancel {
                            return Err("move failed".into());
                        }
                        stopped.set(true);
                    }
                    Ok(())
                },
            );
            assert!(result.is_err());
            assert_eq!(moves, 2);
            assert!(input.buttons.is_empty());
        }
        let mut input = RecordingMouse::default();
        assert!(click_with_move(
            &mut input,
            (100, 200),
            Button::Left,
            1,
            &|| true,
            |_, _, _| { panic!("cancelled click must not move the pointer") }
        )
        .is_err());
    }

    #[test]
    fn mouse_button_is_held_then_released_on_success_error_and_cancellation() {
        for fails in [false, true] {
            let mut input = RecordingMouse::default();
            let result = with_mouse_button(&mut input, Button::Left, |input| {
                assert_eq!(input.buttons, vec![(Button::Left, Press)]);
                wait(0, &|| fails)
            });
            assert_eq!(result.is_err(), fails);
            assert_eq!(
                input.buttons,
                vec![(Button::Left, Press), (Button::Left, Release)]
            );
        }
        let mut input = RecordingMouse::default();
        assert!(
            with_mouse_button(&mut input, Button::Left, |_| Err("move failed".into())).is_err()
        );
        assert_eq!(input.buttons.last(), Some(&(Button::Left, Release)));
    }

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
    #[cfg(windows)]
    fn capture_exclusion_rejects_invalid_window() {
        assert!(CaptureExclusion::new(0).is_err());
    }

    #[test]
    #[cfg(windows)]
    #[ignore = "requires an interactive Windows desktop; briefly raises test windows"]
    fn restored_window_returns_above_target_without_becoming_topmost() {
        use windows::{
            core::w,
            Win32::{Foundation::HWND, UI::WindowsAndMessaging::*},
        };
        struct Windows(Vec<HWND>, HWND);
        impl Drop for Windows {
            fn drop(&mut self) {
                unsafe {
                    for hwnd in &self.0 {
                        let _ = DestroyWindow(*hwnd);
                    }
                    let _ = SetForegroundWindow(self.1);
                }
            }
        }
        unsafe {
            let mut windows = Windows(Vec::new(), GetForegroundWindow());
            for _ in 0..2 {
                let hwnd = CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    w!("STATIC"),
                    w!("Window restoration test"),
                    WS_POPUP,
                    96,
                    96,
                    240,
                    160,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
                windows.0.push(hwnd);
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            let app = windows.0[0];
            let target = windows.0[1];
            for _ in 0..3 {
                let _ = ShowWindow(app, SW_HIDE);
                assert!(!IsWindowVisible(app).as_bool());
                SetWindowPos(target, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE).unwrap();
                raise_restored_window(app.0 as usize).unwrap();
                assert!(IsWindowVisible(app).as_bool());
                assert!(!IsIconic(app).as_bool());
                assert_eq!(GetWindowLongW(app, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0, 0);
                let mut below = GetWindow(app, GW_HWNDNEXT).unwrap();
                while below != target && !below.is_invalid() {
                    below = GetWindow(below, GW_HWNDNEXT).unwrap_or_default();
                }
                assert_eq!(
                    below, target,
                    "restored app must be above the exposed target"
                );
            }
        }
    }

    #[test]
    #[cfg(windows)]
    #[ignore = "requires an interactive Windows desktop; briefly shows test windows"]
    fn capture_exclusion_keeps_window_visible_but_samples_underneath() {
        use windows::{
            core::w,
            Win32::{
                Foundation::{HWND, RECT},
                Graphics::{Dwm::DwmFlush, Gdi::UpdateWindow},
                UI::WindowsAndMessaging::*,
            },
        };
        struct TestWindow(HWND);
        impl Drop for TestWindow {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
        unsafe {
            let _ = initialize_dpi();
            let create = |static_style: u32| {
                let window = TestWindow(
                    CreateWindowExW(
                        WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                        w!("STATIC"),
                        w!("Capture exclusion test"),
                        WS_POPUP | WINDOW_STYLE(static_style),
                        96,
                        96,
                        240,
                        160,
                        None,
                        None,
                        None,
                        None,
                    )
                    .unwrap(),
                );
                let _ = ShowWindow(window.0, SW_SHOWNOACTIVATE);
                let _ = UpdateWindow(window.0);
                window
            };
            let background = create(6); // SS_WHITERECT
            let mut rect = RECT::default();
            GetWindowRect(background.0, &mut rect).unwrap();
            let region = Region {
                x: rect.left + 40,
                y: rect.top + 40,
                width: 32,
                height: 32,
            };
            let sample = || {
                DwmFlush().unwrap();
                thread::sleep(Duration::from_millis(100));
                capture_region(region).unwrap()
            };
            let baseline = sample();
            let overlay = create(4); // SS_BLACKRECT
            SetWindowPos(
                overlay.0,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .unwrap();
            let before = sample();
            assert_ne!(before.get_pixel(16, 16), baseline.get_pixel(16, 16));
            let guard = CaptureExclusion::new(overlay.0 .0 as usize).unwrap();
            assert!(IsWindowVisible(overlay.0).as_bool());
            let clean = sample();
            assert_eq!(clean, baseline);
            let mut detector = observe::SettleDetector::new(
                observe::ObserveParams::default(),
                observe::downscale_gray(&clean, 160),
            );
            for (index, offset) in [0, 10, 20, 0, 10, 20, 0, 10].into_iter().enumerate() {
                SetWindowPos(
                    overlay.0,
                    Some(HWND_TOPMOST),
                    rect.left + offset,
                    rect.top,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE,
                )
                .unwrap();
                let image = sample();
                let out = detector.on_sample(
                    &observe::downscale_gray(&image, 160),
                    index as u64 * 600,
                    0,
                );
                assert_eq!(out.metrics.changed_fraction, 0.0);
                assert_eq!(out.metrics.motion_fraction, 0.0);
                assert!(!out.triggered);
            }
            let _changed_background = create(4);
            SetWindowPos(
                overlay.0,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .unwrap();
            let changed = sample();
            assert_ne!(changed, clean);
            let mut triggered = false;
            for index in 8..16 {
                triggered |= detector
                    .on_sample(&observe::downscale_gray(&changed, 160), index * 600, 0)
                    .triggered;
            }
            assert!(
                triggered,
                "background changes under an excluded window must still trigger"
            );
            drop(guard);
            let mut affinity = u32::MAX;
            GetWindowDisplayAffinity(overlay.0, &mut affinity).unwrap();
            assert_eq!(affinity, WDA_NONE.0);
            let restored = sample();
            assert!(IsWindowVisible(overlay.0).as_bool());
            assert_eq!(restored, before);
        }
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
    #[cfg(windows)]
    #[ignore = "requires an interactive desktop; moves and restores the pointer, without clicking"]
    fn reported_drag_coordinates_reach_expected_screen_pixels() {
        // Test executables do not embed the application's PerMonitorV2 manifest.
        let _ = initialize_dpi();
        let mut input = new_input().unwrap();
        let original = input.location().unwrap();
        struct RestorePointer((i32, i32));
        impl Drop for RestorePointer {
            fn drop(&mut self) {
                if let Ok(mut input) = new_input() {
                    let _ = move_mouse(&mut input, self.0 .0, self.0 .1);
                }
            }
        }
        let _restore = RestorePointer(original);
        let region = Region {
            x: 143,
            y: 103,
            width: 1637,
            height: 922,
        };
        monitor_for_region(region).unwrap();
        for (normalized, expected) in [
            ((796, 920), (1445, 950)),
            ((796, 600), (1445, 656)),
            ((795, 920), (1444, 950)),
            ((500, 650), (961, 702)),
        ] {
            let point = screen_point(region, normalized.0, normalized.1);
            assert_eq!(point, expected);
            move_mouse(&mut input, point.0, point.1).unwrap();
            wait(70, &|| false).unwrap();
            let actual = input.location().unwrap();
            println!("normalized={normalized:?} expected={expected:?} actual={actual:?}");
            assert_eq!(actual, expected);
        }
    }

    #[test]
    #[cfg(windows)]
    #[ignore = "requires an interactive desktop; sends click/drag only to a temporary test window"]
    fn native_window_receives_held_click_and_drag_endpoints() {
        use std::{cell::RefCell, sync::mpsc};
        use windows::{
            core::w,
            Win32::{Foundation::*, UI::WindowsAndMessaging::*},
        };
        thread_local! {
            static EVENTS: RefCell<Vec<(u32, i32, i32, Instant)>> = const { RefCell::new(Vec::new()) };
        }
        unsafe extern "system" fn receive(
            hwnd: HWND,
            message: u32,
            wp: WPARAM,
            lp: LPARAM,
        ) -> LRESULT {
            if matches!(message, WM_LBUTTONDOWN | WM_LBUTTONUP) {
                EVENTS.with(|events| {
                    events.borrow_mut().push((
                        message,
                        lp.0 as i16 as i32,
                        (lp.0 >> 16) as i16 as i32,
                        Instant::now(),
                    ))
                });
            }
            DefWindowProcW(hwnd, message, wp, lp)
        }
        struct Restore(HWND, HWND, (i32, i32));
        impl Drop for Restore {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyWindow(self.0);
                    let _ = UnregisterClassW(w!("VlmPointerReceiverTest"), None);
                    let _ = SetForegroundWindow(self.1);
                }
                if let Ok(mut input) = new_input() {
                    let _ = move_mouse(&mut input, self.2 .0, self.2 .1);
                }
            }
        }
        let _ = initialize_dpi();
        unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(receive),
                lpszClassName: w!("VlmPointerReceiverTest"),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&class), 0);
            let original = new_input().unwrap().location().unwrap();
            let foreground = GetForegroundWindow();
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class.lpszClassName,
                w!("Pointer input regression test"),
                WS_POPUP,
                200,
                200,
                320,
                240,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let _restore = Restore(hwnd, foreground, original);
            let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
            let _ = SetForegroundWindow(hwnd);
            let (sender, receiver) = mpsc::channel();
            let worker = thread::spawn(move || {
                let result = (|| {
                    let mut input = new_input()?;
                    click(&mut input, (240, 240), Button::Left, 1, &|| false)?;
                    drag(
                        &mut input,
                        (240, 240),
                        (440, 340),
                        Button::Left,
                        300,
                        &|| false,
                    )
                })();
                sender.send(result).unwrap();
            });
            let deadline = Instant::now() + Duration::from_secs(10);
            let result = loop {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                if let Ok(result) = receiver.try_recv() {
                    break result;
                }
                assert!(Instant::now() < deadline, "native input test timed out");
                thread::sleep(Duration::from_millis(2));
            };
            worker.join().unwrap();
            result.unwrap();
            // Drain any final queued button-up before checking receipt.
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&message);
            }
            EVENTS.with(|events| {
                let events = events.borrow();
                assert_eq!(
                    events
                        .iter()
                        .map(|(message, x, y, _)| (*message, *x, *y))
                        .collect::<Vec<_>>(),
                    vec![
                        (WM_LBUTTONDOWN, 40, 40),
                        (WM_LBUTTONUP, 40, 40),
                        (WM_LBUTTONDOWN, 40, 40),
                        (WM_LBUTTONUP, 240, 140),
                    ]
                );
                assert!(events[1].3.duration_since(events[0].3) >= Duration::from_millis(40));
                assert!(events[3].3.duration_since(events[2].3) >= Duration::from_millis(450));
            });
        }
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
