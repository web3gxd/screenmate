//! A per-pixel-alpha, click-through monitor glow. Windows must confirm that
//! every strip is excluded from screen capture before any strip is shown.
//! Its own Win32 message loop keeps animation and privacy state independent of
//! whether the main window is visible.
use std::{ffi::c_void, ptr, sync::mpsc, thread, time::{Duration, Instant}};
use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE};

type Hwnd = isize;
type Handle = isize;

#[repr(C)]
struct WindowClass {
    style: u32,
    proc: Option<unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize>,
    class_extra: i32, window_extra: i32, instance: Handle, icon: Handle,
    cursor: Handle, background: Handle, menu_name: *const u16, class_name: *const u16,
}
#[repr(C)]
struct OsVersion { size: u32, major: u32, minor: u32, build: u32, platform: u32, service_pack: [u16; 128] }
#[repr(C)]
#[derive(Clone, Copy)]
struct Point { x: i32, y: i32 }
#[repr(C)]
struct Size { cx: i32, cy: i32 }
#[repr(C)]
struct BitmapInfoHeader { size: u32, width: i32, height: i32, planes: u16, bit_count: u16, compression: u32, image_size: u32, xppm: i32, yppm: i32, colors_used: u32, colors_important: u32 }
#[repr(C)]
struct BitmapInfo { header: BitmapInfoHeader, colors: [u32; 1] }
#[repr(C)]
struct BlendFunction { operation: u8, flags: u8, alpha: u8, format: u8 }

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(class: *const WindowClass) -> u16;
    fn UnregisterClassW(name: *const u16, instance: Handle) -> i32;
    fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: Hwnd, menu: Handle, instance: Handle, param: *mut c_void) -> Hwnd;
    fn DestroyWindow(window: Hwnd) -> i32;
    fn DefWindowProcW(window: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize;
    fn SetWindowDisplayAffinity(window: Hwnd, affinity: u32) -> i32;
    fn GetWindowDisplayAffinity(window: Hwnd, affinity: *mut u32) -> i32;
    fn SetWindowPos(window: Hwnd, after: Hwnd, x: i32, y: i32, w: i32, h: i32, flags: u32) -> i32;
    fn ShowWindow(window: Hwnd, command: i32) -> i32;
    fn GetDC(window: Hwnd) -> Handle;
    fn ReleaseDC(window: Hwnd, dc: Handle) -> i32;
    fn UpdateLayeredWindow(window: Hwnd, dst_dc: Handle, dst: *const Point, size: *const Size, src_dc: Handle, src: *const Point, key: u32, blend: *const BlendFunction, flags: u32) -> i32;
}
#[link(name = "kernel32")]
extern "system" { fn GetModuleHandleW(name: *const u16) -> Handle; }
#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(dc: Handle) -> Handle;
    fn DeleteDC(dc: Handle) -> i32;
    fn CreateDIBSection(dc: Handle, info: *const BitmapInfo, usage: u32, bits: *mut *mut c_void, section: Handle, offset: u32) -> Handle;
    fn SelectObject(dc: Handle, object: Handle) -> Handle;
    fn DeleteObject(object: Handle) -> i32;
}
#[link(name = "ntdll")]
extern "system" { fn RtlGetVersion(version: *mut OsVersion) -> i32; }

unsafe extern "system" fn window_proc(hwnd: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize {
    if message == 0x0084 { return -1; } // WM_NCHITTEST => HTTRANSPARENT
    if message == 0x0021 { return 3; } // WM_MOUSEACTIVATE => MA_NOACTIVATE
    DefWindowProcW(hwnd, message, wparam, lparam)
}

/// A top-down 32-bit BGRA DIB. Pixel values are premultiplied by alpha.
struct Surface { dc: Handle, bitmap: Handle, old: Handle, bits: *mut u8, width: i32, height: i32 }
impl Surface {
    unsafe fn new(width: i32, height: i32) -> Result<Self, String> {
        let dc = CreateCompatibleDC(0);
        if dc == 0 { return Err("Не удалось создать слой свечения.".into()); }
        let info = BitmapInfo { header: BitmapInfoHeader {
            size: 40, width, height: -height, planes: 1, bit_count: 32, compression: 0,
            image_size: 0, xppm: 0, yppm: 0, colors_used: 0, colors_important: 0,
        }, colors: [0] };
        let mut bits = ptr::null_mut();
        let bitmap = CreateDIBSection(dc, &info, 0, &mut bits, 0, 0);
        if bitmap == 0 || bits.is_null() {
            if bitmap != 0 { DeleteObject(bitmap); }
            DeleteDC(dc);
            return Err("Не удалось выделить память для свечения.".into());
        }
        let old = SelectObject(dc, bitmap);
        if old == 0 || old == -1 {
            DeleteObject(bitmap); DeleteDC(dc);
            return Err("Не удалось подготовить слой свечения.".into());
        }
        Ok(Self { dc, bitmap, old, bits: bits.cast(), width, height })
    }
    fn pixels(&mut self) -> &mut [u8] {
        // DIB has exactly width*height 32-bit pixels (32-bit rows are aligned).
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.width as usize * self.height as usize * 4) }
    }
}
impl Drop for Surface {
    fn drop(&mut self) { unsafe { SelectObject(self.dc, self.old); DeleteObject(self.bitmap); DeleteDC(self.dc); } }
}

#[derive(Clone, Copy)]
struct Strip { x: i32, y: i32, w: i32, h: i32 }

type Rect = (i32, i32, u32, u32);
type Source = Box<dyn FnMut() -> Option<(Rect, [u8; 3])> + Send>;

/// A short, interruptible opacity transition. Capture authorization is not
/// controlled here; this state affects only the decorative Windows surfaces.
struct Fade { from: f32, target: f32, began: Instant }
impl Fade {
    fn new(now: Instant) -> Self { Self { from: 0.0, target: 0.0, began: now } }
    fn value(&self, now: Instant) -> f32 {
        let t = (now.saturating_duration_since(self.began).as_secs_f32() / 0.34).clamp(0.0, 1.0);
        let smooth = t*t*(3.0-2.0*t);
        self.from + (self.target-self.from)*smooth
    }
    fn aim(&mut self, target: f32, now: Instant) {
        if self.target != target { self.from = self.value(now); self.target = target; self.began = now; }
    }
}

fn glow_width(width: i32, height: i32) -> i32 {
    let smaller = width.min(height);
    ((smaller as f32 * 0.06).round() as i32).clamp(2, 384).min((smaller-2)/2)
}

fn frame_interval(visible: bool, fading: bool, pixels: u64) -> Duration {
    if !visible { Duration::from_millis(120) }
    else if fading && pixels >= 8_000_000 { Duration::from_millis(50) }
    else if fading { Duration::from_millis(40) }
    else { Duration::from_millis(80) }
}

/// Switch screens in two stages: the current border first fades completely,
/// then the requested border fades in. Reversing the request midway simply
/// reverses the current fade without an opacity jump.
fn resolve_transition(current: Option<Rect>, requested: Option<Rect>, opacity: f32) -> (Option<Rect>, f32) {
    match (current, requested) {
        (Some(old), Some(new)) if old != new && opacity > 0.0 => (Some(old), 0.0),
        (Some(old), None) if opacity > 0.0 => (Some(old), 0.0),
        (_, Some(new)) => (Some(new), 1.0),
        (_, None) => (None, 0.0),
    }
}

struct Renderer {
    windows: [Hwnd; 4],
    instance: Handle,
    class_name: Vec<u16>,
    last: Option<(i32, i32, u32, u32)>,
    strips: [Strip; 4],
    surfaces: Vec<Surface>,
    started: Instant,
    last_frame: Option<Instant>,
    accent: [u8; 3],
    fade: Fade,
}

impl Renderer {
    fn new() -> Result<Self, String> {
        unsafe {
            let mut version: OsVersion = std::mem::zeroed();
            version.size = std::mem::size_of::<OsVersion>() as u32;
            if RtlGetVersion(&mut version) != 0 || version.major < 10 || (version.major == 10 && version.build < 19041) {
                return Err("Свечение недоступно: нужна Windows 10 версии 2004 или новее.".into());
            }
            let class_name: Vec<u16> = "ScreenmateCaptureOverlay\0".encode_utf16().collect();
            let instance = GetModuleHandleW(ptr::null());
            if instance == 0 { return Err("Не удалось создать свечение экрана.".into()); }
            let class = WindowClass { style: 0, proc: Some(window_proc), class_extra: 0, window_extra: 0, instance, icon: 0, cursor: 0, background: 0, menu_name: ptr::null(), class_name: class_name.as_ptr() };
            if RegisterClassW(&class) == 0 { return Err("Не удалось зарегистрировать свечение экрана.".into()); }
            let empty = Strip { x: 0, y: 0, w: 0, h: 0 };
            let now = Instant::now();
            let mut result = Self { windows: [0; 4], instance, class_name, last: None, strips: [empty; 4], surfaces: Vec::new(), started: now, last_frame: None, accent: [90,197,235], fade: Fade::new(now) };
            for i in 0..4 {
                // NOACTIVATE | LAYERED | TOOLWINDOW | TRANSPARENT | TOPMOST.
                let hwnd = CreateWindowExW(0x080800a8, result.class_name.as_ptr(), ptr::null(), 0x80000000, 0, 0, 1, 1, 0, 0, instance, ptr::null_mut());
                if hwnd == 0 { return Err("Не удалось создать свечение экрана.".into()); }
                result.windows[i] = hwnd;
                let mut actual = 0u32;
                if SetWindowDisplayAffinity(hwnd, 0x11) == 0 || GetWindowDisplayAffinity(hwnd, &mut actual) == 0 || actual != 0x11 {
                    return Err("Свечение отключено: Windows не подтвердила исключение из захвата.".into());
                }
            }
            Ok(result)
        }
    }

    fn hide(&mut self) {
        for &window in &self.windows { if window != 0 { unsafe { ShowWindow(window, 0); } } }
        self.last = None;
        self.last_frame = None;
    }

    /// Monitor coordinates are physical pixels. None fades only the visuals;
    /// the application has already stopped frame delivery independently.
    fn update(&mut self, desired: Option<(Rect, [u8; 3])>) -> Result<(), String> {
        let now = Instant::now();
        let requested = desired.map(|(rect, _)| rect);
        let (rect, target) = resolve_transition(self.last, requested, self.fade.value(now));
        self.fade.aim(target, now);
        if let Some((requested_rect, accent)) = desired {
            if rect == Some(requested_rect) { self.accent = accent; }
        }
        let Some((x, y, width, height)) = rect else {
            if self.last.is_some() { self.hide(); }
            return Ok(());
        };
        let opacity = self.fade.value(now);
        let dims = i32::try_from(width).ok().zip(i32::try_from(height).ok());
        let (w, h) = match dims {
            Some((w,h)) if w >= 8 && h >= 8 && x.checked_add(w).is_some() && y.checked_add(h).is_some() => (w,h),
            _ => { self.hide(); return Err("Недопустимые координаты монитора.".into()); }
        };
        let geometry_changed = self.last != Some((x, y, width, height));
        if geometry_changed {
            self.hide();
            // Proportional to the physical monitor, so a 4K display keeps
            // exactly the same apparent composition as a 1080p display.
            let thickness = glow_width(w, h);
            self.strips = [
                Strip { x, y, w, h: thickness },
                Strip { x, y: y+h-thickness, w, h: thickness },
                Strip { x, y: y+thickness, w: thickness, h: h-2*thickness },
                Strip { x: x+w-thickness, y: y+thickness, w: thickness, h: h-2*thickness },
            ];
            let mut surfaces = Vec::with_capacity(4);
            for s in self.strips {
                // Degenerate tiny monitors cannot support a useful four-sided glow.
                if s.w <= 0 || s.h <= 0 { self.hide(); return Err("Монитор слишком мал для свечения.".into()); }
                match unsafe { Surface::new(s.w, s.h) } {
                    Ok(surface) => surfaces.push(surface),
                    Err(error) => { self.hide(); return Err(error); }
                }
            }
            self.surfaces = surfaces;
        }
        if !geometry_changed && self.last_frame.is_some_and(|previous| now.duration_since(previous) < Duration::from_millis(40)) { return Ok(()); }
        let seconds = self.started.elapsed().as_secs_f32();
        let screen_dc = unsafe { GetDC(0) };
        if screen_dc == 0 { self.hide(); return Err("Не удалось обновить свечение экрана.".into()); }
        let mut failure = false;
        for i in 0..4 {
            let strip = self.strips[i];
            render(&mut self.surfaces[i], i, w, h, seconds, self.accent);
            let pos = Point { x: strip.x, y: strip.y };
            let size = Size { cx: strip.w, cy: strip.h };
            let source = Point { x: 0, y: 0 };
            let blend = BlendFunction { operation: 0, flags: 0, alpha: (opacity.clamp(0.0,1.0)*255.0).round() as u8, format: 1 };
            let updated = unsafe { UpdateLayeredWindow(self.windows[i], screen_dc, &pos, &size, self.surfaces[i].dc, &source, 0, &blend, 2) };
            if updated == 0 { failure = true; break; }
        }
        unsafe { ReleaseDC(0, screen_dc); }
        if failure { self.hide(); return Err("Не удалось обновить свечение экрана.".into()); }
        if geometry_changed {
            for (window, strip) in self.windows.iter().zip(self.strips) {
                // Only show after rendering, and never take focus.
                if unsafe { SetWindowPos(*window, -1, strip.x, strip.y, strip.w, strip.h, 0x0050) } == 0 {
                    self.hide(); return Err("Не удалось показать свечение экрана.".into());
                }
            }
        }
        self.last = Some((x, y, width, height));
        self.last_frame = Some(now);
        Ok(())
    }
}

/// One of four narrow strips, with a soft inward falloff and slowly drifting
/// colour. Only the strip-sized memory is updated, never a fullscreen image.
fn render(surface: &mut Surface, edge: usize, monitor_w: i32, monitor_h: i32, seconds: f32, accent: [u8; 3]) {
    let w = surface.width as usize;
    let h = surface.height as usize;
    let horizontal = edge < 2;
    let along_len = if horizontal { w } else { h };
    let depth = if horizontal { h } else { w };
    let phase = seconds * 0.055;
    let mut colors = Vec::with_capacity(along_len);
    for along in 0..along_len {
        let xf = along as f32 / monitor_w as f32;
        let yf = (along + depth) as f32 / monitor_h as f32;
        let u = match edge { 0 => xf, 1 => 3.0-xf, 2 => 4.0-yf, _ => 1.0+yf };
        colors.push(palette(u, phase, accent));
    }
    let mut falloff = Vec::with_capacity(depth);
    for d in 0..depth {
        let t = d as f32 / depth.max(1) as f32;
        let shape = 1.0 - t*t*(3.0-2.0*t);
        // Broad soft bloom, avoiding a bright solid one-pixel border.
        falloff.push((shape * 0.45 * 255.0) as u8);
    }
    let unit = monitor_w.min(monitor_h).max(1) as u64;
    let x_samples: Vec<u32> = (0..w).map(|px| {
        let absolute_x = if edge == 3 { monitor_w as usize-w+px } else { px };
        ((absolute_x as u64 * 1080) / unit) as u32
    }).collect();
    let pixels = surface.pixels();
    for py in 0..h {
        let absolute_y = match edge { 0 => py, 1 => monitor_h as usize-h+py, _ => py+depth };
        let sample_y = ((absolute_y as u64 * 1080) / unit) as u32;
        let corner_y = absolute_y as f32 / monitor_h as f32;
        let left_color = palette(4.0-corner_y, phase, accent);
        let right_color = palette(1.0+corner_y, phase, accent);
        for px in 0..w {
            let along = if horizontal { px } else { py };
            let d = match edge { 0 => py, 1 => h-1-py, 2 => px, _ => w-1-px };
            let (mut r,mut g,mut b,strength) = colors[along];
            let mut intensity = falloff[d] as f32 * strength;
            if horizontal {
                // Blend the two soft fields in a corner instead of picking a
                // nearest edge. This removes the hard diagonal join while
                // still reaching the exact side-strip value at its boundary.
                let left = px;
                let right = w-1-px;
                let side = if left < depth { Some((left,left_color)) } else if right < depth { Some((right,right_color)) } else { None };
                if let Some((side_depth, (sr,sg,sb,side_strength))) = side {
                    let side_intensity = falloff[side_depth] as f32 * side_strength;
                    let total = intensity + side_intensity;
                    if total > 0.0 {
                        let weight = side_intensity / total;
                        r += (sr-r)*weight;
                        g += (sg-g)*weight;
                        b += (sb-b)*weight;
                    }
                    intensity = total - intensity*side_intensity/255.0;
                }
            }
            // Stable, subtle film grain rather than flickering static; variation
            // is strongest at the edge and disappears with the soft falloff.
            // Map noise into a resolution-independent 1080-unit coordinate
            // system. At 2× resolution, details grow 2× rather than shrinking.
            let hash = x_samples[px].wrapping_mul(0x1f123bb5) ^ sample_y.wrapping_mul(0x5f356495);
            let mixed = (hash ^ (hash >> 13)).wrapping_mul(0x85ebca6b);
            let noise = (mixed >> 27) as f32 / 31.0;
            let sparkle = if (mixed & 0x1fff) == 0 { 1.20 } else { 1.0 };
            let alpha = ((intensity * (0.91 + noise*0.18) * sparkle).min(255.0)) as u8;
            let index = (py*w+px)*4;
            pixels[index] = (b * alpha as f32 / 255.0) as u8;
            pixels[index+1] = (g * alpha as f32 / 255.0) as u8;
            pixels[index+2] = (r * alpha as f32 / 255.0) as u8;
            pixels[index+3] = alpha;
        }
    }
}

fn palette(u: f32, phase: f32, accent: [u8; 3]) -> (f32, f32, f32, f32) {
    let a = ((u * 0.25 + phase) * std::f32::consts::TAU).sin() * 0.5 + 0.5;
    let b = ((u * 0.50 - phase * 0.7) * std::f32::consts::TAU).sin() * 0.5 + 0.5;
    // Electric blue -> ice blue -> mint, with a cool violet undertone.
    let r = 28.0 + 66.0*b;
    let g = 92.0 + 145.0*a + 12.0*b;
    let blue = 244.0 - 80.0*a + 10.0*b;
    // Two wide, independent slow-moving pools of light travel around the
    // perimeter. Their motion is continuous across the start/end seam.
    let glow_a = (((u*0.25 + phase*0.7)*std::f32::consts::TAU).sin()*0.5+0.5).powi(3);
    let glow_b = (((u*0.50 - phase*0.45+0.28)*std::f32::consts::TAU).sin()*0.5+0.5).powi(4);
    let strength = 0.44 + 0.56 * glow_a.max(glow_b*0.85);
    // The user's accent colours the same palette used by the rest of the UI.
    let blend = |base: f32, component: u8| (base*0.68 + component as f32*0.32).min(255.0);
    (blend(r,accent[0]), blend(g,accent[1]), blend(blue,accent[2]), strength)
}

impl Drop for Renderer {
    fn drop(&mut self) { unsafe {
        for &window in &self.windows { if window != 0 { DestroyWindow(window); } }
        self.surfaces.clear();
        UnregisterClassW(self.class_name.as_ptr(), self.instance);
    } }
}

/// Owns an independent Win32 thread. The source should be non-blocking and
/// return a snapshot of the authorised monitor and selected accent, or None
/// when paused/unavailable. Dropping this handle stops and joins that thread.
pub struct Overlay {
    stop: mpsc::Sender<()>,
    error: mpsc::Receiver<String>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Overlay {
    pub fn new_with_source(mut source: Source) -> Result<Self, String> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (stop_tx, stop_rx) = mpsc::channel();
        let (error_tx, error_rx) = mpsc::channel();
        let worker = thread::Builder::new().name("screenmate-glow".into()).spawn(move || {
            let mut renderer = match Renderer::new() {
                Ok(renderer) => { let _ = ready_tx.send(Ok(())); renderer },
                Err(error) => { let _ = ready_tx.send(Err(error)); return; }
            };
            loop {
                let frame_start = Instant::now();
                // Overlay windows belong to this thread; pump their messages
                // even when the main application is hidden in the tray.
                unsafe {
                    let mut message: MSG = std::mem::zeroed();
                    while PeekMessageW(&mut message, 0, 0, 0, PM_REMOVE) != 0 {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                if let Err(error) = renderer.update(source()) {
                    let _ = error_tx.send(error);
                    break;
                }
                let pixels = renderer.last.map(|(_,_,w,h)| w as u64*h as u64).unwrap_or(0);
                let fading = (renderer.fade.value(Instant::now()) - renderer.fade.target).abs() > 0.002;
                // Slow colour drift is smooth at 12.5 fps. Spend extra frames
                // only on short opacity transitions; use 20 fps at 4K to cap CPU.
                let interval = frame_interval(renderer.last.is_some(), fading, pixels);
                let delay = interval.saturating_sub(frame_start.elapsed());
                match stop_rx.recv_timeout(delay) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        }).map_err(|error| format!("Не удалось создать поток свечения: {error}"))?;
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self { stop: stop_tx, error: error_rx, worker: Some(worker) }),
            Ok(Err(error)) => { let _ = worker.join(); Err(error) },
            Err(_) => { let _ = stop_tx.send(()); Err("Не удалось запустить свечение экрана.".into()) }
        }
    }
    pub fn poll_error(&mut self) -> Option<String> { self.error.try_recv().ok() }
}
impl Drop for Overlay {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_wraps_smoothly_around_screen() {
        let a = palette(0.0, 0.12, [90,197,235]);
        let b = palette(4.0, 0.12, [90,197,235]);
        for (x,y) in [a.0,a.1,a.2,a.3].into_iter().zip([b.0,b.1,b.2,b.3]) {
            assert!((x-y).abs() < 0.01);
        }
    }

    #[test]
    fn pixels_fade_and_use_premultiplied_alpha() {
        let mut strip = unsafe { Surface::new(512, 40).unwrap() };
        render(&mut strip, 0, 512, 300, 0.0, [90,197,235]);
        let pixels = strip.pixels();
        let at = |x:usize,y:usize| (y*512+x)*4;
        let outer = pixels[at(256,0)+3];
        let middle = pixels[at(256,20)+3];
        let inner = pixels[at(256,39)+3];
        assert!(outer > middle && middle > inner && inner < 2);
        for pixel in pixels.chunks_exact(4) {
            assert!(pixel[0] <= pixel[3] && pixel[1] <= pixel[3] && pixel[2] <= pixel[3]);
        }
    }

    #[test]
    fn width_stays_proportional_across_resolutions() {
        for (w,h) in [(1920,1080), (2560,1440), (3840,2160)] {
            assert!((glow_width(w,h) as f32 / h as f32 - 0.06).abs() < 0.001);
        }
    }

    #[test]
    fn selected_accent_colours_the_glow() {
        let cool = palette(0.8, 0.2, [70,160,245]);
        let mint = palette(0.8, 0.2, [70,230,150]);
        assert!(mint.1 > cool.1 && mint.2 < cool.2);
    }

    #[test]
    fn active_transitions_have_a_higher_frame_rate_than_idle_glow() {
        assert_eq!(frame_interval(true, false, 1920*1080), Duration::from_millis(80));
        assert_eq!(frame_interval(true, true, 1920*1080), Duration::from_millis(40));
        assert_eq!(frame_interval(true, true, 3840*2160), Duration::from_millis(50));
        assert_eq!(frame_interval(false, false, 0), Duration::from_millis(120));
    }

    #[test]
    fn fade_is_smooth_and_can_reverse_without_a_jump() {
        let start = Instant::now();
        let mut fade = Fade::new(start);
        fade.aim(1.0, start);
        assert_eq!(fade.value(start), 0.0);
        let mid = start + Duration::from_millis(170);
        let opacity = fade.value(mid);
        assert!((opacity-0.5).abs() < 0.001);
        fade.aim(0.0, mid);
        assert!((fade.value(mid)-opacity).abs() < 0.001);
        assert_eq!(fade.value(mid + Duration::from_millis(340)), 0.0);
    }

    #[test]
    fn monitor_change_fades_old_before_showing_new_and_can_reverse() {
        let old=(0,0,1920,1080);
        let new=(1920,0,2560,1440);
        assert_eq!(resolve_transition(Some(old),Some(new),1.0),(Some(old),0.0));
        assert_eq!(resolve_transition(Some(old),Some(new),0.5),(Some(old),0.0));
        assert_eq!(resolve_transition(Some(old),Some(new),0.0),(Some(new),1.0));
        assert_eq!(resolve_transition(Some(old),Some(old),0.5),(Some(old),1.0));
        assert_eq!(resolve_transition(Some(old),None,0.0),(None,0.0));
    }
}
