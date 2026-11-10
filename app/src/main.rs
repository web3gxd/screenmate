#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod tray;
mod overlay;
mod ui;
mod icons;
mod access;
mod privacy;
mod activity;
mod i18n;
mod autostart;

use std::{collections::VecDeque, fs, io::{Cursor, Read}, path::PathBuf, sync::{Arc, Mutex}, thread, time::Duration};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::Local;
use eframe::egui;
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, DynamicImage, Rgba, RgbaImage};
use screenshots::Screen;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tiny_http::{Header, Response, Server, StatusCode};
use windows_sys::Win32::{Foundation::{BOOL, HWND, LPARAM, RECT}, System::{StationsAndDesktops::{CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, UOI_NAME}, Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION}}, UI::{Input::KeyboardAndMouse::{RegisterHotKey, MOD_CONTROL, MOD_SHIFT}, WindowsAndMessaging::{EnumWindows, GetClassNameW, GetMessageW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, MSG, WM_HOTKEY}}};
use windows_sys::Win32::Foundation::CloseHandle;

#[derive(Clone, Deserialize)]
struct Config { blocked_titles: Vec<String>, blocked_processes: Vec<String>, #[serde(skip)] user_blocked_processes: Vec<String>, ring_size: usize, jpeg_quality: u8, max_width: u32 }
#[derive(Clone, Serialize, PartialEq, Eq)]
struct Window { id: i64, title: String, process: String, x: i32, y: i32, width: i32, height: i32 }
#[derive(Clone, Serialize)]
struct Monitor { #[serde(serialize_with = "id_as_string")] id: u32, name: String, x: i32, y: i32, width: u32, height: u32, primary: bool }
#[derive(Clone)]
struct Frame { jpg: Vec<u8>, width: u32, height: u32, hash: u64, timestamp: String }
#[derive(Clone, Serialize)]
struct Message { text: String, timestamp: String }
struct State { config: Config, paused: bool, monitor: Option<u32>, window: Option<i64>, region: Option<[i32; 4]>, frames: VecDeque<Frame>, last_look: Option<u64>, inbox: Vec<Message>, accent: [u8; 3], glow_enabled: bool, glow_preference:bool, minimize_to_tray: bool, language:i18n::Language, error: String, capture_activity:Arc<Mutex<activity::CaptureActivity>> }
type Shared = Arc<Mutex<State>>;

fn dir() -> Result<PathBuf, String> { Ok(PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("Нет LOCALAPPDATA")?).join("Screenmate")) }
fn now() -> String { Local::now().to_rfc3339() }
fn secure_session_directory(path: &std::path::Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| e.to_string())?;
    // Restrict access before creating the bearer-token file. Use the SID, not a localized account name.
    let who = std::process::Command::new("whoami").args(["/user", "/fo", "csv", "/nh"]).output().map_err(|e| e.to_string())?;
    if !who.status.success() { return Err("Не удалось определить SID пользователя".into()); }
    let line = String::from_utf8_lossy(&who.stdout);
    let sid = line.split(',').nth(1).unwrap_or("").trim().trim_matches('"');
    if !sid.starts_with("S-1-") || !sid.chars().all(|c| c.is_ascii_digit() || c == 'S' || c == '-') { return Err("Не удалось определить SID пользователя".into()); }
    let acl = format!("*{}:(OI)(CI)F", sid);
    let reset = std::process::Command::new("icacls").arg(path).arg("/reset").output().map_err(|e|e.to_string())?;
    if !reset.status.success() { return Err("Не удалось сбросить ACL папки сеанса".into()); }
    let status = std::process::Command::new("icacls").arg(path).args(["/inheritance:r", "/grant:r", &acl]).output().map_err(|e| e.to_string())?;
    if !status.status.success() { return Err("Не удалось ограничить доступ к папке сеанса".into()); }
    Ok(())
}
fn id_as_string<S: serde::Serializer>(id: &u32, serializer: S) -> Result<S::Ok, S::Error> { serializer.serialize_str(&id.to_string()) }
fn monitors() -> Result<Vec<Monitor>, String> { Ok(Screen::all().map_err(|e| e.to_string())?.into_iter().map(|s| { let d=s.display_info; Monitor { id:d.id, name:format!("Монитор {}",d.id), x:d.x, y:d.y, width:d.width, height:d.height, primary:d.is_primary } }).collect()) }
fn visible_window_rect_available(ok:bool,r:&RECT)->Result<bool,String> {
    if !ok {return Err("Не удалось определить границы видимого окна".into());}
    let width=r.right.checked_sub(r.left).ok_or("Некорректные границы видимого окна")?;
    let height=r.bottom.checked_sub(r.top).ok_or("Некорректные границы видимого окна")?;
    Ok(width>0 && height>0)
}
unsafe extern "system" fn collect_window(hwnd: HWND, param: LPARAM) -> BOOL {
    if IsWindowVisible(hwnd) == 0 { return 1; }
    let mut r: RECT = std::mem::zeroed();
    match visible_window_rect_available(GetWindowRect(hwnd, &mut r)!=0,&r) {Ok(true)=>{},Ok(false)=>return 1,Err(_)=>return 0}
    let mut pid = 0; GetWindowThreadProcessId(hwnd, &mut pid);
    // GetWindowText on our own GUI thread synchronously sends WM_GETTEXT. The
    // capture worker holds the shared lock, so that would deadlock against UI.
    if pid == std::process::id() {
        let mut class=[0u16;128]; let n=GetClassNameW(hwnd,class.as_mut_ptr(),class.len() as i32).max(0) as usize;
        if String::from_utf16_lossy(&class[..n]) == "ScreenmateCaptureOverlay" { return 1; }
        let v = &mut *(param as *mut Vec<Window>);
        v.push(Window { id:hwnd as i64, title:"[Скринмейт]".into(), process:String::new(), x:r.left, y:r.top, width:r.right-r.left, height:r.bottom-r.top });
        return 1;
    }
    let len = GetWindowTextLengthW(hwnd).max(0) as usize;
    let mut buf = vec![0u16; len + 1];
    let count = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
    let title = String::from_utf16_lossy(&buf[..count]);
    let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    let process = if h == 0 { String::new() } else {
        let mut p = vec![0u16; 32768]; let mut n = p.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, p.as_mut_ptr(), &mut n); CloseHandle(h);
        if ok == 0 { String::new() } else { String::from_utf16_lossy(&p[..n as usize]).rsplit(['\\','/']).next().unwrap_or("").to_string() }
    };
    let v = &mut *(param as *mut Vec<Window>);
    v.push(Window { id:hwnd as i64, title, process, x:r.left, y:r.top, width:r.right-r.left, height:r.bottom-r.top });
    1
}
fn sort_window_snapshot(windows:&mut [Window]) { windows.sort_unstable_by_key(|w|w.id); }
fn windows() -> Result<Vec<Window>, String> {
    let mut result = Vec::new();
    if unsafe { EnumWindows(Some(collect_window), &mut result as *mut _ as LPARAM) } == 0 { return Err("Не удалось проверить открытые окна".into()); }
    // Z-order is not relevant for masking: all protected rectangles are
    // obscured. Keep every field in the comparison, but normalize order.
    sort_window_snapshot(&mut result);
    Ok(result)
}
// A locked workstation switches away from the interactive Default desktop; fail closed on errors.
fn desktop_available() -> bool { unsafe {
    let desktop = OpenInputDesktop(0, 0, 1); // DESKTOP_READOBJECTS
    if desktop == 0 { return false; }
    let mut data = [0u16; 256]; let mut needed = 0u32;
    let ok = GetUserObjectInformationW(desktop, UOI_NAME, data.as_mut_ptr() as *mut _, (data.len()*2) as u32, &mut needed);
    CloseDesktop(desktop);
    if ok == 0 { return false; }
    let n = data.iter().position(|x| *x == 0).unwrap_or(data.len());
    String::from_utf16_lossy(&data[..n]).eq_ignore_ascii_case("Default")
} }
fn blocked(w: &Window, cfg: &Config) -> bool {
    let title = w.title.to_lowercase(); let process = w.process.to_lowercase();
    // A window with an inaccessible process is conservatively masked.
    process.is_empty() || cfg.blocked_titles.iter().any(|s| !s.is_empty() && title.contains(&s.to_lowercase())) || cfg.blocked_processes.iter().any(|s| !s.is_empty() && process.contains(&s.to_lowercase())) || cfg.user_blocked_processes.iter().any(|s|process==s.to_lowercase())
}
fn mask_protected_windows(image:&mut RgbaImage,origin_x:i32,origin_y:i32,windows:&[Window],config:&Config) {
    for w in windows {
        if !blocked(w,config) {continue;}
        let x0=(w.x as i64-origin_x as i64).clamp(0,image.width() as i64) as u32;
        let y0=(w.y as i64-origin_y as i64).clamp(0,image.height() as i64) as u32;
        let x1=(w.x as i64+w.width as i64-origin_x as i64).clamp(0,image.width() as i64) as u32;
        let y1=(w.y as i64+w.height as i64-origin_y as i64).clamp(0,image.height() as i64) as u32;
        for y in y0..y1 {for x in x0..x1 {image.put_pixel(x,y,Rgba([0,0,0,255]));}}
    }
}
fn hash(img: &RgbaImage) -> u64 {
    let small = image::imageops::resize(img, 9, 8, FilterType::Triangle); let mut hash=0u64;
    for y in 0..8 { for x in 0..8 { let a=small.get_pixel(x,y); let b=small.get_pixel(x+1,y); let lum=|p:&Rgba<u8>| 299*p[0] as u32+587*p[1] as u32+114*p[2] as u32; hash=(hash<<1) | (lum(a)>lum(b)) as u64; } } hash
}
fn capture(state: &mut State, monitor_override: Option<u32>, region_override: Option<[i32;4]>) -> Result<Frame, String> {
    if state.paused { return Err("Пауза".into()); }
    if !desktop_available() { state.paused=true; state.frames.clear(); return Err("Экран заблокирован или недоступен".into()); }
    let screens = Screen::all().map_err(|e| e.to_string())?;
    let id = monitor_override.or(state.monitor);
    let screen = screens.into_iter().find(|s| id.map_or(s.display_info.is_primary, |id| s.display_info.id == id)).ok_or("Монитор недоступен")?;
    let d = screen.display_info;
    let before = windows()?;
    let mut image = screen.capture().map_err(|e| e.to_string())?;
    let after = windows()?;
    if before != after { return Err("Окна изменились во время захвата; повторите запрос".into()); }
    if !desktop_available() { state.paused=true; state.frames.clear(); return Err("Экран заблокирован".into()); }
    // Windows coordinates and the capture surface must share physical pixels.
    if image.width() != d.width || image.height() != d.height { return Err("Не удалось безопасно сопоставить координаты монитора и кадра".into()); }
    mask_protected_windows(&mut image,d.x,d.y,&after,&state.config);
    let selected = if let Some(region) = region_override.or(state.region) { Some(region) } else if let Some(id) = state.window {
        let w = after.iter().find(|w| w.id == id).ok_or("Выбранное окно недоступно")?;
        Some([w.x-d.x,w.y-d.y,w.width,w.height])
    } else { None };
    if let Some([x,y,w,h]) = selected {
        if x < 0 || y < 0 || w <= 0 || h <= 0 || x as u64 + w as u64 > image.width() as u64 || y as u64 + h as u64 > image.height() as u64 { return Err("Область находится за пределами монитора".into()); }
        image = image::imageops::crop_imm(&image,x as u32,y as u32,w as u32,h as u32).to_image();
    }
    if image.width() > state.config.max_width { let h=((image.height() as u64*state.config.max_width as u64)/image.width() as u64).max(1) as u32; image=image::imageops::resize(&image,state.config.max_width,h,FilterType::Triangle); }
    let frame_hash=hash(&image); let mut bytes=Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut bytes,state.config.jpeg_quality).encode_image(&DynamicImage::ImageRgba8(image.clone())).map_err(|e| e.to_string())?;
    let frame=Frame { jpg:bytes.into_inner(), width:image.width(), height:image.height(), hash:frame_hash, timestamp:now() };
    state.frames.push_back(frame.clone()); while state.frames.len() > state.config.ring_size { state.frames.pop_front(); }
    Ok(frame)
}
fn region(value: &Value) -> Result<Option<[i32;4]>, String> {
    if value.is_null() { return Ok(None); }
    let read=|key| value.get(key).and_then(Value::as_i64).and_then(|x|i32::try_from(x).ok()).ok_or_else(||format!("Неверная область: {key}"));
    Ok(Some([read("x")?,read("y")?,read("width")?,read("height")?]))
}
/// Wrap only an explicit MCP request, never the periodic background capture.
/// The injected capture closure also permits privacy-safe tests with fake data.
fn explicit_capture<F>(state:&mut State,monitor:Option<u32>,region:Option<[i32;4]>,desktop_ok:bool,run:F)->Result<Frame,String>
where F:FnOnce(&mut State,Option<u32>,Option<[i32;4]>)->Result<Frame,String> {
    let target=monitor.or(state.monitor);let indicator=state.capture_activity.clone();
    if desktop_ok && !state.paused && state.glow_enabled && state.error.is_empty() {
        if let Ok(mut a)=indicator.lock(){a.record(std::time::Instant::now(),target);}
    }
    match run(state,monitor,region) {
        Ok(frame)=>{
            // A successful fresh capture supersedes an earlier transient
            // enumeration error; do not silently deliver a frame without the
            // visual indication merely because the prior status was stale.
            state.error.clear();
            if desktop_ok && !state.paused && state.glow_enabled {
                if let Ok(mut a)=indicator.lock(){a.record(std::time::Instant::now(),target);}
            }
            Ok(frame)
        }
        Err(error)=>{if let Ok(mut a)=indicator.lock(){a.clear();}Err(error)}
    }
}
fn rpc(state: &Shared, req: Value) -> Result<Value,String> {
    let method=req.get("method").and_then(Value::as_str).ok_or("Нет метода")?; let params=req.get("params").cloned().unwrap_or(json!({}));
    match method {
        "look" => { let monitor=match params.get("monitor") { None | Some(Value::Null)=>None, Some(v)=>Some(v.as_str().ok_or("Неверный монитор")?.parse::<u32>().map_err(|_|"Неверный монитор")?) }; let reg=region(params.get("region").unwrap_or(&Value::Null))?;
            let mut s=state.lock().map_err(|_|"Ошибка состояния")?;
            let frame=explicit_capture(&mut s,monitor,reg,desktop_available(),capture)?;
            let changed=s.last_look.map_or(true,|h| (h ^ frame.hash).count_ones() > 4); s.last_look=Some(frame.hash);
            Ok(json!({"image_base64": BASE64.encode(&frame.jpg), "mime_type":"image/jpeg", "timestamp":frame.timestamp, "width":frame.width, "height":frame.height,"changed":changed})) }
        "list_windows" => { let s=state.lock().map_err(|_|"Ошибка состояния")?; let mut ws=windows()?; for w in &mut ws { if blocked(w,&s.config) { w.title="[скрыто]".into(); w.process="[скрыто]".into(); } } Ok(json!({"monitors":monitors()?,"windows":ws})) },
        "inbox" => { let mut s=state.lock().map_err(|_|"Ошибка состояния")?; Ok(json!({"messages":std::mem::take(&mut s.inbox)})) }
        "say" => Err("Вывод ответов в приложении отключён".into()),
        _ => Err("Неизвестный метод".into())
    }
}
fn serve(state: Shared, path: PathBuf) -> Result<(),String> {
    let server=Server::http("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port=server.server_addr().to_ip().ok_or("Нет локального адреса")?.port();
    let token=format!("{}{}",uuid::Uuid::new_v4().simple(),uuid::Uuid::new_v4().simple());
    fs::write(path.join("session.json"),serde_json::to_vec(&json!({"port":port,"token":token})).unwrap()).map_err(|e| e.to_string())?;
    thread::spawn(move || { for mut request in server.incoming_requests() {
        let authorized=request.headers().iter().any(|h| h.field.equiv("Authorization") && h.value.as_str() == format!("Bearer {token}"));
        let response = if !authorized { json!({"ok":false,"error":"unauthorized"}) } else if request.method().as_str() != "POST" || request.url() != "/rpc" { json!({"ok":false,"error":"not found"}) } else {
            let mut input=Vec::new(); let read=request.as_reader().take(131_073).read_to_end(&mut input);
            if read.is_err() || input.len() > 131_072 { json!({"ok":false,"error":"request too large"}) } else { match serde_json::from_slice::<Value>(&input).map_err(|e|e.to_string()).and_then(|v|rpc(&state,v)) { Ok(v)=>json!({"ok":true,"result":v}), Err(e)=>json!({"ok":false,"error":e}) } }
        };
        let body=response.to_string(); let mut out=Response::from_string(body).with_status_code(if authorized {StatusCode(200)} else {StatusCode(401)}); out.add_header(Header::from_bytes("Content-Type","application/json").unwrap()); let _=request.respond(out);
    }}); Ok(())
}
struct ScreenmateApp { state: Shared, region_x:i32, region_y:i32, region_w:i32, region_h:i32, mode:u8, tray:Option<tray::TrayController>, exiting:bool, overlay:Option<overlay::Overlay>, overlay_error:String, autostart_error:Option<String>, brand:egui::TextureHandle, monitor_cache:Vec<Monitor>, monitor_cache_at:std::time::Instant }
fn hotkey(state: Shared) { thread::spawn(move || unsafe {
    if RegisterHotKey(0,1,MOD_CONTROL | MOD_SHIFT,0x50) == 0 { state.lock().unwrap().error="Не удалось зарегистрировать Ctrl+Shift+P".into(); return; }
    let mut msg:MSG=std::mem::zeroed(); while GetMessageW(&mut msg,0,0,0)>0 { if msg.message==WM_HOTKEY { let mut s=state.lock().unwrap(); s.paused=!s.paused; if s.paused {s.frames.clear();} } }
}); }

// A GUI-only developer preview. It is compiled out of release builds, never
// connects to the live instance, and never captures pixels or creates a token.
// Its process-local preference directory is isolated under the system temp dir.
#[cfg(debug_assertions)]
fn run_preview() -> Result<(), String> {
    let temporary=std::env::temp_dir().join(format!("screenmate-preview-{}",std::process::id()));
    fs::create_dir_all(temporary.join("Screenmate")).map_err(|e|e.to_string())?;
    std::env::set_var("LOCALAPPDATA", &temporary);
    unsafe { windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2); }
    let config:Config=serde_yaml::from_str(include_str!("../config.yaml")).map_err(|e|e.to_string())?;
    let (accent,glow_preference,minimize_to_tray,language)=ui::load_appearance();
    let state=Arc::new(Mutex::new(State {config,paused:false,monitor:None,window:None,region:None,frames:VecDeque::new(),last_look:None,inbox:Vec::new(),accent,glow_enabled:glow_preference,glow_preference,minimize_to_tray,language,error:String::new(),capture_activity:Arc::new(Mutex::new(activity::CaptureActivity::default()))}));
    let icon=egui::IconData {rgba:include_bytes!("../assets/icon-32.rgba").to_vec(),width:32,height:32};
    let options=eframe::NativeOptions {viewport:egui::ViewportBuilder::default().with_icon(icon).with_inner_size([560.0,610.0]).with_min_inner_size([420.0,440.0]),..Default::default()};
    eframe::run_native("Скринмейт — предпросмотр",options,Box::new(move |cc| {
        let brand=ui::setup(&cc.egui_ctx);
        Box::new(ScreenmateApp {state,region_x:0,region_y:0,region_w:800,region_h:600,mode:0,tray:None,exiting:false,overlay:None,overlay_error:String::new(),autostart_error:None,brand,monitor_cache:monitors().unwrap_or_default(),monitor_cache_at:std::time::Instant::now()})
    })).map_err(|e|e.to_string())
}

fn run() -> Result<(),String> {
    #[cfg(debug_assertions)]
    if std::env::var_os("SCREENMATE_PREVIEW").as_deref()==Some(std::ffi::OsStr::new("1")) {return run_preview();}
    let (accent,glow_preference,minimize_to_tray,language)=ui::load_appearance();
    let glow_enabled=activity::startup_glow_permission(glow_preference,activity::has_autostart_arg(std::env::args_os().skip(1)));
    // Per-monitor physical coordinates are necessary to mask windows at 125/150% scaling.
    unsafe { windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2); }
    let name:Vec<u16>="Local\\Screenmate.Singleton\0".encode_utf16().collect();
    let mutex=unsafe { windows_sys::Win32::System::Threading::CreateMutexW(std::ptr::null(),0,name.as_ptr()) };
    if mutex == 0 { return Err(language.text("Не удалось создать блокировку экземпляра","Could not create the single-instance lock").into()); }
    if unsafe { windows_sys::Win32::Foundation::GetLastError() } == windows_sys::Win32::Foundation::ERROR_ALREADY_EXISTS { unsafe {CloseHandle(mutex)}; return Err(language.text("Скринмейт уже запущен","Screenmate is already running").into()); }
    let path=dir()?; secure_session_directory(&path)?;
    let session=path.join("session.json"); let _=fs::remove_file(&session);
    let config_path=path.join("config.yaml"); if !config_path.exists() { fs::write(&config_path,include_str!("../config.yaml")).map_err(|e|e.to_string())?; }
    let mut config:Config=serde_yaml::from_slice(&fs::read(&config_path).map_err(|e|e.to_string())?).map_err(|e|format!("{}: {e}",language.text("Ошибка config.yaml","config.yaml error")))?;
    config.user_blocked_processes=privacy::load_user_masks_for(&path.join("blocked-apps.yaml"),language)?;
    if config.ring_size==0 || config.ring_size>100 || config.max_width==0 || config.max_width>7680 || config.jpeg_quality==0 || config.jpeg_quality>100 { return Err(language.text("Неверные ограничения config.yaml","Invalid config.yaml limits").into()); }
    let autostart_error=autostart::initialize(language).err();
    let state=Arc::new(Mutex::new(State { config, paused:false, monitor:None,window:None,region:None,frames:VecDeque::new(),last_look:None,inbox:Vec::new(),accent,glow_enabled,glow_preference,minimize_to_tray,language,error:String::new(),capture_activity:Arc::new(Mutex::new(activity::CaptureActivity::default())) }));
    serve(state.clone(),path)?; hotkey(state.clone());
    let background=state.clone(); thread::spawn(move || loop { thread::sleep(Duration::from_secs(2)); let mut s=match background.lock() { Ok(s)=>s,Err(_)=>break }; if !s.paused { match capture(&mut s,None,None) { Ok(_)=>s.error.clear(), Err(e)=>{ if !desktop_available() {s.paused=true;s.frames.clear();} s.error=e; } } } });
    let icon=egui::IconData {rgba:include_bytes!("../assets/icon-32.rgba").to_vec(),width:32,height:32};
    let options=eframe::NativeOptions { viewport:egui::ViewportBuilder::default().with_icon(icon).with_inner_size([560.0,610.0]).with_min_inner_size([420.0,440.0]),..Default::default() };
    let result=eframe::run_native("Скринмейт",options,Box::new(move |cc| {
        let brand=ui::setup(&cc.egui_ctx);
        let source_state=state.clone();
        let source_activity=state.lock().unwrap().capture_activity.clone();
        let mut snapshot=(true,[90,197,235]);
        let mut cached_id:Option<Option<u32>>=None;
        let mut cached_rect=None;
        let mut last_check=std::time::Instant::now().checked_sub(Duration::from_secs(10)).unwrap_or_else(std::time::Instant::now);
        let source=move || {
            match source_state.try_lock() {
                Ok(s)=>snapshot=(s.paused||!s.error.is_empty()||!s.glow_enabled,s.accent),
                Err(std::sync::TryLockError::WouldBlock)=>{},
                Err(std::sync::TryLockError::Poisoned(_))=>return None,
            }
            if snapshot.0 {if let Ok(mut a)=source_activity.lock(){a.clear();}return None;}
            let target=source_activity.lock().ok().and_then(|a|a.monitor(std::time::Instant::now()))?;
            if cached_id!=Some(target)||last_check.elapsed()>=Duration::from_secs(2) {
                cached_rect=monitors().ok().and_then(|ms|ms.into_iter().find(|m|target.map_or(m.primary,|id|m.id==id))).map(|m|(m.x,m.y,m.width,m.height));
                cached_id=Some(target);last_check=std::time::Instant::now();
            }
            cached_rect.map(|rect|(rect,snapshot.1))
        };
        let (overlay,overlay_error)=match overlay::Overlay::new_with_source(Box::new(source)) {Ok(o)=>(Some(o),String::new()),Err(e)=>(None,e)};
        let tray=tray::TrayController::new(state.clone(),cc.egui_ctx.clone()).ok();
        Box::new(ScreenmateApp {state,region_x:0,region_y:0,region_w:800,region_h:600,mode:0,tray,exiting:false,overlay,overlay_error,autostart_error,brand,monitor_cache:monitors().unwrap_or_default(),monitor_cache_at:std::time::Instant::now()})
    })).map_err(|e|e.to_string());
    let _=fs::remove_file(session); unsafe { CloseHandle(mutex); } result
}
fn main() { if let Err(e)=run() { let text:Vec<u16>=format!("{e}\0").encode_utf16().collect(); let title:Vec<u16>=format!("{}\0",ui::load_appearance().3.text("Скринмейт","Screenmate")).encode_utf16().collect(); unsafe { windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(0,text.as_ptr(),title.as_ptr(),0x10); } } }







#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config { Config { blocked_titles: vec!["банк".into()], blocked_processes: vec!["secret.exe".into()], user_blocked_processes:vec![], ring_size:5,jpeg_quality:70,max_width:1920 } }
    fn window(title: &str,process:&str) -> Window { Window { id:1,title:title.into(),process:process.into(),x:0,y:0,width:10,height:10 } }
    #[test] fn masks_protected_and_unknown_processes() { let c=config(); assert!(blocked(&window("Мой БАНК", "browser.exe"), &c)); assert!(blocked(&window("обычное", "Secret.EXE"), &c)); assert!(blocked(&window("обычное", ""), &c)); assert!(!blocked(&window("обычное", "editor.exe"), &c)); }
    #[test] fn unknown_visible_window_geometry_fails_closed() {
        let mut r=RECT {left:10,top:10,right:20,bottom:20};
        assert!(visible_window_rect_available(false,&r).is_err());
        assert_eq!(visible_window_rect_available(true,&r),Ok(true));
        r.right=r.left;assert_eq!(visible_window_rect_available(true,&r),Ok(false));
        r.left=i32::MIN;r.right=i32::MAX;assert!(visible_window_rect_available(true,&r).is_err());
    }
    #[test] fn masks_intersection_on_synthetic_negative_coordinate_monitor() {
        let mut c=config();c.user_blocked_processes=vec!["finance.exe".into()];
        let original=Rgba([90,130,170,255]);let mut image=RgbaImage::from_pixel(6,4,original);
        let protected=Window {id:1,title:"Счёт".into(),process:"FINANCE.EXE".into(),x:-102,y:11,width:5,height:2};
        let allowed=Window {id:2,title:"Обычное".into(),process:"editor.exe".into(),x:-96,y:10,width:2,height:4};
        mask_protected_windows(&mut image,-100,10,&[protected,allowed],&c);
        for y in 0..4 {for x in 0..6 {
            let expected=if x<3 && (1..3).contains(&y) {Rgba([0,0,0,255])} else {original};
            assert_eq!(*image.get_pixel(x,y),expected,"unexpected pixel at {x},{y}");
        }}
        let cropped=image::imageops::crop_imm(&image,2,1,3,2).to_image();
        assert_eq!(*cropped.get_pixel(0,0),Rgba([0,0,0,255]));assert_eq!(*cropped.get_pixel(1,0),original);
    }
    #[test] fn snapshots_ignore_z_order_but_not_content_or_geometry() {
        let first=window("Первое","a.exe");let mut second=window("Второе","b.exe");second.id=2;
        let mut before=vec![first.clone(),second.clone()];let mut after=vec![second.clone(),first.clone()];
        sort_window_snapshot(&mut before);sort_window_snapshot(&mut after);assert!(before==after);
        after[0].title.push('!');assert!(before!=after);after[0]=first;
        after[1].x+=1;assert!(before!=after);
    }
    #[test] fn rejects_malformed_regions() { assert!(region(&json!({"x":0,"y":0,"width":20})).is_err()); assert!(region(&Value::Null).unwrap().is_none()); assert_eq!(region(&json!({"x":1,"y":2,"width":3,"height":4})).unwrap(),Some([1,2,3,4])); }
    #[test] fn hash_finds_pattern_change() { let flat=RgbaImage::from_pixel(36,32,Rgba([100,100,100,255])); let mut stripe=flat.clone(); for y in 0..32 {for x in 0..18 { stripe.put_pixel(x,y,Rgba([255,255,255,255])); }} assert_ne!(hash(&flat),hash(&stripe)); }
    #[test] fn only_explicit_valid_capture_activates_actual_monitor_and_failures_clear_it() {
        let mut s=State {config:config(),paused:false,monitor:Some(3),window:None,region:None,frames:VecDeque::new(),last_look:None,inbox:Vec::new(),accent:[90,197,235],glow_enabled:true,glow_preference:true,minimize_to_tray:true,language:i18n::Language::Ru,error:String::new(),capture_activity:Arc::new(Mutex::new(activity::CaptureActivity::default()))};
        let fake=|_:&mut State,_:Option<u32>,_:Option<[i32;4]>|Ok(Frame {jpg:vec![],width:1,height:1,hash:0,timestamp:String::new()});
        assert_eq!(s.capture_activity.lock().unwrap().monitor(std::time::Instant::now()),None);
        explicit_capture(&mut s,Some(9),None,true,fake).unwrap();
        assert_eq!(s.capture_activity.lock().unwrap().monitor(std::time::Instant::now()),Some(Some(9)));
        let failed=|_:&mut State,_:Option<u32>,_:Option<[i32;4]>|Err("захват не удался".into());
        assert!(explicit_capture(&mut s,Some(9),None,true,failed).is_err());
        assert_eq!(s.capture_activity.lock().unwrap().monitor(std::time::Instant::now()),None);
        s.glow_enabled=false;explicit_capture(&mut s,None,None,true,fake).unwrap();
        assert_eq!(s.capture_activity.lock().unwrap().monitor(std::time::Instant::now()),None);
        s.glow_enabled=true;s.paused=true;explicit_capture(&mut s,None,None,true,fake).unwrap();
        assert_eq!(s.capture_activity.lock().unwrap().monitor(std::time::Instant::now()),None);
        s.paused=false;explicit_capture(&mut s,None,None,false,fake).unwrap();
        assert_eq!(s.capture_activity.lock().unwrap().monitor(std::time::Instant::now()),None);
        s.error="Предыдущая временная ошибка".into();
        explicit_capture(&mut s,None,None,true,fake).unwrap();
        assert!(s.error.is_empty());
        assert_eq!(s.capture_activity.lock().unwrap().monitor(std::time::Instant::now()),Some(Some(3)));
    }
    #[test] fn autostart_session_does_not_overwrite_saved_manual_preference() {
        // Simulate a saved "off" choice and a Windows Run launch.
        let saved=false;
        let mut s=State {config:config(),paused:false,monitor:None,window:None,region:None,frames:VecDeque::new(),last_look:None,inbox:Vec::new(),accent:[90,197,235],glow_enabled:activity::startup_glow_permission(saved,true),glow_preference:saved,minimize_to_tray:true,language:i18n::Language::Ru,error:String::new(),capture_activity:Arc::new(Mutex::new(activity::CaptureActivity::default()))};
        assert!(s.glow_enabled);assert!(!s.glow_preference);
        assert_eq!(s.capture_activity.lock().unwrap().monitor(std::time::Instant::now()),None,"startup must not cause a glow");
        // Saving any other preference must preserve the earlier explicit off.
        s.accent=[89,216,168];
        let persisted=ui::appearance_value(&s);
        assert_eq!(persisted["glow_enabled"],false);
        let next_saved=persisted["glow_enabled"].as_bool().unwrap();
        assert!(!activity::startup_glow_permission(next_saved,false),"next manual launch must retain off");
        // Explicit MCP capture, not startup permission, starts the indicator.
        let fake=|_:&mut State,_:Option<u32>,_:Option<[i32;4]>|Ok(Frame {jpg:vec![],width:1,height:1,hash:0,timestamp:String::new()});
        explicit_capture(&mut s,None,None,true,fake).unwrap();
        assert_eq!(s.capture_activity.lock().unwrap().monitor(std::time::Instant::now()),Some(None));
    }
    #[test] fn say_does_not_claim_delivery_when_chat_is_hidden() {
        let s=Arc::new(Mutex::new(State {config:config(),paused:false,monitor:None,window:None,region:None,frames:VecDeque::new(),last_look:None,inbox:Vec::new(),accent:[90,197,235],glow_enabled:true,glow_preference:true,minimize_to_tray:true,language:i18n::Language::Ru,error:String::new(),capture_activity:Arc::new(Mutex::new(activity::CaptureActivity::default()))}));
        assert_eq!(rpc(&s,json!({"method":"say","params":{"text":"Привет"}})).unwrap_err(),"Вывод ответов в приложении отключён");
        assert!(s.lock().unwrap().inbox.is_empty());
    }
}





