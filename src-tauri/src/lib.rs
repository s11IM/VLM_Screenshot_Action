use std::{
    collections::HashMap,
    error::Error as StdError,
    fs::{self, OpenOptions},
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use desktop_automation_core::{self as desktop, monitor_for_region, screen_point, Region};
use enigo::{Button, Enigo, Key, Mouse, Settings as EnigoSettings};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{
    LogicalSize, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::oneshot;
use xcap::{
    image::{DynamicImage, ImageFormat},
    Monitor,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InputAction {
    #[serde(rename = "type")]
    action_type: String,
    x: Option<i32>,
    y: Option<i32>,
    clicks: Option<u8>,
    from_x: Option<i32>,
    from_y: Option<i32>,
    to_x: Option<i32>,
    to_y: Option<i32>,
    text: Option<String>,
    interval_ms: Option<u64>,
    submit: Option<bool>,
    keys: Option<Vec<String>>,
    hold_ms: Option<u64>,
}

#[derive(Clone, Copy)]
struct SelectionContext {
    monitor_x: i32,
    monitor_y: i32,
    monitor_width: u32,
    monitor_height: u32,
}

#[derive(Default)]
struct RegionSelectionState {
    pending: Mutex<Option<oneshot::Sender<Result<Region, String>>>>,
    context: Mutex<Option<SelectionContext>>,
}

#[derive(Default)]
struct OperationState {
    operations: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CapturedImage {
    data_url: String,
    frame_id: String,
}

struct CapturedFrame {
    id: String,
    region: Region,
    foreground: usize,
    created: Instant,
}

#[derive(Default)]
struct CaptureState {
    frame: Mutex<Option<CapturedFrame>>,
}

impl CaptureState {
    fn take_keyboard_target(
        &self,
        frame_id: Option<&str>,
        region: Region,
        foreground: usize,
    ) -> Result<usize, String> {
        let mut current = self.frame.lock().map_err(|_| "截图状态不可用")?;
        let frame = current
            .as_ref()
            .ok_or("键盘安全中断：请先重新截图并确认焦点")?;
        if frame_id != Some(frame.id.as_str())
            || frame.region != region
            || frame.created.elapsed() > Duration::from_secs(300)
        {
            return Err("键盘安全中断：截图已失效或区域已变化，请重新截图".into());
        }
        if frame.foreground == 0 || frame.foreground != foreground {
            return Err("键盘安全中断：前台窗口与截图时不一致，请确认目标窗口后重新截图".into());
        }
        let target = frame.foreground;
        *current = None;
        Ok(target)
    }
}

fn keyboard_should_stop(
    cancelled: &AtomicBool,
    target: usize,
    foreground: usize,
    emergency_stop: bool,
) -> bool {
    if emergency_stop || target == 0 || foreground != target {
        cancelled.store(true, Ordering::SeqCst);
    }
    cancelled.load(Ordering::SeqCst)
}

#[derive(Clone, Copy)]
struct WindowLayout {
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    maximized: bool,
    always_on_top: bool,
    resizable: bool,
}

#[derive(Default)]
struct WindowLayoutState {
    normal: Mutex<Option<WindowLayout>>,
}

#[derive(Clone, Copy)]
struct WindowPresentation {
    position: Option<PhysicalPosition<i32>>,
    focused: bool,
    maximized: bool,
    minimized: bool,
    hidden_for_action: bool,
}

impl OperationState {
    fn begin(&self, operation_id: String) -> Result<(), String> {
        self.operations
            .lock()
            .map_err(|_| "运行状态不可用".to_string())?
            .insert(operation_id, Arc::new(AtomicBool::new(false)));
        Ok(())
    }

    fn flag(&self, operation_id: &str) -> Result<Arc<AtomicBool>, String> {
        self.operations
            .lock()
            .map_err(|_| "运行状态不可用".to_string())?
            .get(operation_id)
            .cloned()
            .ok_or_else(|| "本轮运行已结束".to_string())
    }

    fn cancel(&self, operation_id: &str) -> Result<(), String> {
        if let Some(flag) = self
            .operations
            .lock()
            .map_err(|_| "运行状态不可用".to_string())?
            .get(operation_id)
        {
            flag.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn finish(&self, operation_id: &str) -> Result<(), String> {
        self.operations
            .lock()
            .map_err(|_| "运行状态不可用".to_string())?
            .remove(operation_id);
        Ok(())
    }
}

fn hide_window_for_background_action(window: &WebviewWindow) -> Result<WindowPresentation, String> {
    let minimized = window.is_minimized().unwrap_or(false);
    let visible = window.is_visible().unwrap_or(true);
    let presentation = WindowPresentation {
        position: window.outer_position().ok(),
        focused: window.is_focused().unwrap_or(false),
        maximized: window.is_maximized().unwrap_or(false),
        minimized,
        hidden_for_action: visible && !minimized,
    };
    if presentation.hidden_for_action {
        window.hide().map_err(|error| error.to_string())?;
    }
    Ok(presentation)
}

fn restore_window_after_background_action(
    window: &WebviewWindow,
    presentation: WindowPresentation,
    restore_focus: bool,
) {
    if !presentation.hidden_for_action {
        return;
    }
    let _ = window.show();
    if presentation.maximized {
        let _ = window.maximize();
    } else {
        let _ = window.unmaximize();
        if let Some(position) = presentation.position {
            let _ = window.set_position(position);
        }
    }
    if presentation.minimized {
        let _ = window.minimize();
    } else if restore_focus && presentation.focused {
        let _ = window.set_focus();
    }
}

const LOG_FILE_NAME: &str = "vlm_screenshot_action.log";
const LOG_MAX_BYTES: u64 = 2 * 1024 * 1024;
const LOG_ARCHIVES: usize = 4;
static APP_LOGGER: OnceLock<AppLogger> = OnceLock::new();
static API_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

struct AppLogger {
    path: PathBuf,
    lock: Mutex<()>,
}

impl AppLogger {
    fn new(directory: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        Ok(Self {
            path: directory.join(LOG_FILE_NAME),
            lock: Mutex::new(()),
        })
    }

    fn write(&self, level: &str, event: &str, data: Value) -> Result<(), String> {
        let _guard = self.lock.lock().map_err(|_| "日志锁不可用".to_string())?;
        self.rotate_if_needed()?;

        let timestamp = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| unix_millis().to_string());
        let entry = json!({
            "timestamp": timestamp,
            "level": normalize_level(level),
            "event": sanitize_event_name(event),
            "pid": std::process::id(),
            "data": sanitize_log_value(&data, 0),
        });
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| error.to_string())?;
        serde_json::to_writer(&mut file, &entry).map_err(|error| error.to_string())?;
        file.write_all(b"\n").map_err(|error| error.to_string())?;
        file.flush().map_err(|error| error.to_string())
    }

    fn rotate_if_needed(&self) -> Result<(), String> {
        let size = fs::metadata(&self.path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        if size < LOG_MAX_BYTES {
            return Ok(());
        }

        for index in (1..=LOG_ARCHIVES).rev() {
            let source = archive_path(&self.path, index);
            if index == LOG_ARCHIVES {
                let _ = fs::remove_file(&source);
                continue;
            }
            if source.exists() {
                let target = archive_path(&self.path, index + 1);
                let _ = fs::remove_file(&target);
                fs::rename(&source, target).map_err(|error| error.to_string())?;
            }
        }
        let first_archive = archive_path(&self.path, 1);
        let _ = fs::remove_file(&first_archive);
        if self.path.exists() {
            fs::rename(&self.path, first_archive).map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

fn archive_path(path: &Path, index: usize) -> PathBuf {
    PathBuf::from(format!("{}.{}", path.display(), index))
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn normalize_level(level: &str) -> &'static str {
    match level.to_ascii_lowercase().as_str() {
        "debug" => "debug",
        "warn" => "warn",
        "error" => "error",
        _ => "info",
    }
}

fn sanitize_event_name(event: &str) -> String {
    event
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
        .take(80)
        .collect()
}

fn sensitive_log_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "apikey"
            | "api_key"
            | "authorization"
            | "body"
            | "content"
            | "dataurl"
            | "image"
            | "imageurl"
            | "messages"
            | "modelmemory"
            | "prompt"
            | "screenshot"
            | "systemprompt"
            | "text"
            | "token"
    )
}

fn sanitize_log_value(value: &Value, depth: usize) -> Value {
    if depth > 5 {
        return Value::String("[depth-limited]".into());
    }
    match value {
        Value::Object(object) => {
            let mut safe = Map::new();
            for (key, value) in object {
                safe.insert(
                    key.chars().take(80).collect(),
                    if sensitive_log_key(key) {
                        Value::String("[redacted]".into())
                    } else {
                        sanitize_log_value(value, depth + 1)
                    },
                );
            }
            Value::Object(safe)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(30)
                .map(|item| sanitize_log_value(item, depth + 1))
                .collect(),
        ),
        Value::String(text) => Value::String(text.chars().take(500).collect()),
        other => other.clone(),
    }
}

fn log_event(level: &str, event: &str, data: Value) {
    if let Some(logger) = APP_LOGGER.get() {
        let _ = logger.write(level, event, data);
    }
}

fn region_metadata(region: Region) -> Value {
    json!({
        "x": region.x,
        "y": region.y,
        "width": region.width,
        "height": region.height,
    })
}

fn selected_screen_region(
    context: SelectionContext,
    window_position: PhysicalPosition<i32>,
    window_size: PhysicalSize<u32>,
    viewport_width: f64,
    viewport_height: f64,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<Region, String> {
    let values = [viewport_width, viewport_height, x, y, width, height];
    if values.iter().any(|value| !value.is_finite())
        || viewport_width <= 0.0
        || viewport_height <= 0.0
        || x < 0.0
        || y < 0.0
        || width <= 0.0
        || height <= 0.0
        || x + width > viewport_width + 1.0
        || y + height > viewport_height + 1.0
    {
        return Err("选择区域坐标无效，请重新框选".into());
    }

    let scale_x = window_size.width as f64 / viewport_width;
    let scale_y = window_size.height as f64 / viewport_height;
    let left = window_position.x as i64 + (x * scale_x).round() as i64;
    let top = window_position.y as i64 + (y * scale_y).round() as i64;
    let right = window_position.x as i64 + ((x + width) * scale_x).round() as i64;
    let bottom = window_position.y as i64 + ((y + height) * scale_y).round() as i64;
    let monitor_right = context.monitor_x as i64 + context.monitor_width as i64;
    let monitor_bottom = context.monitor_y as i64 + context.monitor_height as i64;

    if left < context.monitor_x as i64
        || top < context.monitor_y as i64
        || right > monitor_right
        || bottom > monitor_bottom
        || right - left < 24
        || bottom - top < 24
    {
        return Err("选择区域无效，请重新框选".into());
    }

    Ok(Region {
        x: i32::try_from(left).map_err(|_| "选择区域横坐标超出范围")?,
        y: i32::try_from(top).map_err(|_| "选择区域纵坐标超出范围")?,
        width: u32::try_from(right - left).map_err(|_| "选择区域宽度超出范围")?,
        height: u32::try_from(bottom - top).map_err(|_| "选择区域高度超出范围")?,
    })
}

#[tauri::command]
fn client_log(level: String, event: String, data: Option<Value>) {
    log_event(
        &level,
        &format!("frontend.{event}"),
        data.unwrap_or_else(|| json!({})),
    );
}

#[tauri::command]
fn enter_mini_mode(
    window: WebviewWindow,
    state: State<'_, WindowLayoutState>,
) -> Result<(), String> {
    let mut normal = state.normal.lock().map_err(|_| "窗口状态不可用")?;
    if normal.is_none() {
        *normal = Some(WindowLayout {
            position: window.outer_position().map_err(|error| error.to_string())?,
            size: window.inner_size().map_err(|error| error.to_string())?,
            maximized: window.is_maximized().unwrap_or(false),
            always_on_top: window.is_always_on_top().unwrap_or(false),
            resizable: window.is_resizable().unwrap_or(true),
        });
    }
    let position = normal.as_ref().map(|layout| layout.position);
    drop(normal);

    window.unmaximize().map_err(|error| error.to_string())?;
    window
        .set_min_size(Some(LogicalSize::new(320.0, 112.0)))
        .map_err(|error| error.to_string())?;
    window
        .set_size(LogicalSize::new(380.0, 130.0))
        .map_err(|error| error.to_string())?;
    if let Some(position) = position {
        window
            .set_position(position)
            .map_err(|error| error.to_string())?;
    }
    window
        .set_resizable(false)
        .map_err(|error| error.to_string())?;
    window
        .set_always_on_top(true)
        .map_err(|error| error.to_string())?;
    window
        .set_title("VLM_Screenshot_Action - 运行中")
        .map_err(|error| error.to_string())?;
    log_event("info", "window.mini_mode_entered", json!({}));
    Ok(())
}

#[tauri::command]
fn exit_mini_mode(
    window: WebviewWindow,
    state: State<'_, WindowLayoutState>,
) -> Result<(), String> {
    let layout = state.normal.lock().map_err(|_| "窗口状态不可用")?.take();
    let Some(layout) = layout else {
        return Ok(());
    };

    window.unmaximize().map_err(|error| error.to_string())?;
    window
        .set_min_size(Some(LogicalSize::new(1024.0, 640.0)))
        .map_err(|error| error.to_string())?;
    window
        .set_resizable(layout.resizable)
        .map_err(|error| error.to_string())?;
    window
        .set_size(layout.size)
        .map_err(|error| error.to_string())?;
    window
        .set_position(layout.position)
        .map_err(|error| error.to_string())?;
    window
        .set_always_on_top(layout.always_on_top)
        .map_err(|error| error.to_string())?;
    if layout.maximized {
        window.maximize().map_err(|error| error.to_string())?;
    }
    window
        .set_title("VLM_Screenshot_Action")
        .map_err(|error| error.to_string())?;
    let _ = window.set_focus();
    log_event("info", "window.mini_mode_exited", json!({}));
    Ok(())
}

#[tauri::command]
fn begin_operation(
    operation_id: String,
    round_id: String,
    trigger: String,
    state: State<'_, OperationState>,
) -> Result<(), String> {
    state.begin(operation_id.clone())?;
    log_event(
        "info",
        "operation.started",
        json!({
            "operationId": operation_id,
            "roundId": round_id,
            "trigger": trigger,
        }),
    );
    Ok(())
}

#[tauri::command]
fn cancel_operation(
    operation_id: String,
    round_id: String,
    state: State<'_, OperationState>,
) -> Result<(), String> {
    log_event(
        "warn",
        "operation.cancelled",
        json!({ "operationId": operation_id, "roundId": round_id }),
    );
    state.cancel(&operation_id)
}

#[tauri::command]
fn finish_operation(
    operation_id: String,
    round_id: String,
    outcome: String,
    state: State<'_, OperationState>,
) -> Result<(), String> {
    state.finish(&operation_id)?;
    log_event(
        "info",
        "operation.finished",
        json!({
            "operationId": operation_id,
            "roundId": round_id,
            "outcome": outcome,
        }),
    );
    Ok(())
}

#[tauri::command]
async fn select_region(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, RegionSelectionState>,
) -> Result<Region, String> {
    log_event("info", "region.selection_started", json!({}));
    if state
        .pending
        .lock()
        .map_err(|_| "区域选择状态不可用")?
        .is_some()
    {
        return Err("已经有一个区域选择窗口正在运行".into());
    }

    let enigo = Enigo::new(&EnigoSettings::default()).map_err(|error| error.to_string())?;
    let (cursor_x, cursor_y) = enigo.location().map_err(|error| error.to_string())?;
    let context = {
        let monitor = Monitor::from_point(cursor_x, cursor_y).map_err(|error| error.to_string())?;
        SelectionContext {
            monitor_x: monitor.x().map_err(|error| error.to_string())?,
            monitor_y: monitor.y().map_err(|error| error.to_string())?,
            monitor_width: monitor.width().map_err(|error| error.to_string())?,
            monitor_height: monitor.height().map_err(|error| error.to_string())?,
        }
    };
    *state.context.lock().map_err(|_| "区域选择状态不可用")? = Some(context);

    window.hide().map_err(|error| error.to_string())?;
    thread::sleep(Duration::from_millis(100));

    if let Some(existing) = app.get_webview_window("region-selector") {
        let _ = existing.close();
    }

    let selector_result = WebviewWindowBuilder::new(
        &app,
        "region-selector",
        WebviewUrl::App("index.html?selector=1".into()),
    )
    .title("选择目标区域")
    .decorations(false)
    .shadow(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .closable(false)
    .visible(false)
    .position(context.monitor_x as f64, context.monitor_y as f64)
    .inner_size(context.monitor_width as f64, context.monitor_height as f64)
    .build();
    let selector = match selector_result {
        Ok(selector) => selector,
        Err(error) => {
            log_event(
                "error",
                "region.selector_window_failed",
                json!({ "error": error.to_string() }),
            );
            let _ = window.show();
            let _ = window.set_focus();
            *state.context.lock().map_err(|_| "区域选择状态不可用")? = None;
            return Err(error.to_string());
        }
    };

    // Builder coordinates are logical pixels. Reapply the target monitor geometry in
    // physical pixels so mixed-DPI and non-zero monitor origins cannot shift the overlay.
    if let Err(error) = selector
        .set_position(PhysicalPosition::new(context.monitor_x, context.monitor_y))
        .and_then(|_| {
            selector.set_size(PhysicalSize::new(
                context.monitor_width,
                context.monitor_height,
            ))
        })
    {
        log_event(
            "error",
            "region.selector_geometry_failed",
            json!({ "error": error.to_string() }),
        );
        let _ = selector.close();
        let _ = window.show();
        let _ = window.set_focus();
        *state.context.lock().map_err(|_| "区域选择状态不可用")? = None;
        return Err(error.to_string());
    }

    selector.show().map_err(|error| error.to_string())?;
    selector.set_focus().map_err(|error| error.to_string())?;
    let (sender, receiver) = oneshot::channel();
    *state.pending.lock().map_err(|_| "区域选择状态不可用")? = Some(sender);

    let result = match receiver.await {
        Ok(result) => result,
        Err(_) => Err("区域选择窗口意外关闭".to_string()),
    };
    let _ = selector.close();
    let _ = window.show();
    let _ = window.set_focus();
    match &result {
        Ok(region) => log_event(
            "info",
            "region.selection_completed",
            region_metadata(*region),
        ),
        Err(error) => log_event(
            "warn",
            "region.selection_ended_without_result",
            json!({ "error": error }),
        ),
    }
    result
}

#[tauri::command]
fn complete_region_selection(
    window: WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    viewport_width: f64,
    viewport_height: f64,
    state: State<'_, RegionSelectionState>,
) -> Result<(), String> {
    let context = state
        .context
        .lock()
        .map_err(|_| "区域选择状态不可用")?
        .as_ref()
        .copied()
        .ok_or_else(|| "没有正在进行的区域选择".to_string())?;

    let window_position = window.inner_position().map_err(|error| error.to_string())?;
    let window_size = window.inner_size().map_err(|error| error.to_string())?;
    let region = selected_screen_region(
        context,
        window_position,
        window_size,
        viewport_width,
        viewport_height,
        x,
        y,
        width,
        height,
    )
    .map_err(|error| {
        log_event(
            "warn",
            "region.selection_rejected",
            json!({
                "error": error.clone(),
                "selection": { "x": x, "y": y, "width": width, "height": height },
                "viewport": { "width": viewport_width, "height": viewport_height },
                "window": {
                    "x": window_position.x,
                    "y": window_position.y,
                    "width": window_size.width,
                    "height": window_size.height,
                },
            }),
        );
        error
    })?;

    log_event(
        "debug",
        "region.selection_mapped",
        json!({
            "region": region_metadata(region),
            "selection": { "x": x, "y": y, "width": width, "height": height },
            "viewport": { "width": viewport_width, "height": viewport_height },
            "window": {
                "x": window_position.x,
                "y": window_position.y,
                "width": window_size.width,
                "height": window_size.height,
            },
        }),
    );

    *state.context.lock().map_err(|_| "区域选择状态不可用")? = None;
    if let Some(sender) = state
        .pending
        .lock()
        .map_err(|_| "区域选择状态不可用")?
        .take()
    {
        sender
            .send(Ok(region))
            .map_err(|_| "无法提交区域选择".to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn cancel_region_selection(state: State<'_, RegionSelectionState>) -> Result<(), String> {
    log_event("info", "region.selection_cancelled", json!({}));
    *state.context.lock().map_err(|_| "区域选择状态不可用")? = None;
    if let Some(sender) = state
        .pending
        .lock()
        .map_err(|_| "区域选择状态不可用")?
        .take()
    {
        let _ = sender.send(Err("已取消区域选择".into()));
    }
    Ok(())
}

fn draw_crosshair(image: &mut xcap::image::RgbaImage, cx: i32, cy: i32) {
    use xcap::image::Rgba;
    let w = image.width() as i32;
    let h = image.height() as i32;
    let arm = 14i32;
    let gap = 4i32;
    let red = Rgba([255u8, 45, 45, 255]);
    let white = Rgba([255u8, 255, 255, 255]);
    let mut put = |x: i32, y: i32, color: Rgba<u8>| {
        if x >= 0 && y >= 0 && x < w && y < h {
            image.put_pixel(x as u32, y as u32, color);
        }
    };
    // Draw a white outline first, then a thinner red core, leaving a small center
    // gap so the exact target pixel stays visible.
    for &(color, half) in &[(white, 2i32), (red, 1i32)] {
        for d in gap..=arm {
            for t in -half..=half {
                put(cx - d, cy + t, color);
                put(cx + d, cy + t, color);
                put(cx + t, cy - d, color);
                put(cx + t, cy + d, color);
            }
        }
    }
}

fn draw_drag_trail(image: &mut xcap::image::RgbaImage, fx: i32, fy: i32, tx: i32, ty: i32) {
    use xcap::image::Rgba;
    let w = image.width() as i32;
    let h = image.height() as i32;
    let red = Rgba([255u8, 45, 45, 255]);
    let white = Rgba([255u8, 255, 255, 255]);
    let mut put = |x: i32, y: i32, color: Rgba<u8>| {
        if x >= 0 && y >= 0 && x < w && y < h {
            image.put_pixel(x as u32, y as u32, color);
        }
    };
    let mut draw_point = |x: f64, y: f64, half: i32, color: Rgba<u8>| {
        let px = x.round() as i32;
        let py = y.round() as i32;
        for dy in -half..=half {
            for dx in -half..=half {
                put(px + dx, py + dy, color);
            }
        }
    };

    let dx = (tx - fx) as f64;
    let dy = (ty - fy) as f64;
    let steps = ((dx.abs().max(dy.abs())).ceil() as i32).max(1);

    // Match the click indicator width: 5px white outline, 3px red core.
    for step in 0..=steps {
        let progress = step as f64 / steps as f64;
        let x = fx as f64 + dx * progress;
        let y = fy as f64 + dy * progress;
        draw_point(x, y, 2, white);
    }
    for step in 0..=steps {
        let progress = step as f64 / steps as f64;
        let x = fx as f64 + dx * progress;
        let y = fy as f64 + dy * progress;
        draw_point(x, y, 1, red);
    }

    // Small arrowhead makes the drag direction visible without adding the
    // click-specific crosshair.
    let length = (dx * dx + dy * dy).sqrt();
    if length >= 8.0 {
        let ux = dx / length;
        let uy = dy / length;
        let perp_x = -uy;
        let perp_y = ux;
        let head = 14.0_f64;
        let spread = 7.0_f64;
        let wing_ends = [
            (
                tx as f64 - ux * head + perp_x * spread,
                ty as f64 - uy * head + perp_y * spread,
            ),
            (
                tx as f64 - ux * head - perp_x * spread,
                ty as f64 - uy * head - perp_y * spread,
            ),
        ];
        for (ex, ey) in wing_ends {
            let wing_dx = ex - tx as f64;
            let wing_dy = ey - ty as f64;
            let wing_steps = ((wing_dx.abs().max(wing_dy.abs())).ceil() as i32).max(1);
            for step in 0..=wing_steps {
                let progress = step as f64 / wing_steps as f64;
                draw_point(
                    tx as f64 + wing_dx * progress,
                    ty as f64 + wing_dy * progress,
                    1,
                    red,
                );
            }
        }
    }
}

#[tauri::command]
async fn capture_region(
    window: WebviewWindow,
    region: Region,
    operation_id: Option<String>,
    round_id: Option<String>,
    capture_kind: Option<String>,
    tool_step: Option<u32>,
    marker_x: Option<i32>,
    marker_y: Option<i32>,
    marker_from_x: Option<i32>,
    marker_from_y: Option<i32>,
    captures: State<'_, CaptureState>,
) -> Result<CapturedImage, String> {
    let started = Instant::now();
    *captures.frame.lock().map_err(|_| "截图状态不可用")? = None;
    let capture_context = json!({
        "operationId": operation_id,
        "roundId": round_id,
        "captureKind": capture_kind.as_deref().unwrap_or("unspecified"),
        "toolStep": tool_step,
        "region": region_metadata(region),
    });
    log_event("debug", "capture.started", capture_context.clone());
    if let Err(error) = monitor_for_region(region) {
        log_event(
            "error",
            "capture.validation_failed",
            json!({
                "error": error,
                "operationId": operation_id,
                "roundId": round_id,
                "captureKind": capture_kind,
                "toolStep": tool_step,
                "region": region_metadata(region),
            }),
        );
        return Err(error);
    }
    let presentation = hide_window_for_background_action(&window)?;
    if presentation.hidden_for_action {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let capture_result = (|| {
        let foreground = desktop::foreground_window();
        let mut image = desktop::capture_region(region)?;
        let stable_foreground = foreground != 0 && foreground == desktop::foreground_window();
        if let (Some(mx), Some(my)) = (marker_x, marker_y) {
            let cx = ((region.width.saturating_sub(1) as f64) * mx as f64 / 1000.0).round() as i32;
            let cy = ((region.height.saturating_sub(1) as f64) * my as f64 / 1000.0).round() as i32;
            if let (Some(fx), Some(fy)) = (marker_from_x, marker_from_y) {
                let start_x =
                    ((region.width.saturating_sub(1) as f64) * fx as f64 / 1000.0).round() as i32;
                let start_y =
                    ((region.height.saturating_sub(1) as f64) * fy as f64 / 1000.0).round() as i32;
                draw_drag_trail(&mut image, start_x, start_y, cx, cy);
            } else {
                draw_crosshair(&mut image, cx, cy);
            }
        }
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .map_err(|error| error.to_string())?;
        let frame_id = format!("frame-{}", uuid::Uuid::new_v4());
        *captures.frame.lock().map_err(|_| "截图状态不可用")? = Some(CapturedFrame {
            id: frame_id.clone(),
            region,
            foreground: if stable_foreground { foreground } else { 0 },
            created: Instant::now(),
        });
        Ok::<_, String>(CapturedImage {
            data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
            frame_id,
        })
    })();

    restore_window_after_background_action(&window, presentation, false);
    match &capture_result {
        Ok(image) => log_event(
            "info",
            "capture.completed",
            json!({
                "durationMs": started.elapsed().as_millis(),
                "encodedBytes": image.data_url.len(),
                "operationId": operation_id,
                "roundId": round_id,
                "captureKind": capture_kind,
                "toolStep": tool_step,
                "region": region_metadata(region),
            }),
        ),
        Err(error) => log_event(
            "error",
            "capture.failed",
            json!({
                "durationMs": started.elapsed().as_millis(),
                "error": error,
                "operationId": operation_id,
                "roundId": round_id,
                "captureKind": capture_kind,
                "toolStep": tool_step,
                "region": region_metadata(region),
            }),
        ),
    }
    capture_result
}

fn normalized_coordinate(value: Option<i32>, name: &str) -> Result<i32, String> {
    let value = value.ok_or_else(|| format!("缺少坐标 {name}"))?;
    if !(0..=1000).contains(&value) {
        return Err(format!("坐标 {name} 超出 0..1000 范围"));
    }
    Ok(value)
}

#[tauri::command]
async fn execute_input_action(
    window: WebviewWindow,
    region: Region,
    action: InputAction,
    operation_id: String,
    round_id: String,
    tool_step: u32,
    tool_call_id: String,
    frame_id: Option<String>,
    state: State<'_, OperationState>,
    captures: State<'_, CaptureState>,
) -> Result<String, String> {
    let started = Instant::now();
    let action_type = action.action_type.clone();
    let action_data = json!({
        "type": action_type,
        "x": action.x,
        "y": action.y,
        "clicks": action.clicks,
        "fromX": action.from_x,
        "fromY": action.from_y,
        "toX": action.to_x,
        "toY": action.to_y,
        "textLength": action.text.as_ref().map(|text| text.chars().count()),
        "intervalMs": action.interval_ms,
        "submit": action.submit,
        "keys": action.keys.as_ref(),
        "holdMs": action.hold_ms,
        "operationId": operation_id,
        "roundId": round_id,
        "toolStep": tool_step,
        "toolCallId": tool_call_id,
        "region": region_metadata(region),
    });
    log_event("info", "input.started", action_data.clone());
    if let Err(error) = monitor_for_region(region) {
        log_event(
            "error",
            "input.validation_failed",
            json!({ "error": error, "action": action_data }),
        );
        return Err(error);
    }
    let cancelled = state.flag(&operation_id)?;
    if cancelled.load(Ordering::SeqCst) {
        return Err("本轮操作已取消".into());
    }
    let keyboard_action = matches!(
        action.action_type.as_str(),
        "keyboard_type" | "keyboard_press"
    );
    if !keyboard_action {
        *captures.frame.lock().map_err(|_| "截图状态不可用")? = None;
    }
    let presentation = hide_window_for_background_action(&window)?;
    if presentation.hidden_for_action {
        tokio::time::sleep(Duration::from_millis(120)).await;
    }

    let target_foreground = if keyboard_action {
        match captures.take_keyboard_target(
            frame_id.as_deref(),
            region,
            desktop::foreground_window(),
        ) {
            Ok(target) => Some(target),
            Err(error) => {
                cancelled.store(true, Ordering::SeqCst);
                restore_window_after_background_action(&window, presentation, false);
                log_event(
                    "warn",
                    "input.validation_failed",
                    json!({ "error": error, "action": action_data }),
                );
                return Err(error);
            }
        }
    } else {
        None
    };

    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut enigo = Enigo::new(&EnigoSettings::default()).map_err(|error| error.to_string())?;
        let is_cancelled = || cancelled.load(Ordering::SeqCst);
        let keyboard_cancelled = || {
            keyboard_should_stop(
                &cancelled,
                target_foreground.unwrap_or(0),
                desktop::foreground_window(),
                desktop::keyboard_emergency_stop_pressed(),
            )
        };

        if cancelled.load(Ordering::SeqCst) {
            return Err("本轮操作已取消".into());
        }

        match action.action_type.as_str() {
            "click" => {
                let x = normalized_coordinate(action.x, "x")?;
                let y = normalized_coordinate(action.y, "y")?;
                let (px, py) = screen_point(region, x, y);
                let clicks = action.clicks.unwrap_or(1);
                desktop::click(&mut enigo, (px, py), Button::Left, clicks, &is_cancelled)?;
            }
            "drag" => {
                let from_x = normalized_coordinate(action.from_x, "fromX")?;
                let from_y = normalized_coordinate(action.from_y, "fromY")?;
                let to_x = normalized_coordinate(action.to_x, "toX")?;
                let to_y = normalized_coordinate(action.to_y, "toY")?;
                let duration = 500_u64;
                let (start_x, start_y) = screen_point(region, from_x, from_y);
                let (end_x, end_y) = screen_point(region, to_x, to_y);
                desktop::drag(
                    &mut enigo,
                    (start_x, start_y),
                    (end_x, end_y),
                    Button::Left,
                    duration,
                    &is_cancelled,
                )?;
            }
            "hover" => {
                let x = normalized_coordinate(action.x, "x")?;
                let y = normalized_coordinate(action.y, "y")?;
                let (px, py) = screen_point(region, x, y);
                desktop::move_mouse(&mut enigo, px, py)?;
            }
            "keyboard_type" => {
                let text = action.text.as_deref().ok_or("keyboard_type 缺少 text")?;
                let interval = action.interval_ms.unwrap_or(0);
                let count = desktop::type_text(&mut enigo, text, interval, &keyboard_cancelled)?;
                if action.submit.unwrap_or(false) {
                    desktop::press_keys(&mut enigo, &[Key::Return], 30, &keyboard_cancelled)?;
                }
                log_event(
                    "info",
                    "keyboard.text_dispatched",
                    json!({ "characters": count, "submitted": action.submit.unwrap_or(false) }),
                );
            }
            "keyboard_press" => {
                let key_names = action.keys.as_ref().ok_or("keyboard_press 缺少 keys")?;
                let hold = action.hold_ms.unwrap_or(50);
                let keys = key_names
                    .iter()
                    .map(|key| desktop::parse_key(key))
                    .collect::<desktop::Result<Vec<_>>>()?;
                if keys.contains(&desktop::parse_key("F8")?) {
                    return Err("F8 已保留为键盘输入急停键，不能由模型发送".into());
                }
                for (index, key) in keys.iter().enumerate() {
                    if keys[..index].contains(key) {
                        return Err("keyboard_press 不允许重复按键".into());
                    }
                }
                desktop::press_keys(&mut enigo, &keys, hold, &keyboard_cancelled)?;
            }
            other => return Err(format!("不支持的输入动作：{other}")),
        }
        Ok::<_, String>(())
    })
    .await
    .map_err(|error| error.to_string());

    tokio::time::sleep(Duration::from_millis(180)).await;
    restore_window_after_background_action(&window, presentation, false);
    let result = result?
        .map_err(|error| {
            if error.starts_with("Operation cancelled") {
                "本轮操作已取消".to_string()
            } else {
                error
            }
        })
        .map(|_| {
            format!(
            "{action_type} input event dispatched inside selected region; effect on target not verified"
        )
        });
    match &result {
        Ok(_) => log_event(
            "info",
            "input.completed",
            json!({
                "type": action_type,
                "durationMs": started.elapsed().as_millis(),
                "operationId": operation_id,
                "roundId": round_id,
                "toolStep": tool_step,
                "toolCallId": tool_call_id,
            }),
        ),
        Err(error) => log_event(
            "error",
            "input.failed",
            json!({
                "type": action_type,
                "durationMs": started.elapsed().as_millis(),
                "error": error,
                "operationId": operation_id,
                "roundId": round_id,
                "toolStep": tool_step,
                "toolCallId": tool_call_id,
            }),
        ),
    }
    result
}

fn completion_url(api_url: &str) -> String {
    let trimmed = api_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/chat/completions")
    }
}

fn api_http_client() -> &'static reqwest::Client {
    API_HTTP_CLIENT.get_or_init(reqwest::Client::new)
}

fn chat_payload_metrics(body: &Value) -> (usize, usize, usize, usize) {
    let request_bytes = serde_json::to_vec(body)
        .map(|payload| payload.len())
        .unwrap_or(0);
    let mut image_count = 0;
    let mut image_payload_chars = 0;
    let mut text_chars = 0;

    if let Some(messages) = body.get("messages").and_then(Value::as_array) {
        for message in messages {
            match message.get("content") {
                Some(Value::String(text)) => text_chars += text.chars().count(),
                Some(Value::Array(parts)) => {
                    for part in parts {
                        match part.get("type").and_then(Value::as_str) {
                            Some("text") => {
                                text_chars += part
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .map(|text| text.chars().count())
                                    .unwrap_or(0);
                            }
                            Some("image_url") => {
                                image_count += 1;
                                image_payload_chars += part
                                    .pointer("/image_url/url")
                                    .and_then(Value::as_str)
                                    .map(str::len)
                                    .unwrap_or(0);
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    (request_bytes, image_count, image_payload_chars, text_chars)
}

fn reqwest_error_metadata(error: &reqwest::Error, stage: &str) -> Value {
    let mut causes = Vec::new();
    let mut source = error.source();
    while let Some(cause) = source {
        causes.push(cause.to_string());
        if causes.len() >= 8 {
            break;
        }
        source = cause.source();
    }

    json!({
        "stage": stage,
        "message": error.to_string(),
        "debug": format!("{error:?}"),
        "isConnect": error.is_connect(),
        "isTimeout": error.is_timeout(),
        "isRequest": error.is_request(),
        "isBody": error.is_body(),
        "isDecode": error.is_decode(),
        "causes": causes,
    })
}

fn extract_api_error_detail(text: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(text).ok()?;
    if let Some(message) = value.pointer("/error/message").and_then(Value::as_str) {
        return Some(message.to_string());
    }
    if let Some(message) = value.get("message").and_then(Value::as_str) {
        return Some(message.to_string());
    }
    if let Some(detail) = value.get("detail") {
        match detail {
            Value::String(text) => return Some(text.clone()),
            Value::Array(items) => {
                let parts: Vec<String> = items
                    .iter()
                    .filter_map(|item| {
                        item.as_str()
                            .map(str::to_string)
                            .or_else(|| item.get("msg").and_then(Value::as_str).map(str::to_string))
                    })
                    .collect();
                if !parts.is_empty() {
                    return Some(parts.join("; "));
                }
            }
            _ => {}
        }
    }
    match value.get("error") {
        Some(Value::String(text)) => return Some(text.clone()),
        Some(Value::Object(_)) => {
            if let Some(msg) = value.pointer("/error/error").and_then(Value::as_str) {
                return Some(msg.to_string());
            }
        }
        _ => {}
    }
    None
}

#[tauri::command]
async fn request_chat_completion(
    api_url: String,
    api_key: String,
    body: Value,
    operation_id: String,
    round_id: String,
    request_index: u32,
    attempt: u8,
    tools_allowed: bool,
    timeout_seconds: u64,
    state: State<'_, OperationState>,
) -> Result<Value, String> {
    let started = Instant::now();
    if api_url.trim().is_empty() || api_key.trim().is_empty() {
        log_event(
            "warn",
            "api.configuration_missing",
            json!({
                "operationId": operation_id,
                "roundId": round_id,
                "requestIndex": request_index,
                "attempt": attempt,
            }),
        );
        return Err("API URL 和 API Key 不能为空".into());
    }
    let cancelled = state.flag(&operation_id)?;
    let timeout_seconds = timeout_seconds.clamp(5, 300);

    let endpoint = completion_url(&api_url);
    let endpoint_host = reqwest::Url::parse(&endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "invalid-url".into());
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let message_count = body
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let (request_bytes, image_count, image_payload_chars, request_text_chars) =
        chat_payload_metrics(&body);
    let reasoning_effort = body
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .map(str::to_string);
    let parallel_tool_calls = body.get("parallel_tool_calls").and_then(Value::as_bool);
    let request_fields = body
        .as_object()
        .map(|object| {
            let mut fields = object.keys().cloned().collect::<Vec<_>>();
            fields.sort();
            fields
        })
        .unwrap_or_default();
    log_event(
        "info",
        "api.request_started",
        json!({
            "host": endpoint_host,
            "model": model,
            "messageCount": message_count,
            "requestBytes": request_bytes,
            "imageCount": image_count,
            "imagePayloadChars": image_payload_chars,
            "requestTextChars": request_text_chars,
            "operationId": operation_id,
            "roundId": round_id,
            "requestIndex": request_index,
            "attempt": attempt,
            "toolsAllowed": tools_allowed,
            "reasoningEffort": reasoning_effort,
            "reasoningEffortPresent": reasoning_effort.is_some(),
            "parallelToolCalls": parallel_tool_calls,
            "requestFields": request_fields,
        }),
    );

    let request = async {
        let response = api_http_client()
            .post(endpoint)
            .bearer_auth(api_key.trim())
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                (
                    format!("网络请求失败：{error}"),
                    reqwest_error_metadata(&error, "send"),
                )
            })?;
        let status = response.status();
        let text = response.text().await.map_err(|error| {
            (
                format!("读取模型响应失败：{error}"),
                reqwest_error_metadata(&error, "response_body"),
            )
        })?;
        Ok::<_, (String, Value)>((status, text))
    };
    let wait_for_cancel = async {
        while !cancelled.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let (status, text) = tokio::select! {
        _ = wait_for_cancel => {
            log_event(
                "warn",
                "api.request_cancelled",
                json!({
                    "durationMs": started.elapsed().as_millis(),
                    "host": endpoint_host,
                    "operationId": operation_id,
                    "roundId": round_id,
                    "requestIndex": request_index,
                    "attempt": attempt,
                }),
            );
            return Err("本轮请求已取消".into());
        },
        result = tokio::time::timeout(Duration::from_secs(timeout_seconds), request) => {
            match result {
                Ok(Ok(response)) => response,
                Ok(Err((error, diagnostics))) => {
                    log_event(
                        "error",
                        "api.network_failed",
                        json!({
                            "durationMs": started.elapsed().as_millis(),
                            "error": error,
                            "diagnostics": diagnostics,
                            "host": endpoint_host,
                            "operationId": operation_id,
                            "roundId": round_id,
                            "requestIndex": request_index,
                            "attempt": attempt,
                        }),
                    );
                    return Err(error);
                }
                Err(_) => {
                    log_event(
                        "warn",
                        "api.request_timed_out",
                        json!({
                            "durationMs": timeout_seconds * 1000,
                            "host": endpoint_host,
                            "operationId": operation_id,
                            "roundId": round_id,
                            "requestIndex": request_index,
                            "attempt": attempt,
                        }),
                    );
                    return Err(format!("模型请求超时（{timeout_seconds} 秒）"));
                }
            }
        }
    };

    if !status.is_success() {
        let detail = extract_api_error_detail(&text).unwrap_or(text);
        let error_detail = detail.clone();
        log_event(
            "error",
            "api.response_failed",
            json!({
                "durationMs": started.elapsed().as_millis(),
                "status": status.as_u16(),
                "host": endpoint_host,
                "operationId": operation_id,
                "roundId": round_id,
                "requestIndex": request_index,
                "attempt": attempt,
                "error": error_detail,
                "reasoningEffort": reasoning_effort,
            }),
        );
        return Err(format!("模型接口返回 {}：{}", status.as_u16(), detail));
    }

    let parsed: Result<Value, String> =
        serde_json::from_str(&text).map_err(|error| format!("模型响应不是有效 JSON：{error}"));
    match &parsed {
        Ok(value) => log_event(
            "info",
            "api.request_completed",
            json!({
                "durationMs": started.elapsed().as_millis(),
                "status": status.as_u16(),
                "responseBytes": text.len(),
                "toolCallCount": value
                    .pointer("/choices/0/message/tool_calls")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or_default(),
                "responseModel": value.get("model").and_then(Value::as_str),
                "finishReason": value.pointer("/choices/0/finish_reason").and_then(Value::as_str),
                "assistantContentChars": value
                    .pointer("/choices/0/message/content")
                    .and_then(Value::as_str)
                    .map(|content| content.chars().count()),
                "reasoningContentChars": value
                    .pointer("/choices/0/message/reasoning_content")
                    .and_then(Value::as_str)
                    .map(|content| content.chars().count()),
                "promptTokens": value.pointer("/usage/prompt_tokens").and_then(Value::as_u64),
                "completionTokens": value.pointer("/usage/completion_tokens").and_then(Value::as_u64),
                "reasoningTokens": value
                    .pointer("/usage/completion_tokens_details/reasoning_tokens")
                    .and_then(Value::as_u64),
                "host": endpoint_host,
                "operationId": operation_id,
                "roundId": round_id,
                "requestIndex": request_index,
                "attempt": attempt,
                "toolsAllowed": tools_allowed,
                "reasoningEffort": reasoning_effort,
            }),
        ),
        Err(error) => log_event(
            "error",
            "api.response_parse_failed",
            json!({
                "durationMs": started.elapsed().as_millis(),
                "status": status.as_u16(),
                "responseBytes": text.len(),
                "error": error,
                "host": endpoint_host,
                "operationId": operation_id,
                "roundId": round_id,
                "requestIndex": request_index,
                "attempt": attempt,
                "reasoningEffort": reasoning_effort,
            }),
        ),
    }
    parsed
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(RegionSelectionState::default())
        .manage(OperationState::default())
        .manage(CaptureState::default())
        .manage(WindowLayoutState::default())
        .setup(|app| {
            let logger = app
                .path()
                .app_log_dir()
                .map_err(|error| error.to_string())
                .and_then(AppLogger::new);
            match logger {
                Ok(logger) => {
                    let path = logger.path.clone();
                    let _ = APP_LOGGER.set(logger);
                    log_event(
                        "info",
                        "app.started",
                        json!({
                            "version": env!("CARGO_PKG_VERSION"),
                            "logFile": path.to_string_lossy(),
                        }),
                    );
                }
                Err(error) => eprintln!("failed to initialize app logger: {error}"),
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            client_log,
            enter_mini_mode,
            exit_mini_mode,
            begin_operation,
            cancel_operation,
            finish_operation,
            select_region,
            complete_region_selection,
            cancel_region_selection,
            capture_region,
            execute_input_action,
            request_chat_completion,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run VLM_Screenshot_Action");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn captured_frame() -> CapturedFrame {
        CapturedFrame {
            id: "current-frame".into(),
            region: Region {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            foreground: 42,
            created: Instant::now(),
        }
    }

    #[test]
    fn keyboard_requires_same_frame_region_and_foreground() {
        let frame = captured_frame();
        let region = frame.region;
        let state = CaptureState {
            frame: Mutex::new(Some(frame)),
        };
        assert!(state.take_keyboard_target(None, region, 42).is_err());
        assert!(state
            .take_keyboard_target(Some("old-frame"), region, 42)
            .is_err());
        assert!(state
            .take_keyboard_target(Some("current-frame"), Region { x: 10, ..region }, 42)
            .is_err());
        let error = state
            .take_keyboard_target(Some("current-frame"), region, 99)
            .unwrap_err();
        assert!(error.contains("中断"));
        assert!(state
            .take_keyboard_target(Some("current-frame"), region, 0)
            .is_err());
        assert_eq!(
            state.take_keyboard_target(Some("current-frame"), region, 42),
            Ok(42)
        );
        assert!(state
            .take_keyboard_target(Some("current-frame"), region, 42)
            .is_err());
    }

    #[test]
    fn keyboard_rejects_expired_or_unstable_capture() {
        for frame in [
            CapturedFrame {
                created: Instant::now() - Duration::from_secs(301),
                ..captured_frame()
            },
            CapturedFrame {
                foreground: 0,
                ..captured_frame()
            },
        ] {
            let region = frame.region;
            let state = CaptureState {
                frame: Mutex::new(Some(frame)),
            };
            assert!(state
                .take_keyboard_target(Some("current-frame"), region, 42)
                .is_err());
        }
    }

    #[test]
    fn keyboard_stop_latches_after_f8_or_focus_change() {
        for (foreground, emergency_stop) in [(42, true), (99, false), (0, false)] {
            let flag = AtomicBool::new(false);
            assert!(!keyboard_should_stop(&flag, 42, 42, false));
            assert!(keyboard_should_stop(&flag, 42, foreground, emergency_stop));
            // Releasing F8 or switching back must not resume the current operation.
            assert!(keyboard_should_stop(&flag, 42, 42, false));
        }
        assert!(keyboard_should_stop(&AtomicBool::new(true), 42, 42, false));
    }

    #[test]
    fn capture_response_uses_frontend_field_names() {
        let image = CapturedImage {
            data_url: "data:image/png;base64,test".into(),
            frame_id: "id".into(),
        };
        let value = serde_json::to_value(image).unwrap();
        assert_eq!(value["frameId"], "id");
        assert_eq!(value["dataUrl"], "data:image/png;base64,test");
    }

    #[test]
    fn maps_selection_from_actual_window_origin() {
        let region = selected_screen_region(
            SelectionContext {
                monitor_x: 0,
                monitor_y: 0,
                monitor_width: 1920,
                monitor_height: 1080,
            },
            PhysicalPosition::new(0, 96),
            PhysicalSize::new(1920, 984),
            1536.0,
            787.2,
            100.0,
            2.0,
            1200.0,
            700.0,
        )
        .unwrap();

        assert_eq!(region.x, 125);
        assert_eq!(region.y, 99);
        assert_eq!(region.width, 1500);
        assert_eq!(region.height, 875);
    }

    #[test]
    fn maps_mixed_dpi_secondary_monitor_coordinates() {
        let region = selected_screen_region(
            SelectionContext {
                monitor_x: -2560,
                monitor_y: 180,
                monitor_width: 2560,
                monitor_height: 1440,
            },
            PhysicalPosition::new(-2560, 180),
            PhysicalSize::new(2560, 1440),
            1706.666_666,
            960.0,
            10.0,
            20.0,
            1000.0,
            800.0,
        )
        .unwrap();

        assert_eq!(region.x, -2545);
        assert_eq!(region.y, 210);
        assert_eq!(region.width, 1500);
        assert_eq!(region.height, 1200);
    }

    #[test]
    fn counts_chat_payload_without_logging_content() {
        let body = json!({
            "messages": [
                { "role": "system", "content": "system" },
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "frame" },
                        {
                            "type": "image_url",
                            "image_url": { "url": "data:image/png;base64,AAAA" }
                        }
                    ]
                }
            ]
        });

        let (request_bytes, image_count, image_payload_chars, text_chars) =
            chat_payload_metrics(&body);

        assert!(request_bytes > 0);
        assert_eq!(image_count, 1);
        assert_eq!(image_payload_chars, 26);
        assert_eq!(text_chars, 11);
    }
}
