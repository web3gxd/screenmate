//! Accessible, custom-drawn dark context menu for the Windows tray.
//! Standard child buttons provide the native UI Automation/MSAA providers;
//! owner drawing retains the visual design. Runs on the tray message thread.
use super::{TrayAction, TrayState};
use super::super::i18n::Language;
use std::{cell::RefCell, collections::VecDeque, ffi::c_void, ptr};

type Handle = isize;
#[repr(C)] #[derive(Clone, Copy, Default)] struct Point { x: i32, y: i32 }
#[repr(C)] #[derive(Clone, Copy, Default)] struct Rect { left: i32, top: i32, right: i32, bottom: i32 }
#[repr(C)] struct Paint { dc: Handle, erase: i32, rect: Rect, restore: i32, inc_update: i32, reserved: [u8; 32] }
#[repr(C)] struct WindowClass { style: u32, proc: Option<unsafe extern "system" fn(Handle,u32,usize,isize)->isize>, class_extra: i32, window_extra: i32, instance: Handle, icon: Handle, cursor: Handle, background: Handle, menu_name: *const u16, class_name: *const u16 }
#[repr(C)] struct MonitorInfo { size:u32, monitor:Rect, work:Rect, flags:u32 }
#[repr(C)] struct DrawItem { ctl_type:u32, ctl_id:u32, item_id:u32, action:u32, state:u32, item:Handle, dc:Handle, rect:Rect, data:usize }
#[repr(C)] struct TrackMouse { size:u32, flags:u32, hwnd:Handle, hover_time:u32 }
#[link(name="user32")]
extern "system" {
    fn RegisterClassW(class:*const WindowClass)->u16;
    fn UnregisterClassW(name:*const u16, instance:Handle)->i32;
    fn CreateWindowExW(ex:u32,class:*const u16,title:*const u16,style:u32,x:i32,y:i32,w:i32,h:i32,parent:Handle,menu:Handle,instance:Handle,param:*mut c_void)->Handle;
    fn DestroyWindow(hwnd:Handle)->i32;
    fn DefWindowProcW(hwnd:Handle,msg:u32,w:usize,l:isize)->isize;
    fn ShowWindow(hwnd:Handle,command:i32)->i32;
    fn IsWindowVisible(hwnd:Handle)->i32;
    fn SetWindowPos(hwnd:Handle,after:Handle,x:i32,y:i32,w:i32,h:i32,flags:u32)->i32;
    fn SetWindowRgn(hwnd:Handle,region:Handle,redraw:i32)->i32;
    fn InvalidateRect(hwnd:Handle,rect:*const Rect,erase:i32)->i32;
    fn BeginPaint(hwnd:Handle,paint:*mut Paint)->Handle;
    fn EndPaint(hwnd:Handle,paint:*const Paint)->i32;
    fn FillRect(dc:Handle,rect:*const Rect,brush:Handle)->i32;
    fn DrawTextW(dc:Handle,text:*const u16,count:i32,rect:*mut Rect,format:u32)->i32;
    fn GetCursorPos(point:*mut Point)->i32;
    fn GetWindowRect(hwnd:Handle,rect:*mut Rect)->i32;
    fn GetAsyncKeyState(key:i32)->i16;
    fn MonitorFromPoint(point:Point,flags:u32)->Handle;
    fn GetMonitorInfoW(monitor:Handle,info:*mut MonitorInfo)->i32;
    fn GetDpiForWindow(hwnd:Handle)->u32;
    fn GetDlgItem(hwnd:Handle,id:i32)->Handle;
    fn GetParent(hwnd:Handle)->Handle;
    fn SetFocus(hwnd:Handle)->Handle;
    fn SetForegroundWindow(hwnd:Handle)->i32;
    fn GetForegroundWindow()->Handle;
    fn IsWindow(hwnd:Handle)->i32;
    fn PostMessageW(hwnd:Handle,msg:u32,w:usize,l:isize)->i32;
    fn SetWindowTextW(hwnd:Handle,text:*const u16)->i32;
    fn GetKeyState(key:i32)->i16;
    fn TrackMouseEvent(event:*mut TrackMouse)->i32;
}
#[link(name="comctl32")]
extern "system" {
    fn SetWindowSubclass(hwnd:Handle,callback:Option<unsafe extern "system" fn(Handle,u32,usize,isize,usize,usize)->isize>,id:usize,data:usize)->i32;
    fn DefSubclassProc(hwnd:Handle,msg:u32,w:usize,l:isize)->isize;
}
#[link(name="gdi32")]
extern "system" {
    fn CreateSolidBrush(color:u32)->Handle;
    fn DeleteObject(object:Handle)->i32;
    fn CreateRoundRectRgn(left:i32,top:i32,right:i32,bottom:i32,ellipse_width:i32,ellipse_height:i32)->Handle;
    fn CreateFontW(height:i32,width:i32,escapement:i32,orientation:i32,weight:i32,italic:u32,underline:u32,strike:u32,charset:u32,out_precision:u32,clip_precision:u32,quality:u32,pitch:u32,face:*const u16)->Handle;
    fn SelectObject(dc:Handle,object:Handle)->Handle;
    fn SetTextColor(dc:Handle,color:u32)->u32;
    fn SetBkMode(dc:Handle,mode:i32)->i32;
    fn AddFontMemResourceEx(data:*const c_void,size:u32,reserved:*mut c_void,count:*mut u32)->Handle;
    fn RemoveFontMemResourceEx(handle:Handle)->i32;
}
#[link(name="kernel32")]
extern "system" { fn GetModuleHandleW(name:*const u16)->Handle; }

const WIDTH: i32 = 300;
const HEIGHT: i32 = 272;
const ROW_TOP: i32 = 66;
const ROW_HEIGHT: i32 = 37;
const FIRST_BUTTON: i32 = 100;
const ROWS: [TrayAction; 5] = [TrayAction::Open, TrayAction::TogglePause, TrayAction::ToggleGlow, TrayAction::NextMonitor, TrayAction::Exit];

#[derive(Clone)]
struct Data { state:TrayState, glow:bool, monitor:String, accent:[u8;3], language:Language, hover:i32, scale:f32, restore:Handle, queue:VecDeque<TrayAction>, #[cfg(test)] preview_keep_open:bool }
impl Default for Data { fn default()->Self { Self { state:TrayState::Watching, glow:true, monitor:"Основной монитор".into(), accent:[90,197,235], language:Language::Ru, hover:-1, scale:1.0, restore:0, queue:VecDeque::new(), #[cfg(test)] preview_keep_open:false } } }
thread_local! { static DATA:RefCell<Data> = RefCell::new(Data::default()); }
fn wide(s:&str)->Vec<u16> { s.encode_utf16().chain(Some(0)).collect() }
fn rgb(r:u8,g:u8,b:u8)->u32 { r as u32 | (g as u32)<<8 | (b as u32)<<16 }
fn sc(v:i32,s:f32)->i32 { (v as f32*s).round() as i32 }
fn hit(y:i32,s:f32)->i32 { let y=(y as f32/s).floor() as i32; let row=(y-ROW_TOP)/ROW_HEIGHT; if y>=ROW_TOP && row>=0 && row<ROWS.len() as i32 {row} else {-1} }
fn row_label(d:&Data,index:usize)->&'static str {
    let l=d.language;
    match index {
        0=>l.text("Открыть Скринмейт","Open Screenmate"),
        1=>if d.state==TrayState::Paused {l.text("Продолжить захват","Resume capture")} else {l.text("Пауза","Pause")},
        2=>if d.glow {l.text("Выключить свечение","Turn off glow")} else {l.text("Включить свечение","Turn on glow")},
        3=>l.text("Следующий монитор","Next monitor"),
        _=>l.text("Выйти","Quit"),
    }
}
fn preview_keep_open()->bool {
    #[cfg(test)] { DATA.with(|v|v.borrow().preview_keep_open) }
    #[cfg(not(test))] { false }
}

unsafe fn refresh_buttons(parent:Handle) {
    let data=DATA.with(|v|v.borrow().clone());
    for i in 0..ROWS.len() {
        let button=GetDlgItem(parent,FIRST_BUTTON+i as i32);
        if button!=0 { SetWindowTextW(button,wide(row_label(&data,i)).as_ptr());InvalidateRect(button,ptr::null(),0); }
    }
}

unsafe fn close_popup(hwnd:Handle, restore_focus:bool) {
    let previous=DATA.with(|v|v.borrow().restore);
    let should_restore=restore_focus && GetForegroundWindow()==hwnd;
    ShowWindow(hwnd,0);
    // Don't steal focus back when the user explicitly clicked another window.
    if should_restore && previous!=0 && previous!=hwnd && IsWindow(previous)!=0 && IsWindowVisible(previous)!=0 {
        SetForegroundWindow(previous);
    }
}

unsafe fn fill(dc:Handle,rect:Rect,color:u32) { let b=CreateSolidBrush(color); if b!=0 { FillRect(dc,&rect,b); DeleteObject(b); } }
unsafe fn label(dc:Handle,text:&str,rect:Rect,color:u32,font:Handle,flags:u32) {
    let previous=SelectObject(dc,font); SetTextColor(dc,color); SetBkMode(dc,1);
    let mut r=rect; let text=wide(text); DrawTextW(dc,text.as_ptr(),-1,&mut r,flags);
    SelectObject(dc,previous);
}
unsafe fn paint(hwnd:Handle) {
    let mut ps:Paint=std::mem::zeroed(); let dc=BeginPaint(hwnd,&mut ps); if dc==0 { return; }
    let d=DATA.with(|v|v.borrow().clone()); let s=d.scale;
    let r=|x1,y1,x2,y2|Rect {left:sc(x1,s),top:sc(y1,s),right:sc(x2,s),bottom:sc(y2,s)};
    fill(dc,r(0,0,WIDTH,HEIGHT),rgb(19,27,34));
    let name=wide("Onest");
    let bold=CreateFontW(-sc(15,s),0,0,0,700,0,0,0,1,0,0,5,0,name.as_ptr());
    let small=CreateFontW(-sc(11,s),0,0,0,400,0,0,0,1,0,0,5,0,name.as_ptr());
    label(dc,d.language.text("Скринмейт","Screenmate"),r(18,11,200,34),rgb(238,247,246),bold,0x24);
    let (state,state_color)=match d.state { TrayState::Watching=>(d.language.text("Смотрю","Watching"),rgb(d.accent[0],d.accent[1],d.accent[2])), TrayState::Paused=>(d.language.text("Пауза","Paused"),rgb(255,194,103)), TrayState::Error=>(d.language.text("Ничего не вижу","No capture"),rgb(255,125,128)) };
    label(dc,state,r(18,34,180,53),state_color,small,0x24);
    fill(dc,r(15,59,WIDTH-15,60),rgb(52,66,71));
    label(dc,&format!("{}: {}",d.language.text("Экран","Screen"),d.monitor),r(20,251,WIDTH-16,269),rgb(139,160,159),small,0x24 | 0x8000);
    if bold!=0 {DeleteObject(bold);} if small!=0 {DeleteObject(small);}
    EndPaint(hwnd,&ps);
}

unsafe fn draw_button(draw:&DrawItem) {
    let index=draw.ctl_id as i32-FIRST_BUTTON;
    if !(0..ROWS.len() as i32).contains(&index) {return;}
    let d=DATA.with(|v|v.borrow().clone());let s=d.scale;
    let hovered=d.hover==index;
    let focused=draw.state&0x10!=0; // ODS_FOCUS
    let pressed=draw.state&0x01!=0; // ODS_SELECTED
    let color=if pressed {rgb(32+d.accent[0]/10,43+d.accent[1]/10,47+d.accent[2]/10)}
        else if hovered||focused {rgb(25+d.accent[0]/10,34+d.accent[1]/10,39+d.accent[2]/10)}
        else {rgb(19,27,34)};
    fill(draw.dc,draw.rect,color);
    if focused {
        let marker=Rect{left:sc(1,s),top:sc(7,s),right:sc(4,s),bottom:sc(30,s)};
        fill(draw.dc,marker,rgb(d.accent[0],d.accent[1],d.accent[2]));
    }
    let face=wide("Onest");
    let font=CreateFontW(-sc(14,s),0,0,0,400,0,0,0,1,0,0,5,0,face.as_ptr());
    let text_color=if index==4 {rgb(255,166,160)} else {rgb(223,236,235)};
    let text_rect=Rect{left:sc(12,s),top:0,right:draw.rect.right-sc(12,s),bottom:draw.rect.bottom};
    label(draw.dc,row_label(&d,index as usize),text_rect,text_color,font,0x24);
    if font!=0 {DeleteObject(font);}
}
unsafe extern "system" fn window_proc(hwnd:Handle,msg:u32,w:usize,l:isize)->isize {
    match msg {
        0x000F => {paint(hwnd);0} // WM_PAINT
        0x0014 => 1, // avoid flicker: background is fully painted
        0x002B => {if l!=0 {draw_button(&*(l as *const DrawItem));}1} // WM_DRAWITEM
        0x0111 => { // Native button clicked, including UIA Invoke/BM_CLICK.
            let id=(w&0xffff) as i32;let notification=(w>>16)&0xffff;
            if notification==0 && (FIRST_BUTTON..FIRST_BUTTON+ROWS.len() as i32).contains(&id) {
                DATA.with(|v| {let mut d=v.borrow_mut();d.queue.push_back(ROWS[(id-FIRST_BUTTON) as usize]);d.hover=-1;});
                close_popup(hwnd,true);return 0;
            }
            DefWindowProcW(hwnd,msg,w,l)
        }
        0x0006 if w&0xffff==0 => { // WM_ACTIVATE: defer dismissal to avoid reentrant hide.
            if !preview_keep_open() {PostMessageW(hwnd,0x8001,0,0);}0
        }
        0x8001 => {if IsWindowVisible(hwnd)!=0 && GetForegroundWindow()!=hwnd {close_popup(hwnd,false);}0}
        0x0200 => { // WM_MOUSEMOVE
            let y=((l>>16) as u16 as i16) as i32;
            let changed=DATA.with(|v| {let mut d=v.borrow_mut();let row=hit(y,d.scale);if row!=d.hover {d.hover=row;true}else{false}});
            if changed {InvalidateRect(hwnd,ptr::null(),0);} 0
        }
        0x0202 => { // WM_LBUTTONUP
            let y=((l>>16) as u16 as i16) as i32;
            DATA.with(|v| {let mut d=v.borrow_mut();let row=hit(y,d.scale);if row>=0 {d.queue.push_back(ROWS[row as usize]);}d.hover=-1;});
            close_popup(hwnd,true);0
        }
        0x001B => {close_popup(hwnd,false);0} // WM_CANCELMODE
        _=>DefWindowProcW(hwnd,msg,w,l),
    }
}

unsafe extern "system" fn button_proc(hwnd:Handle,msg:u32,w:usize,l:isize,id:usize,_data:usize)->isize {
    let index=id as i32-FIRST_BUTTON;
    match msg {
        0x0087 => DefSubclassProc(hwnd,msg,w,l) | 0x01 | 0x04, // WM_GETDLGCODE: arrows + all keys
        0x0100 => { // WM_KEYDOWN
            let step=match w {0x26=>Some(-1),0x28=>Some(1),0x09=>Some(if GetKeyState(0x10)<0 {-1}else{1}),_=>None};
            if let Some(step)=step {
                let next=(index+step).rem_euclid(ROWS.len() as i32);
                let target=GetDlgItem(GetParent(hwnd),FIRST_BUTTON+next);
                if target!=0 {SetFocus(target);} return 0;
            }
            if w==0x1b {close_popup(GetParent(hwnd),true);return 0;} // Escape
            if w==0x0d || w==0x20 { // Enter / Space
                if (0..ROWS.len() as i32).contains(&index) {DATA.with(|v|v.borrow_mut().queue.push_back(ROWS[index as usize]));}
                close_popup(GetParent(hwnd),true);return 0;
            }
            DefSubclassProc(hwnd,msg,w,l)
        }
        0x0102 if w==0x0d || w==0x20 => 0, // do not beep after handled activation
        0x0007 | 0x0008 => {InvalidateRect(hwnd,ptr::null(),0);DefSubclassProc(hwnd,msg,w,l)},
        0x0200 => { // WM_MOUSEMOVE: preserve hover styling on child controls.
            let old=DATA.with(|v|{let mut d=v.borrow_mut();let old=d.hover;d.hover=index;old});
            if old!=index {
                let parent=GetParent(hwnd);
                if old>=0 {let previous=GetDlgItem(parent,FIRST_BUTTON+old);if previous!=0 {InvalidateRect(previous,ptr::null(),0);}}
                InvalidateRect(hwnd,ptr::null(),0);
            }
            let mut event=TrackMouse{size:std::mem::size_of::<TrackMouse>() as u32,flags:2,hwnd,hover_time:0};
            TrackMouseEvent(&mut event);DefSubclassProc(hwnd,msg,w,l)
        }
        0x02A3 => { // WM_MOUSELEAVE
            DATA.with(|v|{let mut d=v.borrow_mut();if d.hover==index {d.hover=-1;}});
            InvalidateRect(hwnd,ptr::null(),0);DefSubclassProc(hwnd,msg,w,l)
        }
        _=>DefSubclassProc(hwnd,msg,w,l),
    }
}

pub struct Popup { hwnd:Handle, instance:Handle, class_name:Vec<u16>, font_regular:Handle, font_black:Handle, left_down:bool, right_down:bool }
impl Popup {
    pub fn new()->Result<Self,String> { unsafe {
        let instance=GetModuleHandleW(ptr::null()); let class_name=wide("ScreenmateTrayPopup");
        let class=WindowClass {style:0,proc:Some(window_proc),class_extra:0,window_extra:0,instance,icon:0,cursor:0,background:0,menu_name:ptr::null(),class_name:class_name.as_ptr()};
        if instance==0 || RegisterClassW(&class)==0 {return Err("Не удалось зарегистрировать меню трея".into());}
        // Hidden popup; TOPMOST | TOOLWINDOW. It accepts focus like a native menu.
        let title=wide("Скринмейт — меню");
        let hwnd=CreateWindowExW(0x00000088,class_name.as_ptr(),title.as_ptr(),0x80000000,0,0,WIDTH,HEIGHT,0,0,instance,ptr::null_mut());
        if hwnd==0 {UnregisterClassW(class_name.as_ptr(),instance);return Err("Не удалось создать меню трея".into());}
        let button_class=wide("BUTTON");let initial=DATA.with(|v|v.borrow().clone());
        for i in 0..ROWS.len() {
            // Standard BUTTON class supplies OS accessibility and Invoke;
            // BS_OWNERDRAW preserves our design without fake accessibility.
            let title=wide(row_label(&initial,i));
            let button=CreateWindowExW(0,button_class.as_ptr(),title.as_ptr(),0x5001000b,
                8,ROW_TOP+i as i32*ROW_HEIGHT,WIDTH-16,ROW_HEIGHT,hwnd,(FIRST_BUTTON+i as i32) as Handle,instance,ptr::null_mut());
            if button==0 || SetWindowSubclass(button,Some(button_proc),(FIRST_BUTTON+i as i32) as usize,0)==0 {
                DestroyWindow(hwnd);UnregisterClassW(class_name.as_ptr(),instance);return Err("Не удалось создать доступные пункты меню".into());
            }
        }
        let mut n=0; let regular=include_bytes!("../assets/fonts/Onest-Regular.ttf");let black=include_bytes!("../assets/fonts/Onest-Black.ttf");
        let font_regular=AddFontMemResourceEx(regular.as_ptr() as *const _,regular.len() as u32,ptr::null_mut(),&mut n);
        let font_black=AddFontMemResourceEx(black.as_ptr() as *const _,black.len() as u32,ptr::null_mut(),&mut n);
        Ok(Self {hwnd,instance,class_name,font_regular,font_black,left_down:false,right_down:false})
    } }
    pub fn set_details(&self,state:TrayState,glow:bool,monitor:&str,accent:[u8;3],language:Language) {
        DATA.with(|v| {let mut d=v.borrow_mut();d.state=state;d.glow=glow;d.monitor=monitor.chars().take(80).collect();d.accent=accent;d.language=language;});
        unsafe { SetWindowTextW(self.hwnd,wide(language.text("Скринмейт — меню","Screenmate — menu")).as_ptr());refresh_buttons(self.hwnd);if IsWindowVisible(self.hwnd)!=0 {InvalidateRect(self.hwnd,ptr::null(),0);} }
    }
    pub fn show(&mut self,x:i32,y:i32)->Result<(),String> { unsafe {
        // Avoid flashing the popup at the temporary DPI-probe position when
        // the user invokes the tray menu again while it is already open.
        let foreground=GetForegroundWindow();
        DATA.with(|v| {let mut d=v.borrow_mut();if foreground!=self.hwnd {d.restore=foreground;}});
        ShowWindow(self.hwnd,0);
        let point=Point{x,y};let monitor=MonitorFromPoint(point,2);let mut info=MonitorInfo{size:std::mem::size_of::<MonitorInfo>() as u32,monitor:Rect::default(),work:Rect::default(),flags:0};
        if monitor==0 || GetMonitorInfoW(monitor,&mut info)==0 {return Err("Не удалось определить положение трея".into());}
        // Place the hidden window well inside the target monitor before asking
        // for its DPI. Moving directly to the tray cursor can put most of the
        // window on an adjacent monitor and return the wrong scale.
        let midx=info.work.left+(info.work.right-info.work.left)/2;
        let midy=info.work.top+(info.work.bottom-info.work.top)/2;
        if SetWindowPos(self.hwnd,0,midx-WIDTH/2,midy-HEIGHT/2,0,0,0x0015)==0 {return Err("Не удалось расположить меню трея".into());}
        let dpi=GetDpiForWindow(self.hwnd).clamp(96,288);let scale=dpi as f32/96.0;
        let w=sc(WIDTH,scale);let h=sc(HEIGHT,scale);
        let posx=(x-w+16).clamp(info.work.left,(info.work.right-w).max(info.work.left));
        let posy=(y-h-10).clamp(info.work.top,(info.work.bottom-h).max(info.work.top));
        DATA.with(|v|{let mut d=v.borrow_mut();d.scale=scale;d.hover=-1;});
        for i in 0..ROWS.len() {
            let button=GetDlgItem(self.hwnd,FIRST_BUTTON+i as i32);
            if button==0 || SetWindowPos(button,0,sc(8,scale),sc(ROW_TOP+i as i32*ROW_HEIGHT,scale),sc(WIDTH-16,scale),sc(ROW_HEIGHT,scale),0x0014)==0 {
                return Err("Не удалось расположить пункты меню".into());
            }
        }
        refresh_buttons(self.hwnd);
        let region=CreateRoundRectRgn(0,0,w+1,h+1,sc(20,scale),sc(20,scale));
        if region!=0 && SetWindowRgn(self.hwnd,region,0)==0 {DeleteObject(region);}
        if SetWindowPos(self.hwnd,-1,posx,posy,w,h,0x0040)==0 {return Err("Не удалось открыть меню трея".into());}
        SetForegroundWindow(self.hwnd);
        let first=GetDlgItem(self.hwnd,FIRST_BUTTON);if first!=0 {SetFocus(first);}
        self.left_down=GetAsyncKeyState(1)<0;self.right_down=GetAsyncKeyState(2)<0; Ok(())
    } }
    pub fn poll(&mut self)->Option<TrayAction> { unsafe {
        if IsWindowVisible(self.hwnd)!=0 {
            let down=GetAsyncKeyState(1)<0;
            let right=GetAsyncKeyState(2)<0;
            if (down && !self.left_down) || (right && !self.right_down) {
                let mut p=Point::default();let mut r=Rect::default();
                if GetCursorPos(&mut p)!=0 && GetWindowRect(self.hwnd,&mut r)!=0 && (p.x<r.left || p.x>=r.right || p.y<r.top || p.y>=r.bottom) {close_popup(self.hwnd,false);}
            }
            self.left_down=down;self.right_down=right;
        }
        DATA.with(|v|v.borrow_mut().queue.pop_front())
    } }
}
impl Drop for Popup { fn drop(&mut self) { unsafe {DestroyWindow(self.hwnd);UnregisterClassW(self.class_name.as_ptr(),self.instance);if self.font_regular!=0 {RemoveFontMemResourceEx(self.font_regular);} if self.font_black!=0 {RemoveFontMemResourceEx(self.font_black);} } } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn row_hit_test_tracks_dpi_scaling() {
        assert_eq!(hit(sc(ROW_TOP + ROW_HEIGHT * 2 + 5, 1.5), 1.5), 2);
        assert_eq!(hit(sc(ROW_TOP - 2, 1.5), 1.5), -1);
        assert_eq!(hit(sc(ROW_TOP + ROW_HEIGHT * 5 + 1, 2.0), 2.0), -1);
    }

    #[test]
    fn labels_follow_live_privacy_state() {
        let mut d=Data::default();
        assert_eq!(row_label(&d,1),"Пауза");
        assert_eq!(row_label(&d,2),"Выключить свечение");
        d.state=TrayState::Paused;d.glow=false;
        assert_eq!(row_label(&d,1),"Продолжить захват");
        assert_eq!(row_label(&d,2),"Включить свечение");
        d.language=Language::En;
        assert_eq!(row_label(&d,1),"Resume capture");
        assert_eq!(row_label(&d,2),"Turn on glow");
    }

    /// Creates the actual production popup and exits after one test action or
    /// dismissal. It has its own process and never touches an app instance.
    #[test]
    #[ignore = "manual desktop accessibility preview only"]
    fn dev_tray_popup_accessibility() {
        #[link(name="user32")]
        extern "system" { fn SetWindowLongPtrW(hwnd:Handle,index:i32,value:isize)->isize; }
        use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW,PeekMessageW,TranslateMessage,MSG,PM_REMOVE};
        let mut popup=Popup::new().expect("create real tray popup");
        // Tool windows are intentionally filtered from Computer Use window
        // inventory. Promote this isolated test window only; controls,
        // input, painting, placement, and event handling remain production.
        unsafe {SetWindowLongPtrW(popup.hwnd,-20,0x00040008);}
        DATA.with(|v|v.borrow_mut().preview_keep_open=true);
        popup.set_details(TrayState::Watching,true,"Монитор 1",[90,197,235],Language::Ru);
        popup.show(780,720).expect("show real tray popup");
        println!("SCREENMATE_ACCESSIBILITY_POPUP_READY");
        let mut message:MSG=unsafe{std::mem::zeroed()};
        let until=std::time::Instant::now()+std::time::Duration::from_secs(90);
        while std::time::Instant::now()<until {
            unsafe {while PeekMessageW(&mut message,0,0,0,PM_REMOVE)!=0 {TranslateMessage(&message);DispatchMessageW(&message);}}
            if let Some(action)=popup.poll() {println!("SCREENMATE_ACCESSIBILITY_ACTION {action:?}");break;}
            if unsafe{IsWindowVisible(popup.hwnd)}==0 {println!("SCREENMATE_ACCESSIBILITY_DISMISSED");break;}
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }

    /// Checks production activation semantics without synthesising user input.
    #[test]
    #[ignore = "manual desktop activation check only"]
    fn dev_tray_popup_stays_open() {
        #[link(name="user32")]
        extern "system" {fn GetFocus()->Handle;}
        use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW,PeekMessageW,TranslateMessage,MSG,PM_REMOVE};
        let mut popup=Popup::new().expect("create production popup");
        popup.show(780,720).expect("show production popup");
        let until=std::time::Instant::now()+std::time::Duration::from_secs(2);
        let mut message:MSG=unsafe{std::mem::zeroed()};
        while std::time::Instant::now()<until {
            unsafe {while PeekMessageW(&mut message,0,0,0,PM_REMOVE)!=0 {TranslateMessage(&message);DispatchMessageW(&message);}}
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        unsafe {
            assert_ne!(IsWindowVisible(popup.hwnd),0,"popup hid without a click or Escape");
            assert_eq!(GetForegroundWindow(),popup.hwnd,"popup did not retain activation");
            assert_eq!(GetFocus(),GetDlgItem(popup.hwnd,FIRST_BUTTON),"first button did not receive keyboard focus");
        }
    }

    /// A diagnostic window using the exact production WM_PAINT path. It is
    /// compiled only into the test binary and never runs in normal test suites.
    #[test]
    #[ignore = "manual desktop preview only"]
    fn dev_tray_popup_preview() {
        #[link(name="user32")]
        extern "system" {
            fn SetWindowLongPtrW(hwnd:Handle,index:i32,value:isize)->isize;
            fn SetWindowTextW(hwnd:Handle,text:*const u16)->i32;
            fn AdjustWindowRectEx(rect:*mut Rect,style:u32,menu:i32,ex:u32)->i32;
            fn IsWindow(hwnd:Handle)->i32;
        }
        use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW,PeekMessageW,TranslateMessage,MSG,PM_REMOVE};
        let popup=Popup::new().expect("create diagnostic tray popup");
        popup.set_details(TrayState::Watching,true,"Монитор 1",[90,197,235],Language::Ru);
        unsafe {
            // Promote the tool window to an ordinary titled test window so a
            // desktop inspector can enumerate it. Production never does this.
            const EX_APPWINDOW:isize=0x00040000;
            const OVERLAPPED:u32=0x00cf0000;
            SetWindowLongPtrW(popup.hwnd,-20,EX_APPWINDOW);
            SetWindowLongPtrW(popup.hwnd,-16,OVERLAPPED as isize);
            let title=wide("Screenmate tray menu preview");
            SetWindowTextW(popup.hwnd,title.as_ptr());
            let scale=GetDpiForWindow(popup.hwnd).clamp(96,288) as f32/96.0;
            DATA.with(|v|v.borrow_mut().scale=scale);
            let mut bounds=Rect{left:0,top:0,right:sc(WIDTH,scale),bottom:sc(HEIGHT,scale)};
            AdjustWindowRectEx(&mut bounds,OVERLAPPED,0,EX_APPWINDOW as u32);
            assert_ne!(SetWindowPos(popup.hwnd,0,120,120,bounds.right-bounds.left,bounds.bottom-bounds.top,0x0064),0);
            println!("SCREENMATE_TRAY_PREVIEW_READY");
            let mut message:MSG=std::mem::zeroed();
            let until=std::time::Instant::now()+std::time::Duration::from_secs(120);
            while IsWindow(popup.hwnd)!=0 && std::time::Instant::now()<until {
                while PeekMessageW(&mut message,0,0,0,PM_REMOVE)!=0 {TranslateMessage(&message);DispatchMessageW(&message);}
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    }
}
