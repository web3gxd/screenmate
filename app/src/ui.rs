//! Compact control panel for capture and appearance.
use super::*;
use crate::icons::{self, Icon};
use eframe::egui::{self, Align2, Color32, FontFamily, FontId, Rect, RichText, Sense, Stroke, Vec2};

#[cfg(test)]
#[path="privacy_layout_tests.rs"]
mod privacy_layout_tests;

#[cfg(test)]
fn update_privacy_layout(ctx:&egui::Context,update:impl FnOnce(&mut privacy_layout_tests::PrivacyLayoutGeometry)) {
    ctx.data_mut(|data| {
        let id=egui::Id::new("privacy-layout-test");
        let mut geometry=data.get_temp::<privacy_layout_tests::PrivacyLayoutGeometry>(id).unwrap_or_default();
        update(&mut geometry);data.insert_temp(id,geometry);
    });
}

// One spacing scale in egui points. Smaller drawing offsets may remain for
// optically centering strokes and icons within these layout bounds.
mod metrics {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 16.0;
    pub const LG: f32 = 24.0;
    pub const BUTTON: f32 = 40.0;
    pub const SWATCH: f32 = 36.0;
    pub const CARD_MAX: f32 = 204.0;
}

const TEXT: Color32 = Color32::from_rgb(238, 243, 246);
const MUTED: Color32 = Color32::from_rgb(156, 169, 180);
const PALETTE: [[u8; 3]; 6] = [
    [90,197,235], [89,216,168], [128,153,255],
    [198,151,244], [248,159,155], [235,194,107],
];

fn heavy(size:f32)->FontId { FontId::new(size,FontFamily::Name("onest-black".into())) }
fn regular(size:f32)->FontId { FontId::new(size,FontFamily::Proportional) }
fn title(text:&str,size:f32)->RichText { RichText::new(text).font(heavy(size)).color(TEXT) }
fn label(text:&str,size:f32)->RichText { RichText::new(text).font(regular(size)).color(MUTED) }

#[derive(Clone,Copy)]
struct Theme { accent:Color32, bg:Color32, panel:Color32, field:Color32, selected:Color32, edge:Color32 }
impl Theme {
    fn new(rgb:[u8;3])->Self {
        let accent=Color32::from_rgb(rgb[0],rgb[1],rgb[2]);
        let mix=|base:[u8;3],t:f32| Color32::from_rgb(
            (base[0] as f32*(1.0-t)+rgb[0] as f32*t) as u8,
            (base[1] as f32*(1.0-t)+rgb[1] as f32*t) as u8,
            (base[2] as f32*(1.0-t)+rgb[2] as f32*t) as u8);
        Self {accent,bg:mix([12,15,20],0.025),panel:mix([23,27,34],0.065),field:mix([30,36,43],0.075),selected:mix([27,36,43],0.20),edge:mix([53,65,73],0.21)}
    }
}

pub fn setup(ctx:&egui::Context)->egui::TextureHandle {
    let mut fonts=egui::FontDefinitions::default();
    fonts.font_data.insert("onest".into(),egui::FontData::from_static(include_bytes!("../assets/fonts/Onest-Regular.ttf")));
    fonts.font_data.insert("onest-black".into(),egui::FontData::from_static(include_bytes!("../assets/fonts/Onest-Black.ttf")));
    fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0,"onest".into());
    fonts.families.insert(FontFamily::Name("onest-black".into()),vec!["onest-black".into(),"onest".into()]);
    ctx.set_fonts(fonts);
    let mut style=(*ctx.style()).clone();
    style.spacing.item_spacing=Vec2::new(metrics::SM,metrics::XS);
    style.spacing.button_padding=Vec2::new(metrics::MD,metrics::SM);
    style.spacing.interact_size.y=metrics::BUTTON;
    style.visuals=egui::Visuals::dark();
    style.visuals.override_text_color=Some(TEXT);
    for widget in [&mut style.visuals.widgets.inactive,&mut style.visuals.widgets.hovered,&mut style.visuals.widgets.active] {widget.rounding=egui::Rounding::same(10.0);widget.fg_stroke=Stroke::new(1.0_f32,TEXT);}
    ctx.set_style(style);
    let image=egui::ColorImage::from_rgba_unmultiplied([128,128],include_bytes!("../assets/icon-128.rgba"));
    ctx.load_texture("screenmate-mark",image,egui::TextureOptions::LINEAR)
}

/// The appearance preference is separate from the privacy configuration and
/// from the per-session socket token. An invalid file is simply ignored.
pub fn load_appearance()->([u8;3],bool,bool,crate::i18n::Language) {
    let defaults=([90,197,235],true,true,crate::i18n::Language::Ru);
    let Some(data)=dir().ok().and_then(|p|fs::read(p.join("appearance.json")).ok()) else {return defaults};
    if data.len()>1024{return defaults;}
    let Ok(value)=serde_json::from_slice::<Value>(&data) else {return defaults};
    let accent=value.get("accent").and_then(|v|serde_json::from_value::<[u8;3]>(v.clone()).ok()).filter(|a|PALETTE.contains(a)).unwrap_or(defaults.0);
    let glow=value.get("glow_enabled").and_then(Value::as_bool).unwrap_or(defaults.1);
    let minimize=value.get("minimize_to_tray").and_then(Value::as_bool).unwrap_or(defaults.2);
    let language=value.get("language").and_then(Value::as_str).map(crate::i18n::Language::from_code).unwrap_or(defaults.3);
    (accent,glow,minimize,language)
}
pub(super) fn appearance_value(s:&State)->Value {
    json!({"accent":s.accent,"glow_enabled":s.glow_preference,"minimize_to_tray":s.minimize_to_tray,"language":s.language.code()})
}
pub(super) fn save_appearance(s:&State) {
    if let Ok(path)=dir() {let _=fs::write(path.join("appearance.json"),appearance_value(s).to_string());}
}

fn text_and_icon(ui:&mut egui::Ui,size:Vec2,icon:Icon,text:&str,theme:Theme,selected:bool)->egui::Response {
    let (rect,response)=ui.allocate_exact_size(size,Sense::click());
    crate::access::radio(&response,text,selected);
    let fill=if selected {theme.selected} else if response.hovered(){theme.field} else {theme.panel};
    let stroke=if response.has_focus() {Stroke::new(2.0_f32,theme.accent)} else if selected {Stroke::new(1.0_f32,theme.accent)} else {Stroke::new(1.0_f32,theme.edge)};
    let p=ui.painter();p.rect_filled(rect,12.0,fill);p.rect_stroke(rect,12.0,stroke);
    let ir=Rect::from_min_size(rect.min+Vec2::new(12.0,(rect.height()-16.0)*0.5),Vec2::splat(16.0));
    icons::paint(p,ir,icon,if selected {theme.accent}else{MUTED});
    p.text(egui::pos2(rect.left()+36.0,rect.center().y),Align2::LEFT_CENTER,text,regular(13.0),TEXT);
    response
}
fn primary(ui:&mut egui::Ui,size:Vec2,icon:Icon,text:&str,theme:Theme)->egui::Response {
    let (rect,response)=ui.allocate_exact_size(size,Sense::click());let p=ui.painter();
    crate::access::button(&response,text);
    let fill=if response.hovered(){theme.accent.gamma_multiply(0.87)}else{theme.accent};
    p.rect_filled(rect,12.0,fill);
    if response.has_focus(){p.rect_stroke(rect.shrink(2.0),10.0,Stroke::new(2.0_f32,Color32::from_rgb(11,24,29)));}
    let ink=Color32::from_rgb(11,24,29);
    icons::paint(p,Rect::from_min_size(rect.min+Vec2::new(16.0,(rect.height()-16.0)*0.5),Vec2::splat(16.0)),icon,ink);
    p.text(egui::pos2(rect.left()+40.0,rect.center().y),Align2::LEFT_CENTER,text,regular(13.0),ink);response
}

fn language_option(ui:&mut egui::Ui,text:&str,name:&str,selected:bool,theme:Theme)->bool {
    let (rect,response)=ui.allocate_exact_size(Vec2::new(36.0,28.0),Sense::click());
    crate::access::radio(&response,name,selected);
    let painter=ui.painter();
    painter.rect_filled(rect,8.0,if selected {theme.selected} else if response.hovered(){theme.field}else{theme.bg});
    if selected||response.has_focus(){painter.rect_stroke(rect,8.0,Stroke::new(if response.has_focus(){2.0_f32}else{1.0_f32},theme.accent));}
    painter.text(rect.center(),Align2::CENTER_CENTER,text,regular(11.0),if selected{theme.accent}else{MUTED});
    response.clicked()
}

fn privacy_button(ui:&mut egui::Ui,theme:Theme,compact:bool,language:crate::i18n::Language)->egui::Response {
    let width=if compact {metrics::BUTTON} else {188.0};
    let (rect,response)=ui.allocate_exact_size(Vec2::new(width,metrics::SWATCH),Sense::click());
    let name=language.text("Скрывать приложения","Hide apps");
    crate::access::button(&response,name);
    let p=ui.painter();
    p.rect_filled(rect,10.0,if response.hovered(){theme.selected}else{theme.field});
    p.rect_stroke(rect,10.0,Stroke::new(if response.has_focus(){2.0_f32}else{1.0_f32},if response.has_focus(){theme.accent}else{theme.edge}));
    let left=if compact {(width-16.0)*0.5}else{12.0};
    icons::paint(p,Rect::from_min_size(rect.min+Vec2::new(left,(rect.height()-16.0)*0.5),Vec2::splat(16.0)),Icon::Shield,theme.accent);
    if !compact {p.text(egui::pos2(rect.left()+36.0,rect.center().y),Align2::LEFT_CENTER,name,regular(12.0),TEXT);}
    response.on_hover_text(name)
}

#[derive(Clone,Default)]
struct PrivacyPicker {
    apps:Vec<crate::privacy::PrivacyApp>,
    query:String,
    error:Option<String>,
    refresh:bool,
}

fn preview_privacy_apps(existing:&[crate::privacy::PrivacyApp])->Option<Vec<crate::privacy::PrivacyApp>> {
    #[cfg(debug_assertions)]
    if std::env::var_os("SCREENMATE_PREVIEW").as_deref()==Some(std::ffi::OsStr::new("1")) {
        if !existing.is_empty(){return Some(existing.to_vec());}
        if std::env::var_os("SCREENMATE_PREVIEW_LONG").as_deref()==Some(std::ffi::OsStr::new("1")) {
            return Some((1..=18).map(|n|crate::privacy::PrivacyApp {
                executable:format!("preview-app-{n:02}.exe"),configured:n==1,selected:n<=3,running:n>2,
            }).collect());
        }
        return Some(vec![
            crate::privacy::PrivacyApp {executable:"browser.exe".into(),configured:false,selected:false,running:true},
            crate::privacy::PrivacyApp {executable:"editor.exe".into(),configured:false,selected:false,running:true},
        ]);
    }
    let _=existing;
    None
}

// Debug-only, opt-in visual regression aid: the viewport reads its own GPU
// framebuffer. This never captures the desktop or another application's pixels.
#[cfg(debug_assertions)]
fn preview_framebuffer(ctx:&egui::Context,state:&Shared) {
    if std::env::var_os("SCREENMATE_PREVIEW").as_deref()!=Some(std::ffi::OsStr::new("1")) {return;}
    let mode=match std::env::var("SCREENMATE_PREVIEW_SHOT") {Ok(v) if v=="main"||v=="privacy"=>v,_=>return};
    let id=egui::Id::new("preview-frame-count");
    let frame=ctx.data(|d|d.get_temp::<u32>(id).unwrap_or(0));
    let language=crate::i18n::Language::from_code(&std::env::var("SCREENMATE_PREVIEW_LANG").unwrap_or_default());
    if frame==0 {
        if let Ok(mut s)=state.lock() {s.language=language;}
        if std::env::var_os("SCREENMATE_PREVIEW_SIZE").as_deref()==Some(std::ffi::OsStr::new("small")) {ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(420.0,440.0)));}
        if mode=="privacy" {
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::new("privacy-picker-open"),true);
                d.insert_temp(egui::Id::new("privacy-picker-data"),PrivacyPicker {refresh:true,..Default::default()});
            });
        }
    }
    if frame==10 {ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);}
    let screenshot=ctx.input(|input|input.events.iter().find_map(|event|if let egui::Event::Screenshot{image,..}=event {Some(image.clone())}else{None}));
    if let Some(image)=screenshot {
        let bytes:Vec<u8>=image.pixels.iter().flat_map(|pixel|pixel.to_array()).collect();
        let list=if std::env::var_os("SCREENMATE_PREVIEW_LONG").as_deref()==Some(std::ffi::OsStr::new("1")){"-long"}else{""};
        let size=if std::env::var_os("SCREENMATE_PREVIEW_SIZE").as_deref()==Some(std::ffi::OsStr::new("small")){"-small"}else{""};
        let path=std::env::temp_dir().join(format!("screenmate-preview-{}-{}{}{}.png",mode,language.code(),list,size));
        if let Err(e)=image::save_buffer_with_format(path,&bytes,image.size[0] as u32,image.size[1] as u32,image::ColorType::Rgba8,image::ImageFormat::Png) {eprintln!("Preview framebuffer: {e}");}
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    ctx.data_mut(|d|d.insert_temp(id,frame.saturating_add(1)));
}

fn refresh_privacy(picker:&mut PrivacyPicker,state:&Shared,language:crate::i18n::Language) {
    if let Some(apps)=preview_privacy_apps(&picker.apps){picker.apps=apps;picker.refresh=false;return;}
    let config=match state.lock(){Ok(s)=>s.config.clone(),Err(_)=>{picker.error=Some(language.text("Не удалось прочитать настройки.","Could not read settings.").into());picker.refresh=false;return;}};
    match crate::privacy::apps(&config,language) {
        Ok(apps)=>{picker.apps=apps;picker.error=None;},
        Err(error)=>picker.error=Some(error),
    }
    picker.refresh=false;
}

fn save_privacy_choice(state:&Shared,process:&str,checked:bool,language:crate::i18n::Language)->Result<(),String> {
    let mut state=state.lock().map_err(|_|language.text("Не удалось открыть настройки.","Could not open settings.").to_owned())?;
    crate::privacy::set_user_mask(&mut state,process,checked)
}

fn app_display_name(executable:&str,language:crate::i18n::Language)->String {
    match executable.to_lowercase().as_str() {
        "browser.exe"=>language.text("Браузер","Browser").into(),
        "editor.exe"=>language.text("Редактор","Editor").into(),
        "chrome.exe"=>"Google Chrome".into(),
        "msedge.exe"=>"Microsoft Edge".into(),
        "firefox.exe"=>"Firefox".into(),
        "code.exe"=>"Visual Studio Code".into(),
        "telegram.exe"=>"Telegram".into(),
        _=>executable.to_owned(),
    }
}

fn fitted_text(ui:&egui::Ui,text:&str,font:FontId,max_width:f32)->String {
    let width=|value:String|ui.painter().layout_no_wrap(value,font.clone(),TEXT).size().x;
    if width(text.to_owned())<=max_width {return text.to_owned();}
    let chars:Vec<char>=text.chars().collect();let mut low=0usize;let mut high=chars.len();
    while low<high {
        let mid=(low+high+1)/2;let candidate=format!("{}…",chars[..mid].iter().collect::<String>());
        if width(candidate)<=max_width {low=mid;} else {high=mid-1;}
    }
    if low==0 {"…".into()}else{format!("{}…",chars[..low].iter().collect::<String>())}
}

/// A visual hint that the list continues. It is drawn *inside* the already
/// clipped viewport and does not create a widget or intercept input.
fn list_edge_fade(ui:&egui::Ui,viewport:Rect,color:Color32,at_top:bool) {
    let height=12.0_f32.min(viewport.height()*0.25);
    if height<=0.0 {return;}
    let rect=if at_top {Rect::from_min_max(viewport.min,egui::pos2(viewport.right(),viewport.top()+height))}
        else {Rect::from_min_max(egui::pos2(viewport.left(),viewport.bottom()-height),viewport.max)};
    let clear=Color32::TRANSPARENT;
    let (top,bottom)=if at_top {(color,clear)}else{(clear,color)};
    let mut mesh=egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(),top);mesh.colored_vertex(rect.right_top(),top);
    mesh.colored_vertex(rect.left_bottom(),bottom);mesh.colored_vertex(rect.right_bottom(),bottom);
    mesh.add_triangle(0,1,2);mesh.add_triangle(2,1,3);
    ui.painter().with_clip_rect(viewport).add(egui::Shape::mesh(mesh));
}

/// One accessible row represents an executable, rather than an individual window.
/// Base rules are visibly checked but cannot be removed from this list.
fn privacy_app_row(ui:&mut egui::Ui,app:&crate::privacy::PrivacyApp,theme:Theme,language:crate::i18n::Language)->Option<bool> {
    let selected=app.configured||app.selected;
    let inner=ui.add_enabled_ui(!app.configured,|ui| {
        let (rect,response)=ui.allocate_exact_size(Vec2::new(ui.available_width(),52.0),Sense::click());
        #[cfg(test)]
        update_privacy_layout(ui.ctx(),|geometry|geometry.rows.push((rect,ui.clip_rect())));
        let name=app_display_name(&app.executable,language);
        let accessible=if app.configured {format!("{}: {} ({})",language.text("Скрывать","Hide"),app.executable,language.text("базовое правило","base rule"))} else {format!("{}: {}",language.text("Скрывать","Hide"),app.executable)};
        crate::access::checkbox(&response,&accessible,selected);
        if app.configured {ui.ctx().accesskit_node_builder(response.id,|node|{node.set_disabled();node.clear_actions();node.clear_default_action_verb();});}
        let p=ui.painter();
        p.rect_filled(rect,12.0,if selected{theme.selected}else if response.hovered(){theme.field}else{theme.panel});
        p.rect_stroke(rect,12.0,Stroke::new(if response.has_focus(){2.0_f32}else{1.0_f32},if response.has_focus()||selected{theme.accent}else{theme.edge}));
        let mark=Rect::from_min_size(rect.min+Vec2::new(10.0,10.0),Vec2::splat(32.0));
        p.rect_filled(mark,9.0,theme.field);
        icons::paint(p,Rect::from_center_size(mark.center(),Vec2::splat(18.0)),if selected{Icon::Shield}else{Icon::Window},if selected{theme.accent}else{MUTED});
        let clip=Rect::from_min_max(egui::pos2(rect.left()+52.0,rect.top()),egui::pos2(rect.right()-46.0,rect.bottom()));
        let p=p.with_clip_rect(clip);
        let text_width=(clip.width()-4.0).max(0.0);
        let fitted_name=fitted_text(ui,&name,heavy(12.0),text_width);
        p.text(egui::pos2(rect.left()+52.0,rect.top()+17.0),Align2::LEFT_CENTER,&fitted_name,heavy(12.0),TEXT);
        let detail=if app.configured {language.text("Базовое правило", "Base rule")} else if selected {language.text("Добавлено вами", "Added by you")} else {language.text("Открыто сейчас", "Running now")};
        let full_detail=format!("{}  ·  {}",detail,app.executable);
        let fitted_detail=fitted_text(ui,&full_detail,regular(10.5),text_width);
        p.text(egui::pos2(rect.left()+52.0,rect.top()+36.0),Align2::LEFT_CENTER,&fitted_detail,regular(10.5),MUTED);
        let check=Rect::from_center_size(egui::pos2(rect.right()-24.0,rect.center().y),Vec2::splat(20.0));
        let p=ui.painter();p.rect_filled(check,6.0,if selected{theme.accent}else{theme.bg});
        p.rect_stroke(check,6.0,Stroke::new(1.0_f32,if selected{theme.accent}else{theme.edge}));
        if selected {icons::paint(p,check.shrink(3.0),Icon::Check,Color32::from_rgb(12,25,28));}
        (if response.clicked(){Some(!selected)}else{None},fitted_name!=name||fitted_detail!=full_detail)
    });
    if inner.inner.1 {inner.response.on_hover_text(&app.executable);}
    inner.inner.0
}

fn privacy_dialog(ctx:&egui::Context,state:&Shared,picker:&mut PrivacyPicker,open:&mut bool,theme:Theme,language:crate::i18n::Language) {
    if picker.refresh {refresh_privacy(picker,state,language);}
    #[cfg(test)]
    ctx.data_mut(|data|data.insert_temp(egui::Id::new("privacy-layout-test"),privacy_layout_tests::PrivacyLayoutGeometry::default()));
    // Reserve a 16-point margin around the dialog, even at the smallest viewport.
    let content_width=(ctx.screen_rect().width()-64.0).clamp(240.0,432.0);
    let content_height=(ctx.screen_rect().height()-64.0).clamp(300.0,488.0);
    let mut update=None;let mut choose=false;let mut close=false;
    let title_text=language.text("Скрывать приложения","Hide apps");
    let frame=egui::Frame::none().fill(theme.bg).rounding(20.0)
        .stroke(Stroke::new(1.0_f32,theme.edge)).inner_margin(egui::Margin::same(metrics::MD));
    egui::Window::new(title_text).id(egui::Id::new("privacy-picker-window"))
        .open(open).title_bar(false).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER,Vec2::ZERO)
        .fixed_size(Vec2::new(content_width,content_height)).frame(frame).show(ctx,|ui| {
            ui.set_width(content_width);
            crate::access::group(ui,"privacy-dialog",title_text,|ui| {
                ui.horizontal(|ui| {
                    icons::add(ui,Icon::Shield,20.0,theme.accent);
                    let remaining=(ui.available_width()-40.0).max(80.0);
                    ui.add_sized([remaining,28.0],egui::Label::new(title(title_text,19.0)).truncate(true));
                    let (rect,response)=ui.allocate_exact_size(Vec2::splat(28.0),Sense::click());
                    crate::access::button(&response,language.text("Закрыть","Close"));
                    if response.hovered(){ui.painter().rect_filled(rect,8.0,theme.field);}
                    if response.has_focus(){ui.painter().rect_stroke(rect,8.0,Stroke::new(2.0_f32,theme.accent));}
                    icons::paint(ui.painter(),rect.shrink(6.0),Icon::Close,MUTED);
                    if response.clicked(){close=true;}
                });
                ui.add_space(metrics::XS);
                ui.add(egui::Label::new(label(language.text(
                    "Выберите приложения для маски. Правила по названиям окон также действуют.",
                    "Choose apps to mask. Window-title rules still apply."),12.0)).wrap(true));
                ui.add_space(metrics::SM);
                let search_name=language.text("Поиск приложения","Search apps");
                let search_id=ui.make_persistent_id("privacy-search");
                let has_focus=ctx.memory(|memory|memory.has_focus(search_id));
                let search_frame=egui::Frame::none().fill(theme.field).rounding(12.0)
                    .stroke(Stroke::new(if has_focus {2.0_f32}else{1.0_f32},if has_focus{theme.accent}else{theme.edge}))
                    .inner_margin(egui::Margin::symmetric(12.0,metrics::SM)).show(ui,|ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let (icon,_)=ui.allocate_exact_size(Vec2::splat(16.0),Sense::hover());
                            let center=icon.center()-Vec2::splat(1.0);
                            ui.painter().circle_stroke(center,5.0,Stroke::new(1.4_f32,MUTED));
                            ui.painter().line_segment([center+Vec2::new(3.5,3.5),center+Vec2::new(6.5,6.5)],Stroke::new(1.4_f32,MUTED));
                            let edit=egui::TextEdit::singleline(&mut picker.query).id(search_id).frame(false)
                                .font(regular(13.0)).desired_width(ui.available_width()).hint_text(search_name);
                            let response=ui.add_sized([ui.available_width(),24.0],edit);
                            response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::TextEdit,search_name));
                        });
                    });
                #[cfg(test)] update_privacy_layout(ctx,|g|g.search=Some(search_frame.response.rect));
                #[cfg(not(test))] let _=search_frame;
                ui.add_space(metrics::SM);
                let query=picker.query.trim().to_lowercase();
                // Reserve the full footer before assigning the remaining vertical space to the list.
                let error_reserve=if picker.error.is_some(){24.0}else{0.0};
                let list_height=(ui.available_height()-120.0-error_reserve).clamp(64.0,240.0);
                egui::Frame::none().fill(theme.panel).rounding(12.0)
                    .stroke(Stroke::new(1.0_f32,theme.edge)).inner_margin(egui::Margin::same(metrics::SM)).show(ui,|ui| {
                        ui.set_min_width(ui.available_width());
                        let scroll=egui::ScrollArea::vertical().id_source("privacy-app-list")
                            .max_height(list_height).min_scrolled_height(list_height).auto_shrink([false,false]);
                        #[cfg(debug_assertions)]
                        let scroll=if std::env::var_os("SCREENMATE_PREVIEW").as_deref()==Some(std::ffi::OsStr::new("1")) &&
                           std::env::var_os("SCREENMATE_PREVIEW_SCROLL").as_deref()==Some(std::ffi::OsStr::new("1")) {
                            scroll.vertical_scroll_offset(33.0)
                        } else {scroll};
                        let result=scroll.show_viewport(ui,|ui,viewport| {
                            // egui expands scroll clipping by default and may skip clipping on
                            // the first frame. Intersect with the actual viewport every frame.
                            let viewport_screen=Rect::from_min_size(ui.max_rect().min+viewport.min.to_vec2(),viewport.size());
                            ui.set_clip_rect(ui.clip_rect().intersect(viewport_screen));
                            let matches=|app:&&crate::privacy::PrivacyApp|query.is_empty()||app.executable.to_lowercase().contains(&query)||app_display_name(&app.executable,language).to_lowercase().contains(&query);
                            let protected:Vec<_>=picker.apps.iter().filter(|app|app.configured||app.selected).filter(matches).collect();
                            let running:Vec<_>=picker.apps.iter().filter(|app|!app.configured&&!app.selected&&app.running).filter(matches).collect();
                            if !protected.is_empty() {
                                ui.label(RichText::new(format!("{}  {}",language.text("Правила маскирования","Masking rules"),protected.len())).font(heavy(12.0)).color(MUTED));
                                ui.add_space(metrics::XS);
                                for app in protected {if let Some(checked)=privacy_app_row(ui,app,theme,language){update=Some((app.executable.clone(),checked));}}
                                ui.add_space(metrics::SM);
                            }
                            if !running.is_empty() {
                                ui.label(RichText::new(language.text("Открыты сейчас","Running now")).font(heavy(12.0)).color(MUTED));
                                ui.add_space(metrics::XS);
                                for app in running {if let Some(checked)=privacy_app_row(ui,app,theme,language){update=Some((app.executable.clone(),checked));}}
                            }
                            if picker.apps.iter().filter(matches).count()==0 {ui.label(label(if query.is_empty(){language.text("Открытых приложений пока нет.","No running apps found.")}else{language.text("Ничего не найдено.","No matching apps.")},13.0));}
                        });
                        if result.state.offset.y>0.5 {list_edge_fade(ui,result.inner_rect,theme.panel,true);}
                        if result.state.offset.y+result.inner_rect.height()+0.5<result.content_size.y {list_edge_fade(ui,result.inner_rect,theme.panel,false);}
                        #[cfg(test)] update_privacy_layout(ctx,|g|{g.list=Some(result.inner_rect);g.scroll_id=Some(result.id);g.content_height=result.content_size.y;});
                        #[cfg(not(test))] let _=result;
                    });
                if let Some(error)=&picker.error {
                    let response=ui.add(egui::Label::new(RichText::new(error).font(regular(12.0)).color(Color32::from_rgb(255,157,146))).truncate(true));
                    response.on_hover_text(error);
                }
                ui.add_space(metrics::SM);
                let choose_response=primary(ui,Vec2::new(ui.available_width(),40.0),Icon::Window,language.text("Выбрать приложение…","Choose an app…"),theme);
                #[cfg(test)] update_privacy_layout(ctx,|g|g.choose=Some(choose_response.rect));
                if choose_response.clicked(){choose=true;}
                let footer=ui.horizontal(|ui| {
                    if ui.link(language.text("Обновить список","Refresh list")).clicked(){picker.refresh=true;}
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                        let done=egui::Button::new(RichText::new(language.text("Готово","Done")).font(regular(13.0)).color(TEXT)).fill(theme.field).rounding(10.0).stroke(Stroke::NONE);
                        if ui.add_sized([96.0,36.0],done).clicked(){close=true;}
                    });
                });
                #[cfg(test)] update_privacy_layout(ctx,|g|g.footer=Some(footer.response.rect));
                #[cfg(not(test))] let _=footer;
            });
        });
    if ctx.input(|input|input.key_pressed(egui::Key::Escape)) {close=true;}
    if let Some((name,checked))=update {
        match save_privacy_choice(state,&name,checked,language) {
            Ok(())=>{if let Some(app)=picker.apps.iter_mut().find(|a|a.executable.eq_ignore_ascii_case(&name)){app.selected=checked;}picker.error=None;},
            Err(error)=>picker.error=Some(error),
        }
    }
    // The native picker must be called without the application state mutex held.
    if choose {match crate::privacy::choose_executable(language) {
        Ok(Some(name))=>match save_privacy_choice(state,&name,true,language) {
            Ok(())=>{
                if let Some(app)=picker.apps.iter_mut().find(|a|a.executable.eq_ignore_ascii_case(&name)){app.selected=true;}
                else {picker.apps.push(crate::privacy::PrivacyApp {executable:name,configured:false,selected:true,running:false});}
                picker.error=None;picker.refresh=true;
            },
            Err(error)=>picker.error=Some(error),
        },
        Ok(None)=>{},
        Err(error)=>picker.error=Some(error),
    }}
    if close {*open=false;}
}

// Material-inspired track and thumb with a short, interruptible transition.
fn switch(ui:&mut egui::Ui,checked:&mut bool,theme:Theme,name:&str)->egui::Response {
    let (rect,mut response)=ui.allocate_exact_size(Vec2::new(48.0,28.0),Sense::click());
    if response.clicked(){*checked=!*checked;response.mark_changed();}
    crate::access::checkbox(&response,name,*checked);
    if !response.enabled() {ui.ctx().accesskit_node_builder(response.id,|node|{node.set_disabled();node.clear_actions();node.clear_default_action_verb();});}
    let t=ui.ctx().animate_bool(response.id,*checked);
    let off=theme.field; let on=theme.accent;
    let mix=|a:u8,b:u8|((a as f32)*(1.0-t)+(b as f32)*t).round() as u8;
    let fill=Color32::from_rgb(mix(off.r(),on.r()),mix(off.g(),on.g()),mix(off.b(),on.b()));
    ui.painter().rect_filled(rect,14.0,fill);
    ui.painter().rect_stroke(rect,14.0,Stroke::new(if response.has_focus(){2.0_f32}else{1.0_f32},if *checked||response.has_focus(){theme.accent}else{theme.edge}));
    let center=egui::pos2(rect.left()+14.0+t*20.0,rect.center().y);
    ui.painter().circle_filled(center,if *checked{10.0}else{8.0},if *checked{Color32::from_rgb(13,25,30)}else{MUTED});
    if *checked {icons::paint(ui.painter(),Rect::from_center_size(center,Vec2::splat(12.0)),Icon::Check,theme.accent);}
    response
}

#[derive(Clone,Default)]
struct AutostartControl {
    enabled:Option<bool>,
    status_error:Option<String>,
    action_error:Option<(crate::i18n::Language,String)>,
    last_check:Option<std::time::Instant>,
    language:crate::i18n::Language,
}

fn preview_mode()->bool {
    cfg!(debug_assertions)&&std::env::var_os("SCREENMATE_PREVIEW").as_deref()==Some(std::ffi::OsStr::new("1"))
}

fn refresh_autostart(control:&mut AutostartControl,language:crate::i18n::Language) {
    control.last_check=Some(std::time::Instant::now());control.language=language;
    if preview_mode() {if control.enabled.is_none(){control.enabled=Some(true);}control.status_error=None;return;}
    match crate::autostart::status(language) {
        Ok(enabled)=>{control.enabled=Some(enabled);control.status_error=None;},
        Err(error)=>{control.enabled=None;control.status_error=Some(error);},
    }
}

fn autostart_row(ui:&mut egui::Ui,control:&AutostartControl,theme:Theme,language:crate::i18n::Language)->Option<bool> {
    let mut desired=None;
    ui.horizontal(|ui| {
        let hint=language.text("Windows может отдельно отключить пункт в диспетчере задач.","Windows can separately disable this entry in Task Manager.");
        icons::add(ui,Icon::Play,18.0,theme.accent);
        ui.label(RichText::new(language.text("Запускать при входе","Start at sign-in")).font(regular(13.0)).color(TEXT)).on_hover_text(hint);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
            let mut enabled=control.enabled.unwrap_or(false);
            let response=ui.add_enabled_ui(control.enabled.is_some(),|ui|switch(ui,&mut enabled,theme,language.text("Запускать при входе","Start at sign-in"))).inner;
            let clicked=response.clicked();response.on_hover_text(hint);
            if clicked {desired=Some(enabled);}
        });
    });
    desired
}

fn monitor_card(ui:&mut egui::Ui,m:&Monitor,index:usize,selected:bool,theme:Theme,width:f32,language:crate::i18n::Language)->bool {
    let (r,response)=ui.allocate_exact_size(Vec2::new(width,96.0),Sense::click());let p=ui.painter();
    crate::access::radio(&response,&format!("{} {}, {} {} {}{}",language.text("Монитор","Monitor"),index+1,m.width,language.text("на","by"),m.height,if m.primary {language.text(", основной",", primary")}else{""}),selected);
    p.rect_filled(r,16.0,if selected{theme.selected}else if response.hovered(){theme.field}else{theme.panel});
    p.rect_stroke(r,16.0,Stroke::new(if response.has_focus(){2.0_f32}else if selected{1.5_f32}else{1.0_f32},if selected||response.has_focus(){theme.accent}else{theme.edge}));
    let max=Vec2::new((width-32.0).max(32.0),40.0);
    let aspect=(m.width as f32 / m.height.max(1) as f32).max(0.01);
    let screen=if max.x/max.y>aspect {Vec2::new(max.y*aspect,max.y)} else {Vec2::new(max.x,max.x/aspect)};
    let screen_r=Rect::from_center_size(egui::pos2(r.center().x,r.top()+28.0),screen);
    p.rect_filled(screen_r,6.0,theme.bg);
    p.rect_stroke(screen_r,6.0,Stroke::new(1.5_f32,if selected{theme.accent}else{theme.edge}));
    // A simple miniature of the display, scaled to the actual aspect ratio.
    p.rect_filled(Rect::from_min_size(screen_r.min+Vec2::new(5.0,5.0),Vec2::new((screen.x*0.36).max(4.0),(screen.y*0.12).max(3.0))),2.0,if selected{theme.selected}else{theme.field});
    p.line_segment([egui::pos2(screen_r.center().x,screen_r.bottom()+1.0),egui::pos2(screen_r.center().x,screen_r.bottom()+5.0)],Stroke::new(2.0_f32,theme.edge));
    p.line_segment([egui::pos2(screen_r.center().x-10.0,screen_r.bottom()+6.0),egui::pos2(screen_r.center().x+10.0,screen_r.bottom()+6.0)],Stroke::new(2.0_f32,theme.edge));
    p.text(egui::pos2(r.left()+16.0,r.bottom()-32.0),Align2::LEFT_CENTER,format!("{} {}",language.text("Монитор","Monitor"),index+1),heavy(12.0),TEXT);
    let detail=if m.primary {format!("{} × {}  ·  {}",m.width,m.height,language.text("основной","primary"))}else{format!("{} × {}",m.width,m.height)};
    p.text(egui::pos2(r.left()+16.0,r.bottom()-12.0),Align2::LEFT_CENTER,detail,regular(10.0),MUTED);
    if selected {let check=Rect::from_min_size(egui::pos2(r.right()-28.0,r.bottom()-40.0),Vec2::splat(16.0));icons::paint(p,check,Icon::Check,theme.accent);}
    response.clicked()
}

impl eframe::App for ScreenmateApp {
    fn update(&mut self,ctx:&egui::Context,_frame:&mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(66));
        #[cfg(debug_assertions)]
        preview_framebuffer(ctx,&self.state);
        if self.monitor_cache_at.elapsed()>=Duration::from_secs(2) {self.monitor_cache=monitors().unwrap_or_default();self.monitor_cache_at=std::time::Instant::now();}
        if let Some(tray)=&mut self.tray {
            while let Some(action)=tray.poll(){match action {
                tray::TrayAction::Open=>{ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));ctx.send_viewport_cmd(egui::ViewportCommand::Focus);},
                tray::TrayAction::TogglePause=>{},
                tray::TrayAction::ToggleGlow=>{},
                tray::TrayAction::NextMonitor=>{self.mode=0;},
                tray::TrayAction::Exit=>{self.exiting=true;ctx.send_viewport_cmd(egui::ViewportCommand::Close);}
            }}
            let s=self.state.lock().unwrap();let status=if s.paused{tray::TrayState::Paused}else if !s.error.is_empty(){tray::TrayState::Error}else{tray::TrayState::Watching};let _=tray.set_state(status);let monitor=self.monitor_cache.iter().position(|m|s.monitor.map_or(m.primary,|id|m.id==id)).map(|i|format!("Монитор {}",i+1)).unwrap_or_else(||"Монитор недоступен".into());tray.set_details(s.glow_enabled,&monitor);
        }
        if !self.exiting&&self.tray.is_some()&&self.state.lock().unwrap().minimize_to_tray&&ctx.input(|i|i.viewport().close_requested()){ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));}
        if let Some(overlay)=&mut self.overlay {
            if let Some(e)=overlay.poll_error(){self.overlay_error=e;}
        }
        let (theme,language)={let s=self.state.lock().unwrap();(Theme::new(s.accent),s.language)};
        let autostart_id=egui::Id::new("autostart-control");
        let mut autostart=ctx.data(|data|data.get_temp::<AutostartControl>(autostart_id).unwrap_or_default());
        if autostart.last_check.map_or(true,|at|at.elapsed()>=Duration::from_secs(2))||autostart.language!=language {refresh_autostart(&mut autostart,language);}
        let mut pending_autostart=None;
        let preview=cfg!(debug_assertions)&&std::env::var_os("SCREENMATE_PREVIEW").as_deref()==Some(std::ffi::OsStr::new("1"));
        let window_title=if preview {language.text("Скринмейт — предпросмотр","Screenmate — preview")}else{language.text("Скринмейт","Screenmate")};
        let title_id=egui::Id::new("screenmate-window-title");
        if ctx.data(|data|data.get_temp::<String>(title_id)).as_deref()!=Some(window_title) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(window_title.into()));
            ctx.data_mut(|data|data.insert_temp(title_id,window_title.to_owned()));
        }
        let mut style=(*ctx.style()).clone();
        style.visuals.panel_fill=theme.bg;style.visuals.window_fill=theme.panel;style.visuals.extreme_bg_color=theme.bg;
        style.visuals.widgets.inactive.bg_fill=theme.field;style.visuals.widgets.inactive.weak_bg_fill=theme.field;
        style.visuals.widgets.hovered.bg_fill=theme.selected;style.visuals.widgets.hovered.weak_bg_fill=theme.selected;
        style.visuals.widgets.active.bg_fill=theme.selected;style.visuals.widgets.active.weak_bg_fill=theme.selected;
        style.visuals.selection.bg_fill=theme.selected;style.visuals.selection.stroke=Stroke::new(1.0_f32,theme.accent);ctx.set_style(style);
        let open_id=egui::Id::new("privacy-picker-open");
        let mut privacy_open=ctx.data(|data|data.get_temp::<bool>(open_id).unwrap_or(false));
        let picker_id=egui::Id::new("privacy-picker-data");
        let mut privacy_picker=ctx.data(|data|data.get_temp::<PrivacyPicker>(picker_id).unwrap_or_default());
        egui::CentralPanel::default().frame(egui::Frame::none().fill(theme.bg)).show(ctx,|ui| {
            if privacy_open {ui.set_enabled(false);}
            egui::ScrollArea::vertical().auto_shrink([false,false]).show(ui,|ui| {
                egui::Frame::none().inner_margin(egui::Margin::symmetric(metrics::LG,12.0)).show(ui,|ui| {
                    let width=ui.available_width().min(660.0);ui.set_width(width);
                    let mut s=self.state.lock().unwrap();
                    // Wordmark, live state and pause are always visible near each other.
                    ui.add_space(metrics::XS);
                    ui.horizontal(|ui| {
                        ui.add(egui::Image::new((self.brand.id(),Vec2::splat(metrics::BUTTON))));
                        ui.add_space(metrics::XS);
                        ui.label(title(language.text("Скринмейт","Screenmate"),28.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                            crate::access::group(ui,"language",language.text("Язык","Language"),|ui| {
                                ui.horizontal(|ui| {
                                    if language_option(ui,"RU",language.text("Русский язык","Russian language"),s.language==crate::i18n::Language::Ru,theme) {s.language=crate::i18n::Language::Ru;save_appearance(&s);}
                                    if language_option(ui,"EN",language.text("Английский язык","English language"),s.language==crate::i18n::Language::En,theme) {s.language=crate::i18n::Language::En;save_appearance(&s);}
                                });
                            });
                        });
                    });
                    ui.add_space(metrics::SM);
                    crate::access::group(ui,"status",language.text("Состояние захвата","Capture status"),|ui| {egui::Frame::none().fill(theme.panel).rounding(metrics::MD).stroke(Stroke::new(1.0_f32,theme.edge)).inner_margin(egui::Margin::symmetric(metrics::MD,metrics::XS)).show(ui,|ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let (dot,_)=ui.allocate_exact_size(Vec2::splat(12.0),Sense::hover());
                            let status_color=if !s.error.is_empty()&&!s.paused{Color32::from_rgb(255,157,146)}else if s.paused{MUTED}else{theme.accent};
                            ui.painter().circle_filled(dot.center(),4.2,status_color);
                            ui.label(RichText::new(if s.paused{language.text("Пауза","Paused")}else if !s.error.is_empty(){language.text("Ничего не вижу","Can't see the screen")}else{language.text("Смотрю","Watching")}).font(heavy(14.0)).color(TEXT));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {if primary(ui,Vec2::new(144.0,36.0),if s.paused{Icon::Play}else{Icon::Pause},if s.paused{language.text("Продолжить","Resume")}else{language.text("Пауза","Pause")},theme).clicked(){s.paused=!s.paused;if s.paused{s.frames.clear();}}});
                        });
                    });});
                    if !self.overlay_error.is_empty() {ui.add_space(metrics::XS);ui.colored_label(Color32::from_rgb(244,190,134),language.text("Подсветка недоступна","Glow unavailable"));}
                    ui.add_space(metrics::XS);
                    crate::access::group(ui,"source",language.text("Что показать","What to share"),|ui| {
                    ui.label(title(language.text("Что показать","What to share"),18.0));ui.add_space(metrics::XS);
                    let w=((ui.available_width()-metrics::MD)/3.0).max(92.0);
                    ui.horizontal(|ui| {
                        for (mode,icon,text) in [(0,Icon::Monitor,language.text("Монитор","Screen")),(1,Icon::Window,language.text("Окно","Window")),(2,Icon::Area,language.text("Область","Region"))] {
                            if text_and_icon(ui,Vec2::new(w,metrics::BUTTON),icon,text,theme,self.mode==mode).clicked(){
                                self.mode=mode;
                                match mode {0=>{s.window=None;s.region=None;},1=>{s.window=None;s.region=Some([0,0,0,0]);},_=>{s.window=None;s.region=Some([self.region_x,self.region_y,self.region_w,self.region_h]);}}
                            }
                        }
                    });
                    ui.add_space(metrics::SM);
                    match self.mode {
                        0=>{
                            let monitors=&self.monitor_cache;
                            if monitors.is_empty(){ui.label(label(language.text("Мониторы не найдены","No screens found"),13.0));}
                            let cols=if ui.available_width()>=420.0 {monitors.len().clamp(1,3)}else{monitors.len().clamp(1,2)};
                            let card_width=((ui.available_width()-metrics::SM*(cols-1) as f32)/cols as f32).min(metrics::CARD_MAX).max(112.0);
                            ui.horizontal_wrapped(|ui| {for (index,m) in monitors.iter().enumerate(){
                                let selected=s.monitor.map_or(m.primary,|id|m.id==id);
                                if monitor_card(ui,m,index,selected,theme,card_width,language){s.monitor=Some(m.id);s.window=None;s.region=None;}
                            }});
                        },
                        1=>{
                            let ws=windows().unwrap_or_default();
                            let selected=s.window.and_then(|id|ws.iter().find(|w|w.id==id)).map(|w|w.title.clone()).unwrap_or_else(||language.text("Выберите окно","Choose a window").into());
                            let choice=egui::ComboBox::from_id_source("source-window").width((ui.available_width()-metrics::MD).max(100.0)).selected_text(selected.clone()).show_ui(ui,|ui| {
                                for w in ws.iter().filter(|w|!w.title.is_empty()){if ui.selectable_label(s.window==Some(w.id),&w.title).clicked(){
                                    let center_x=w.x as i64+w.width as i64/2;let center_y=w.y as i64+w.height as i64/2;
                                    if let Some(m)=self.monitor_cache.iter().find(|m|center_x>=m.x as i64&&center_y>=m.y as i64&&center_x<(m.x as i64+m.width as i64)&&center_y<(m.y as i64+m.height as i64)){s.monitor=Some(m.id);}
                                    s.window=Some(w.id);s.region=None;
                                }}
                            });
                            crate::access::combobox(&choice.response,language.text("Окно для захвата","Window to capture"),&selected);
                        },
                        _=>{
                            egui::Frame::none().fill(theme.panel).rounding(metrics::MD).inner_margin(metrics::MD).show(ui,|ui| {ui.set_min_width(ui.available_width());ui.horizontal_wrapped(|ui| {
                                for (name,positive,value) in [("X",false,&mut self.region_x),("Y",false,&mut self.region_y),(language.text("Ширина","Width"),true,&mut self.region_w),(language.text("Высота","Height"),true,&mut self.region_h)] {let caption=ui.label(label(name,12.0));ui.add(egui::DragValue::new(value).speed(1).clamp_range(if positive{1..=100000}else{0..=100000})).labelled_by(caption.id);}
                            });});
                            s.region=Some([self.region_x,self.region_y,self.region_w,self.region_h]);
                        }
                    }
                    });
                    ui.horizontal(|ui| {icons::add(ui,Icon::Palette,18.0,theme.accent);ui.label(title(language.text("Цвет","Color"),17.0));});
                    let compact_privacy=ui.available_width()<464.0;
                    ui.horizontal(|ui| {
                        crate::access::group(ui,"accent",language.text("Цвет акцента","Accent color"),|ui| {
                            ui.horizontal(|ui| {for (color,name) in PALETTE.into_iter().zip([language.text("Голубой","Cyan"),language.text("Зелёный","Green"),language.text("Синий","Blue"),language.text("Фиолетовый","Purple"),language.text("Коралловый","Coral"),language.text("Золотой","Gold")]) {
                                let fill=Color32::from_rgb(color[0],color[1],color[2]);
                                let (rect,response)=ui.allocate_exact_size(Vec2::splat(metrics::SWATCH),Sense::click());
                                crate::access::radio(&response,&format!("{}: {name}",language.text("Цвет акцента","Accent color")),s.accent==color);
                                ui.painter().circle_filled(rect.center(),13.5,fill);
                                if s.accent==color {ui.painter().circle_stroke(rect.center(),17.0,Stroke::new(1.3_f32,fill));icons::paint(ui.painter(),Rect::from_center_size(rect.center(),Vec2::splat(14.0)),Icon::Check,Color32::from_rgb(12,25,28));}
                                if response.has_focus(){ui.painter().circle_stroke(rect.center(),18.0,Stroke::new(2.0_f32,TEXT));}
                                if response.clicked(){s.accent=color;save_appearance(&s);}
                            }});
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                            if privacy_button(ui,theme,compact_privacy,language).clicked(){privacy_open=true;privacy_picker.refresh=true;privacy_picker.query.clear();}
                        });
                    });
                    ui.add_space(metrics::SM);
                    crate::access::group(ui,"settings",language.text("Настройки","Settings"),|ui| {
                    ui.label(title(language.text("Настройки","Settings"),17.0)); ui.add_space(metrics::XS);
                    egui::Frame::none().fill(theme.panel).rounding(metrics::MD).stroke(Stroke::new(1.0_f32,theme.edge)).inner_margin(egui::Margin::symmetric(metrics::MD,metrics::SM)).show(ui,|ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            icons::add(ui,Icon::Sparkle,18.0,theme.accent);ui.label(RichText::new(language.text("Свечение экрана","Screen glow")).font(regular(13.0)).color(TEXT));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {if switch(ui,&mut s.glow_enabled,theme,language.text("Свечение экрана","Screen glow")).clicked(){s.glow_preference=s.glow_enabled;save_appearance(&s);}});
                        });
                        ui.add_space(metrics::XS);
                        ui.horizontal(|ui| {
                            icons::add(ui,Icon::Minimize,18.0,theme.accent);ui.label(RichText::new(language.text("Сворачивать в трей","Close to tray")).font(regular(13.0)).color(TEXT));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {if switch(ui,&mut s.minimize_to_tray,theme,language.text("Сворачивать в трей","Close to tray")).clicked(){save_appearance(&s);}});
                        });
                        ui.add_space(metrics::XS);
                        if let Some(desired)=autostart_row(ui,&autostart,theme,language){pending_autostart=Some(desired);}
                        let failure=autostart.action_error.as_ref().map(|(error_language,error)|if *error_language==language {error.as_str()} else {language.text("Не удалось изменить автозапуск.","Could not change startup settings.")})
                            .or(autostart.status_error.as_deref())
                            .or_else(||self.autostart_error.as_ref().map(|_|language.text("Не удалось настроить автозапуск.","Could not configure startup.")));
                        if let Some(error)=failure {
                            let error_label=ui.add(egui::Label::new(RichText::new(error).font(regular(11.0)).color(Color32::from_rgb(255,157,146))).wrap(true));
                            if autostart.action_error.is_none()&&autostart.status_error.is_none(){if let Some(detail)=&self.autostart_error {error_label.on_hover_text(detail);}}
                            if autostart.status_error.is_some()&&ui.small_button(language.text("Выключить автозапуск","Turn off startup")).clicked(){pending_autostart=Some(false);}
                        }
                    });
                    });
                });
            });
        });
        if let Some(enabled)=pending_autostart {
            if preview_mode() {
                autostart.enabled=Some(enabled);autostart.status_error=None;autostart.action_error=None;self.autostart_error=None;
            } else {
                match crate::autostart::set(enabled,language) {
                    Ok(())=>{autostart.action_error=None;self.autostart_error=None;refresh_autostart(&mut autostart,language);},
                    Err(error)=>{refresh_autostart(&mut autostart,language);autostart.action_error=Some((language,error));},
                }
            }
        }
        ctx.data_mut(|data|data.insert_temp(autostart_id,autostart));
        if privacy_open {privacy_dialog(ctx,&self.state,&mut privacy_picker,&mut privacy_open,theme,language);}
        ctx.data_mut(|data|{data.insert_temp(open_id,privacy_open);data.insert_temp(picker_id,privacy_picker);});
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{accesskit::{Action,ActionRequest,Checked,Role},Event,RawInput};

    #[test]
    fn english_autostart_switch_exposes_state_and_default_action() {
        let ctx=egui::Context::default();ctx.enable_accesskit();let _brand=setup(&ctx);
        let control=AutostartControl {enabled:Some(true),..Default::default()};let mut desired=None;
        let render=|ctx:&egui::Context,desired:&mut Option<bool>| {
            egui::CentralPanel::default().show(ctx,|ui| {*desired=autostart_row(ui,&control,Theme::new([90,197,235]),crate::i18n::Language::En);});
        };
        let output=ctx.run(RawInput::default(),|ctx|render(ctx,&mut desired));
        let update=output.platform_output.accesskit_update.unwrap();
        let (id,node)=update.nodes.iter().find(|(_,node)|node.name()==Some("Start at sign-in")&&node.role()==Role::CheckBox).expect("startup switch");
        assert_eq!(node.checked(),Some(Checked::True));
        let mut input=RawInput::default();input.events.push(Event::AccessKitActionRequest(ActionRequest {action:Action::Default,target:*id,data:None}));
        let _=ctx.run(input,|ctx|render(ctx,&mut desired));
        assert_eq!(desired,Some(false));
    }

    #[test]
    fn unknown_autostart_status_disables_checkbox() {
        let ctx=egui::Context::default();ctx.enable_accesskit();let _brand=setup(&ctx);
        let control=AutostartControl::default();let mut desired=None;
        let render=|ctx:&egui::Context,desired:&mut Option<bool>| {
            egui::CentralPanel::default().show(ctx,|ui| {*desired=autostart_row(ui,&control,Theme::new([90,197,235]),crate::i18n::Language::En);});
        };
        let output=ctx.run(RawInput::default(),|ctx|render(ctx,&mut desired));
        let update=output.platform_output.accesskit_update.unwrap();
        let (id,node)=update.nodes.iter().find(|(_,node)|node.name()==Some("Start at sign-in")&&node.role()==Role::CheckBox).expect("startup switch");
        assert!(node.is_disabled());
        let mut input=RawInput::default();input.events.push(Event::AccessKitActionRequest(ActionRequest {action:Action::Default,target:*id,data:None}));
        let _=ctx.run(input,|ctx|render(ctx,&mut desired));
        assert_eq!(desired,None);
    }

    #[test]
    fn english_privacy_row_exposes_checkbox_state_and_default_action() {
        let ctx=egui::Context::default();ctx.enable_accesskit();
        let _brand=setup(&ctx);
        let app=crate::privacy::PrivacyApp {executable:"browser.exe".into(),configured:false,selected:false,running:true};
        let mut changed=None;
        let render=|ctx:&egui::Context,changed:&mut Option<bool>| {
            egui::CentralPanel::default().show(ctx,|ui| {
                *changed=privacy_app_row(ui,&app,Theme::new([90,197,235]),crate::i18n::Language::En);
            });
        };
        let output=ctx.run(RawInput::default(),|ctx|render(ctx,&mut changed));
        let update=output.platform_output.accesskit_update.unwrap();
        let (id,node)=update.nodes.iter().find(|(_,node)|node.name()==Some("Hide: browser.exe")).expect("named privacy checkbox");
        assert_eq!(node.role(),Role::CheckBox);
        assert_eq!(node.checked(),Some(Checked::False));
        let mut input=RawInput::default();
        input.events.push(Event::AccessKitActionRequest(ActionRequest {action:Action::Default,target:*id,data:None}));
        let _=ctx.run(input,|ctx|render(ctx,&mut changed));
        assert_eq!(changed,Some(true));
    }

    #[test]
    fn language_switch_exposes_selected_radio() {
        let ctx=egui::Context::default();ctx.enable_accesskit();
        let output=ctx.run(RawInput::default(),|ctx| {
            egui::CentralPanel::default().show(ctx,|ui| {
                language_option(ui,"EN","English language",true,Theme::new([90,197,235]));
            });
        });
        let update=output.platform_output.accesskit_update.unwrap();
        let node=&update.nodes.iter().find(|(_,node)|node.name()==Some("English language")).expect("language radio").1;
        assert_eq!(node.role(),Role::RadioButton);
        assert_eq!(node.checked(),Some(Checked::True));
    }

    #[test]
    fn base_mask_rule_is_checked_and_cannot_be_removed_from_picker() {
        let ctx=egui::Context::default();ctx.enable_accesskit();
        let _brand=setup(&ctx);
        let app=crate::privacy::PrivacyApp {executable:"browser.exe".into(),configured:true,selected:true,running:false};
        let mut changed=None;
        let render=|ctx:&egui::Context,changed:&mut Option<bool>| {
            egui::CentralPanel::default().show(ctx,|ui| {
                *changed=privacy_app_row(ui,&app,Theme::new([90,197,235]),crate::i18n::Language::En);
            });
        };
        let output=ctx.run(RawInput::default(),|ctx|render(ctx,&mut changed));
        let update=output.platform_output.accesskit_update.unwrap();
        let (id,node)=update.nodes.iter().find(|(_,node)|node.name()==Some("Hide: browser.exe (base rule)")).expect("base-rule checkbox");
        assert_eq!(node.role(),Role::CheckBox);
        assert_eq!(node.checked(),Some(Checked::True));
        let mut input=RawInput::default();
        input.events.push(Event::AccessKitActionRequest(ActionRequest {action:Action::Default,target:*id,data:None}));
        let _=ctx.run(input,|ctx|render(ctx,&mut changed));
        assert_eq!(changed,None);
    }

    #[test]
    fn long_app_labels_are_ellipsized_to_available_width() {
        let ctx=egui::Context::default();let _brand=setup(&ctx);
        let _=ctx.run(RawInput::default(),|ctx| {egui::CentralPanel::default().show(ctx,|ui| {
            let font=heavy(12.0);let text="a-very-long-application-executable-name.exe";
            let shortened=fitted_text(ui,text,font.clone(),90.0);
            assert!(shortened.ends_with('…'));
            assert!(ui.painter().layout_no_wrap(shortened,font,TEXT).size().x<=90.0);
            assert_eq!(fitted_text(ui,"App",heavy(12.0),90.0),"App");
        });});
    }
}
