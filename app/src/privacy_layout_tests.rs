//! Geometry and accessibility regression tests for the masking-app picker.
//! These exercise egui off-screen with synthetic apps only; they never query
//! actual windows, capture pixels, or manipulate the user's desktop.
use super::*;
use eframe::egui::{accesskit::{Action,ActionRequest,Checked,Role}, scroll_area, Event, Id, RawInput};

#[derive(Clone, Default)]
pub(super) struct PrivacyLayoutGeometry {
    pub search: Option<Rect>,
    pub list: Option<Rect>,
    pub choose: Option<Rect>,
    pub footer: Option<Rect>,
    /// A row's content rect and its effective scroll-area clip rect.
    pub rows: Vec<(Rect,Rect)>,
    pub scroll_id: Option<Id>,
    pub content_height: f32,
}

fn app_state(language:crate::i18n::Language)->Shared {
    Arc::new(Mutex::new(State {
        config:Config {blocked_titles:vec![],blocked_processes:vec!["base.exe".into()],user_blocked_processes:vec![],ring_size:5,jpeg_quality:70,max_width:1920},
        paused:false,monitor:None,window:None,region:None,frames:VecDeque::new(),last_look:None,inbox:vec![],
        accent:[90,197,235],glow_enabled:true,glow_preference:true,minimize_to_tray:true,language,error:String::new(),
        capture_activity:Arc::new(Mutex::new(crate::activity::CaptureActivity::default())),
    }))
}

fn many_apps()->Vec<crate::privacy::PrivacyApp> {
    let mut apps=vec![
        crate::privacy::PrivacyApp {executable:"base.exe".into(),configured:true,selected:true,running:false},
        crate::privacy::PrivacyApp {executable:"already-hidden.exe".into(),configured:false,selected:true,running:false},
    ];
    for i in 0..24 {
        apps.push(crate::privacy::PrivacyApp {
            executable:format!("app-{i:02}-{}-extremely-long-unbroken-process-name.exe","x".repeat(70)),
            configured:false,selected:false,running:true,
        });
    }
    apps
}

fn frame(ctx:&egui::Context,state:&Shared,picker:&mut PrivacyPicker,language:crate::i18n::Language,size:Vec2)->egui::FullOutput {
    frame_with_events(ctx,state,picker,language,size,vec![])
}
fn frame_with_events(ctx:&egui::Context,state:&Shared,picker:&mut PrivacyPicker,language:crate::i18n::Language,size:Vec2,events:Vec<Event>)->egui::FullOutput {
    let input=RawInput {screen_rect:Some(Rect::from_min_size(egui::Pos2::ZERO,size)),events,..Default::default()};
    ctx.run(input,|ctx| {
        let mut open=true;
        privacy_dialog(ctx,state,picker,&mut open,Theme::new([90,197,235]),language);
        assert!(open);
    })
}

fn geometry(ctx:&egui::Context)->PrivacyLayoutGeometry {
    ctx.data(|d|d.get_temp::<PrivacyLayoutGeometry>(Id::new("privacy-layout-test")))
        .expect("privacy_dialog should publish its test geometry")
}

fn assert_layout(g:&PrivacyLayoutGeometry,size:Vec2,visible_frame:bool) {
    let search=g.search.expect("search rect");
    let list=g.list.expect("list viewport");
    let choose=g.choose.expect("choose rect");
    let footer=g.footer.expect("footer rect");
    let eps=0.5;
    assert!(search.is_positive()&&list.is_positive()&&choose.is_positive()&&footer.is_positive(),"empty rect: search={search:?}, list={list:?}, choose={choose:?}, footer={footer:?}");
    assert!(search.bottom()<=list.top()+eps,"list overlaps search: {search:?} vs {list:?}");
    assert!(list.bottom()<=choose.top()+eps,"list overlaps choose action: {list:?} vs {choose:?}");
    assert!(choose.bottom()<=footer.top()+eps,"choose overlaps footer: {choose:?} vs {footer:?}");
    if visible_frame {
        assert!(search.left()>=-eps && search.right()<=size.x+eps && footer.bottom()<=size.y+eps,
            "picker extends past {size:?}: search={search:?}, footer={footer:?}");
    }
    let mut visible_rows=0;
    for (row,clip) in &g.rows {
        let visible=row.intersect(*clip);
        if visible.is_positive() {
            visible_rows+=1;
            assert!(visible.top()>=list.top()-eps && visible.bottom()<=list.bottom()+eps,
                "row paints outside list viewport: row={row:?} clip={clip:?} list={list:?}");
            assert!(!visible.intersects(search) && !visible.intersects(choose) && !visible.intersects(footer),
                "row paints over reserved controls: visible={visible:?}");
        }
    }
    if visible_frame {assert!(visible_rows>0,"no visible rows recorded");}
    assert!(g.content_height>list.height(),"fixture must exercise scrolling");
}

#[test]
fn picker_never_overlaps_search_or_footer_even_when_scrolled() {
    for language in [crate::i18n::Language::Ru,crate::i18n::Language::En] {
        for size in [Vec2::new(420.0,440.0),Vec2::new(560.0,610.0)] {
            let ctx=egui::Context::default();ctx.enable_accesskit();
            let _brand=setup(&ctx);
            let state=app_state(language);
            let mut picker=PrivacyPicker {apps:many_apps(),..Default::default()};
            // egui intentionally draws a newly created Window invisibly on
            // its first sizing pass, before its anchor position is known.
            let _=frame(&ctx,&state,&mut picker,language,size);
            assert_layout(&geometry(&ctx),size,false);
            for _ in 0..2 {let _=frame(&ctx,&state,&mut picker,language,size);assert_layout(&geometry(&ctx),size,true);}
            let before=geometry(&ctx);assert_layout(&before,size,true);
            let id=before.scroll_id.expect("scroll area id");
            let mut scroll=scroll_area::State::load(&ctx,id).expect("scroll state");
            scroll.offset.y=(before.content_height-before.list.unwrap().height()).max(0.0)*0.55;
            scroll.store(&ctx,id);
            let _=frame(&ctx,&state,&mut picker,language,size);
            let after=geometry(&ctx);assert_layout(&after,size,true);
            let top_before=before.rows.first().unwrap().0.top();
            let top_after=after.rows.first().unwrap().0.top();
            assert!(top_after<top_before-1.0,"fixture did not actually scroll");
        }
    }
}

#[test]
fn long_error_never_pushes_picker_footer_offscreen() {
    for language in [crate::i18n::Language::Ru,crate::i18n::Language::En] {
        let ctx=egui::Context::default();ctx.enable_accesskit();let _brand=setup(&ctx);
        let state=app_state(language);
        let mut picker=PrivacyPicker {apps:many_apps(),error:Some("A deliberately long error message that remains accessible in the tooltip ".repeat(10)),..Default::default()};
        let size=Vec2::new(420.0,440.0);
        let _=frame(&ctx,&state,&mut picker,language,size);
        for _ in 0..2 {let _=frame(&ctx,&state,&mut picker,language,size);assert_layout(&geometry(&ctx),size,true);}
    }
}

#[test]
fn long_executable_text_is_ellipsized_and_clipped_before_the_checkbox() {
    let language=crate::i18n::Language::En;
    let ctx=egui::Context::default();ctx.enable_accesskit();let _brand=setup(&ctx);
    let state=app_state(language);
    let mut picker=PrivacyPicker {apps:vec![crate::privacy::PrivacyApp {
        executable:format!("app-00-{}-long-name.exe","x".repeat(90)),configured:false,selected:true,running:false,
    }],..Default::default()};
    let size=Vec2::new(420.0,440.0);
    for _ in 0..2 {let _=frame(&ctx,&state,&mut picker,language,size);}
    let output=frame(&ctx,&state,&mut picker,language,size);
    let g=geometry(&ctx);
    let row=g.rows.first().expect("long-name row").0;
    let list=g.list.expect("list viewport");
    let mut found=false;
    for shape in &output.shapes {
        let egui::Shape::Text(text)=&shape.shape else {continue};
        if !text.galley.job.text.starts_with("app-00-") {continue;}
        found=true;
        assert!(text.galley.job.text.ends_with('…'),"long visual name must be ellipsized");
        assert!(text.galley.size().x<=shape.clip_rect.width()+0.5,"text extends past its clip");
        assert!(shape.clip_rect.right()<=row.right()-46.0+0.5,"text paints over checkbox");
        assert!(shape.clip_rect.top()>=list.top()-0.5 && shape.clip_rect.bottom()<=list.bottom()+0.5,"text escapes list clip");
    }
    assert!(found,"long-name visual text was not found");
}

#[test]
fn picker_retains_accessibility_for_base_rule_and_long_names() {
    for language in [crate::i18n::Language::Ru,crate::i18n::Language::En] {
        let ctx=egui::Context::default();ctx.enable_accesskit();let _brand=setup(&ctx);
        let state=app_state(language);let mut picker=PrivacyPicker {apps:many_apps(),..Default::default()};
        let size=Vec2::new(560.0,610.0);
        for _ in 0..2 {let _=frame(&ctx,&state,&mut picker,language,size);}
        let output=frame(&ctx,&state,&mut picker,language,size);
        let tree=output.platform_output.accesskit_update.expect("picker accessibility tree");
        let base_name=match language {crate::i18n::Language::Ru=>"Скрывать: base.exe (базовое правило)",crate::i18n::Language::En=>"Hide: base.exe (base rule)"};
        let base=tree.nodes.iter().find(|(_,n)|n.name()==Some(base_name)).expect("base rule checkbox");
        assert_eq!(base.1.role(),Role::CheckBox);
        assert_eq!(base.1.checked(),Some(Checked::True));
        assert!(base.1.is_disabled(),"base rules must remain non-removable");
        assert!(!base.1.supports_action(Action::Default),"disabled base rules must not expose a default action");
        let base_id=base.0;
        let long_prefix=match language {crate::i18n::Language::Ru=>"Скрывать: app-00-",crate::i18n::Language::En=>"Hide: app-00-"};
        assert!(tree.nodes.iter().any(|(_,n)|n.role()==Role::CheckBox && n.name().is_some_and(|name|name.starts_with(long_prefix))),"long executable must keep an accessible name");
        let before_rules=state.lock().unwrap().config.blocked_processes.clone();
        let events=vec![Event::AccessKitActionRequest(ActionRequest {action:Action::Default,target:base_id,data:None})];
        let _=frame_with_events(&ctx,&state,&mut picker,language,size,events);
        assert!(picker.apps[0].configured && picker.apps[0].selected,"UIA must not toggle a base rule");
        assert!(picker.error.is_none(),"disabled control must not invoke its save handler");
        assert_eq!(state.lock().unwrap().config.blocked_processes,before_rules,"base config must remain unchanged");
    }
}
