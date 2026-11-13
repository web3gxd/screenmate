//! Windows notification-area integration with its own Win32 message loop.
#[path = "tray_popup.rs"] mod popup;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
use super::i18n::Language;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayState { Watching, Paused, Error }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayAction { Open, TogglePause, ToggleGlow, NextMonitor, Exit }

struct NativeMenu { open:MenuItem, pause:MenuItem, glow:MenuItem, monitor:MenuItem, exit:MenuItem }
impl NativeMenu {
    fn create(language:Language) -> Result<(Self, Menu), String> {
        let menu=Menu::new();
        let open=MenuItem::new(language.text("Открыть Скринмейт","Open Screenmate"),true,None);
        let pause=MenuItem::new(language.text("Пауза","Pause"),true,None);
        let glow=MenuItem::new(language.text("Выключить свечение","Turn off glow"),true,None);
        let monitor=MenuItem::new(language.text("Следующий монитор","Next monitor"),true,None);
        let exit=MenuItem::new(language.text("Выход","Quit"),true,None);
        menu.append_items(&[&open,&pause,&glow,&monitor,&exit]).map_err(|e|e.to_string())?;
        Ok((Self {open,pause,glow,monitor,exit},menu))
    }
    fn event(&self,id:&tray_icon::menu::MenuId)->Option<TrayAction> {
        if id==self.open.id(){Some(TrayAction::Open)}
        else if id==self.pause.id(){Some(TrayAction::TogglePause)}
        else if id==self.glow.id(){Some(TrayAction::ToggleGlow)}
        else if id==self.monitor.id(){Some(TrayAction::NextMonitor)}
        else if id==self.exit.id(){Some(TrayAction::Exit)} else {None}
    }
}

struct TrayRuntime {
    icon:TrayIcon,
    popup:Option<popup::Popup>,
    fallback:Option<NativeMenu>,
    state:TrayState,
    glow:bool,
    monitor:String,
    accent:[u8;3],
    language:Language,
}
impl TrayRuntime {
    fn new()->Result<Self,String> {
        let popup=popup::Popup::new().ok();
        let mut builder=TrayIconBuilder::new().with_tooltip("Скринмейт — смотрю")
            .with_menu_on_left_click(false).with_icon(make_icon(TrayState::Watching)?);
        let fallback=if popup.is_none(){let (items,menu)=NativeMenu::create(Language::Ru)?;builder=builder.with_menu(Box::new(menu));Some(items)}else{None};
        let icon=builder.build().map_err(|e|e.to_string())?;
        Ok(Self {icon,popup,fallback,state:TrayState::Watching,glow:true,monitor:"Основной монитор".into(),accent:[90,197,235],language:Language::Ru})
    }
    fn set_state(&mut self,state:TrayState)->Result<(),String> {
        if self.state==state{return Ok(());}
        self.icon.set_icon(Some(make_icon(state)?)).map_err(|e|e.to_string())?;
        self.icon.set_tooltip(Some(tooltip(state,self.language))).map_err(|e|e.to_string())?;
        self.state=state;
        self.refresh_menu();
        Ok(())
    }
    /// Synchronize quick actions with the same live state as the main window.
    fn set_details(&mut self,glow_enabled:bool,monitor_label:&str,accent:[u8;3],language:Language) {
        if self.glow==glow_enabled && self.monitor==monitor_label && self.accent==accent && self.language==language{return;}
        self.glow=glow_enabled; self.monitor=monitor_label.to_owned(); self.accent=accent;
        if self.language!=language {self.language=language;let _=self.icon.set_tooltip(Some(tooltip(self.state,language)));}
        self.refresh_menu();
    }
    fn refresh_menu(&mut self) {
        if let Some(p)=&self.popup {p.set_details(self.state,self.glow,&self.monitor,self.accent,self.language);}
        if let Some(m)=&self.fallback {
            let l=self.language;
            m.open.set_text(l.text("Открыть Скринмейт","Open Screenmate"));
            m.pause.set_text(if self.state==TrayState::Paused {l.text("Продолжить захват","Resume capture")} else {l.text("Пауза","Pause")});
            m.glow.set_text(if self.glow {l.text("Выключить свечение","Turn off glow")} else {l.text("Включить свечение","Turn on glow")});
            m.monitor.set_text(l.text("Следующий монитор","Next monitor"));
            m.exit.set_text(l.text("Выход","Quit"));
        }
    }
    fn poll(&mut self)->Option<TrayAction> {
        if let Some(p)=&mut self.popup {if let Some(a)=p.poll(){return Some(a);}}
        while let Ok(event)=MenuEvent::receiver().try_recv() {
            if let Some(action)=self.fallback.as_ref().and_then(|m|m.event(&event.id)){return Some(action);}
        }
        while let Ok(event)=TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click{id,position,button,button_state:MouseButtonState::Up,..}=event {
                if &id!=self.icon.id(){continue;}
                match button {
                    MouseButton::Left=>return Some(TrayAction::Open),
                    MouseButton::Right=>if let Some(p)=&mut self.popup {
                        if p.show(position.x.round() as i32,position.y.round() as i32).is_err() {
                            // On a rare Win32 popup failure retain a functional system menu.
                            if let Ok((items,menu))=NativeMenu::create(self.language){self.icon.set_menu(Some(Box::new(menu)));self.fallback=Some(items);self.popup=None;self.refresh_menu();}
                        }
                    },
                    _=>{}
                }
            }
        }
        None
    }
}

fn tooltip(state:TrayState,language:Language)->&'static str {
    match state {
        TrayState::Watching=>language.text("Скринмейт — смотрю","Screenmate — watching"),
        TrayState::Paused=>language.text("Скринмейт — пауза","Screenmate — paused"),
        TrayState::Error=>language.text("Скринмейт — ничего не вижу","Screenmate — no capture"),
    }
}

#[link(name="user32")]
extern "system" {
    fn FindWindowW(class_name:*const u16,window_name:*const u16)->isize;
    fn GetWindowThreadProcessId(hwnd:isize,pid:*mut u32)->u32;
    fn IsWindowVisible(hwnd:isize)->i32;
    fn ShowWindow(hwnd:isize,command:i32)->i32;
    fn SetForegroundWindow(hwnd:isize)->i32;
}

/// Make a hidden winit window visible before requesting a redraw. A hidden
/// viewport need not receive redraws, so requesting egui repaint alone is not
/// enough to deliver the Open or Exit actions reliably.
fn wake_main_window(activate:bool,ctx:&eframe::egui::Context) {
    unsafe {
        for label in ["Скринмейт\0","Screenmate\0"] {
            let title:Vec<u16>=label.encode_utf16().collect();
            let hwnd=FindWindowW(std::ptr::null(),title.as_ptr());
            let mut pid=0;
            if hwnd!=0 && GetWindowThreadProcessId(hwnd,&mut pid)!=0 && pid==std::process::id() {
                if IsWindowVisible(hwnd)==0 {ShowWindow(hwnd,if activate {5} else {4});}
                if activate {SetForegroundWindow(hwnd);}
                break;
            }
        }
    }
    ctx.request_repaint();
}

pub struct TrayController {
    actions:std::sync::mpsc::Receiver<TrayAction>,
    shutdown:std::sync::mpsc::Sender<()>,
    worker:Option<std::thread::JoinHandle<()>>,
}

/// Apply privacy-relevant quick actions without involving the GUI event loop.
/// The caller holds the shared state mutex and persists appearance if needed.
fn apply_capture_action(action:TrayAction,s:&mut super::State,monitors:&[super::Monitor])->bool {
    match action {
        TrayAction::TogglePause=>{s.paused=!s.paused;if s.paused{s.frames.clear();}true}
        TrayAction::ToggleGlow=>{s.glow_enabled=!s.glow_enabled;s.glow_preference=s.glow_enabled;true}
        TrayAction::NextMonitor=>{
            if monitors.is_empty(){return false;}
            let index=monitors.iter().position(|m|s.monitor.map_or(m.primary,|id|id==m.id)).unwrap_or(0);
            s.monitor=Some(monitors[(index+1)%monitors.len()].id);
            s.window=None;s.region=None;s.last_look=None;true
        }
        TrayAction::Open|TrayAction::Exit=>false,
    }
}

impl TrayController {
    /// Create the tray and its native popup on a dedicated message-loop thread.
    pub fn new(state:super::Shared,ctx:eframe::egui::Context)->Result<Self,String> {
        let (action_tx,actions)=std::sync::mpsc::channel();
        let (shutdown,shutdown_rx)=std::sync::mpsc::channel();
        let (ready_tx,ready_rx)=std::sync::mpsc::sync_channel(1);
        let worker=std::thread::Builder::new().name("screenmate-tray".into()).spawn(move || {
            let mut runtime=match TrayRuntime::new() {Ok(v)=>{let _=ready_tx.send(Ok(()));v},Err(e)=>{let _=ready_tx.send(Err(e));return;}};
            use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE};
            let mut message:MSG=unsafe{std::mem::zeroed()};
            let mut cached_monitors=Vec::new();
            let mut updated=std::time::Instant::now()-std::time::Duration::from_secs(3);
            loop {
                unsafe {while PeekMessageW(&mut message,0,0,0,PM_REMOVE)!=0 {TranslateMessage(&message);DispatchMessageW(&message);}}
                if shutdown_rx.try_recv().is_ok(){break;}
                if updated.elapsed()>=std::time::Duration::from_secs(2) {cached_monitors=super::monitors().unwrap_or_default();updated=std::time::Instant::now();}
                if let Ok(s)=state.try_lock() {
                    let status=if s.paused {TrayState::Paused} else if !s.error.is_empty() {TrayState::Error} else {TrayState::Watching};
                    let l=s.language;
                    let monitor=cached_monitors.iter().position(|m|s.monitor.map_or(m.primary,|id|id==m.id)).map(|i|format!("{} {}",l.text("Монитор","Monitor"),i+1)).unwrap_or_else(||l.text("недоступен","unavailable").into());
                    let _=runtime.set_state(status);runtime.set_details(s.glow_enabled,&monitor,s.accent,l);
                }
                while let Some(action)=runtime.poll() {
                    match action {
                        TrayAction::Open=>{let _=action_tx.send(action);wake_main_window(true,&ctx);}
                        TrayAction::Exit=>{let _=action_tx.send(action);wake_main_window(false,&ctx);}
                        TrayAction::TogglePause|TrayAction::ToggleGlow|TrayAction::NextMonitor=>{
                            let mut changed=false;
                            if let Ok(mut s)=state.lock(){
                                changed=apply_capture_action(action,&mut s,&cached_monitors);
                                if changed && action==TrayAction::ToggleGlow {super::ui::save_appearance(&s);}
                            }
                            if changed {
                                if action==TrayAction::NextMonitor {let _=action_tx.send(action);}
                                ctx.request_repaint();
                            }
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }).map_err(|e|format!("Не удалось запустить трей: {e}"))?;
        match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(()))=>Ok(Self {actions,shutdown,worker:Some(worker)}),
            Ok(Err(e))=>{let _=worker.join();Err(e)},
            Err(_)=>{let _=shutdown.send(());Err("Не удалось запустить трей вовремя".into())}
        }
    }
    pub fn poll(&mut self)->Option<TrayAction> {self.actions.try_recv().ok()}
    // Runtime derives these values from Shared directly, including while the
    // viewport is hidden. Retain these methods for existing GUI call sites.
    pub fn set_state(&mut self,_state:TrayState)->Result<(),String> {Ok(())}
    pub fn set_details(&mut self,_glow:bool,_monitor:&str) {}
}
impl Drop for TrayController {fn drop(&mut self) {let _=self.shutdown.send(());if let Some(worker)=self.worker.take(){let _=worker.join();}}}

// Full-size status glyphs: no tiny badges. Supersampling keeps curves readable
// when Windows displays the icon at both 16 and 32 physical pixels.
fn render_icon(state:TrayState)->Vec<u8> {
    const W:usize=32; const N:usize=4;
    fn inside_segment(x:f32,y:f32,a:(f32,f32),b:(f32,f32),width:f32)->bool {
        let (vx,vy)=(b.0-a.0,b.1-a.1);let t=(((x-a.0)*vx+(y-a.1)*vy)/(vx*vx+vy*vy)).clamp(0.0,1.0);
        (x-(a.0+t*vx)).powi(2)+(y-(a.1+t*vy)).powi(2)<=width.powi(2)
    }
    let mut out=vec![0u8;W*W*4];
    for y in 0..W {for x in 0..W {
        let mut sum=[0u32;4];
        for sy in 0..N {for sx in 0..N {
            let px=x as f32+(sx as f32+0.5)/N as f32;let py=y as f32+(sy as f32+0.5)/N as f32;
            let dx=px-16.0;let dy=py-16.0;let d2=dx*dx+dy*dy;
            let c=match state {
                TrayState::Watching=>{
                    let eye=(dx/12.8).powi(2)+(dy/8.8).powi(2)<=1.0;
                    let pupil=d2<=4.2*4.2;
                    let glint=(px-17.3).powi(2)+(py-14.5).powi(2)<=1.2*1.2;
                    if glint {[235,255,250,255]} else if pupil {[13,42,47,255]} else if eye {[76,226,186,255]} else if inside_segment(px,py,(4.0,7.0),(4.0,3.8),1.4)||inside_segment(px,py,(4.0,3.8),(7.2,3.8),1.4)||inside_segment(px,py,(24.8,28.2),(28.0,28.2),1.4)||inside_segment(px,py,(28.0,28.2),(28.0,25.0),1.4) {[99,163,255,255]} else {[0,0,0,0]}
                }
                TrayState::Paused=>{
                    if d2<=14.3*14.3 {if (px>=11.0&&px<=13.8||px>=18.2&&px<=21.0)&&py>=9.0&&py<=23.0 {[42,33,22,255]} else {[255,195,106,255]}} else {[0,0,0,0]}
                }
                TrayState::Error=>{
                    let bar=px>=14.5&&px<=17.5&&py>=7.7&&py<=18.0;
                    let dot=(px-16.0).powi(2)+(py-22.0).powi(2)<=1.6*1.6;
                    if d2<=14.3*14.3 {if bar||dot {[48,23,30,255]} else {[255,127,132,255]}} else {[0,0,0,0]}
                }
            };
            sum[0]+=c[0] as u32*c[3] as u32;sum[1]+=c[1] as u32*c[3] as u32;sum[2]+=c[2] as u32*c[3] as u32;sum[3]+=c[3] as u32;
        }}
        let i=(y*W+x)*4;let alpha=sum[3];
        if alpha>0 {for k in 0..3 {out[i+k]=(sum[k]/alpha) as u8;}out[i+3]=(alpha/(N*N) as u32) as u8;}
    }}
    out
}
fn make_icon(state:TrayState)->Result<Icon,String> {Icon::from_rgba(render_icon(state),32,32).map_err(|e|e.to_string())}

#[cfg(test)]
mod tests {
    use super::*;
    fn state()->super::super::State {
        super::super::State {
            config:super::super::Config {blocked_titles:vec![],blocked_processes:vec![],user_blocked_processes:vec![],visible_only:false,ring_size:5,jpeg_quality:70,max_width:1920},
            paused:false,monitor:None,window:None,region:None,frames:std::collections::VecDeque::new(),last_look:Some(42),inbox:vec![],
            accent:[90,197,235],glow_enabled:true,glow_preference:true,minimize_to_tray:true,error:String::new(),language:Language::Ru,capture_activity:std::sync::Arc::new(std::sync::Mutex::new(crate::activity::CaptureActivity::default())),
        }
    }
    #[test] fn status_icons_are_full_size_and_distinct() {
        let watch=render_icon(TrayState::Watching);let paused=render_icon(TrayState::Paused);let error=render_icon(TrayState::Error);
        assert_eq!(watch.len(),32*32*4);assert_ne!(watch,paused);assert_ne!(paused,error);
        for icon in [watch,paused,error] {assert!(icon.chunks_exact(4).filter(|p|p[3]>128).count()>250);}
    }
    #[test] fn tray_tooltips_follow_language_and_state() {
        assert_eq!(tooltip(TrayState::Paused,Language::En),"Screenmate — paused");
        assert_eq!(tooltip(TrayState::Error,Language::Ru),"Скринмейт — ничего не вижу");
    }
    #[test] fn quick_controls_work_without_a_gui_and_pause_clears_frames() {
        let mut s=state();
        s.frames.push_back(super::super::Frame {jpg:vec![1],width:1,height:1,hash:0,timestamp:String::new()});
        assert!(apply_capture_action(TrayAction::TogglePause,&mut s,&[]));
        assert!(s.paused);assert!(s.frames.is_empty());
        assert!(apply_capture_action(TrayAction::TogglePause,&mut s,&[]));assert!(!s.paused);
        assert!(apply_capture_action(TrayAction::ToggleGlow,&mut s,&[]));assert!(!s.glow_enabled);assert!(!s.glow_preference);
    }
    #[test] fn next_monitor_clears_old_capture_region_and_resets_diff() {
        let mut s=state();s.window=Some(99);s.region=Some([1,2,3,4]);
        let monitors=vec![
            super::super::Monitor{id:10,name:"a".into(),x:0,y:0,width:800,height:600,primary:true},
            super::super::Monitor{id:11,name:"b".into(),x:800,y:0,width:800,height:600,primary:false},
        ];
        assert!(apply_capture_action(TrayAction::NextMonitor,&mut s,&monitors));
        assert_eq!(s.monitor,Some(11));assert!(s.window.is_none());assert!(s.region.is_none());assert!(s.last_look.is_none());
        assert!(apply_capture_action(TrayAction::NextMonitor,&mut s,&monitors));assert_eq!(s.monitor,Some(10));
        assert!(!apply_capture_action(TrayAction::NextMonitor,&mut s,&[]));
    }
}
