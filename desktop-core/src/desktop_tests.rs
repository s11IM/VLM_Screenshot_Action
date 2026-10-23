//! Interactive Windows desktop checks, kept apart from the unit tests in `lib.rs`.
//!
//! Every test here is `#[ignore]`d because it creates real windows or moves the
//! real pointer; CI never runs them. Run them one at a time on an interactive
//! desktop with the commands in `docs/validation.md`.

use super::*;

#[test]
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
            let out =
                detector.on_sample(&observe::downscale_gray(&image, 160), index as u64 * 600, 0);
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
    unsafe extern "system" fn receive(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
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
