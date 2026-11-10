//! Accessible metadata for custom-drawn egui controls.
//!
//! A clickable Response already participates in keyboard and AccessKit default
//! actions; WidgetInfo additionally supplies the semantic role, name and state.
use eframe::egui::{self, Response, Ui, WidgetInfo, WidgetType};

pub fn button(response:&Response,name:&str) {
    response.widget_info(||WidgetInfo::labeled(WidgetType::Button,name));
}
pub fn radio(response:&Response,name:&str,checked:bool) {
    response.widget_info(||WidgetInfo::selected(WidgetType::RadioButton,checked,name));
}
pub fn checkbox(response:&Response,name:&str,checked:bool) {
    response.widget_info(||WidgetInfo::selected(WidgetType::Checkbox,checked,name));
}
pub fn combobox(response:&Response,name:&str,value:&str) {
    response.widget_info(|| {let mut info=WidgetInfo::labeled(WidgetType::ComboBox,name);info.current_text_value=Some(value.to_owned());info});
}

/// Attach a named semantic group without introducing extra visual layout.
pub fn group(ui:&mut Ui,id:&str,name:&str,add:impl FnOnce(&mut Ui)) {
    let ctx=ui.ctx().clone();
    let id=ui.make_persistent_id(("screenmate_accessibility_group",id));
    let top=ui.cursor().min;let width=ui.available_width();
    let _=ctx.accesskit_node_builder(id,|builder| {builder.set_role(egui::accesskit::Role::Group);builder.set_name(name.to_owned());});
    ctx.with_accessibility_parent(id,||add(ui));
    let bottom=ui.cursor().min.y.max(top.y);
    let _=ctx.accesskit_node_builder(id,|builder|builder.set_bounds(egui::accesskit::Rect {
        x0:top.x.into(),y0:top.y.into(),x1:(top.x+width).into(),y1:bottom.into(),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{accesskit::{Action,ActionRequest,Checked,Role},Event,RawInput,Sense,Vec2};

    #[test]
    fn custom_controls_publish_roles_names_and_values_and_accept_default_action() {
        let ctx=egui::Context::default();ctx.enable_accesskit();
        let mut checked=false;let mut selected=false;let mut paused=false;
        let render=|ctx:&egui::Context,checked:&mut bool,selected:&mut bool,paused:&mut bool| {
            egui::CentralPanel::default().show(ctx,|ui| {
                group(ui,"settings","Настройки",|ui| {
                    let (_,mut response)=ui.allocate_exact_size(Vec2::new(48.0,28.0),Sense::click());
                    if response.clicked(){*checked=!*checked;response.mark_changed();}
                    checkbox(&response,"Свечение экрана",*checked);
                    let (_,mut response)=ui.allocate_exact_size(Vec2::new(80.0,32.0),Sense::click());
                    if response.clicked(){*selected=true;response.mark_changed();}
                    radio(&response,"Монитор 1, основной",*selected);
                    let (_,response)=ui.allocate_exact_size(Vec2::new(80.0,32.0),Sense::click());
                    if response.clicked(){*paused=true;}
                    button(&response,"Пауза");
                });
            });
        };
        let output=ctx.run(RawInput::default(),|ctx|render(ctx,&mut checked,&mut selected,&mut paused));
        let tree=output.platform_output.accesskit_update.expect("AccessKit tree");
        let find=|name:&str|tree.nodes.iter().find(|(_,n)|n.name()==Some(name)).expect("named node");
        let (checkbox_id,checkbox_node)=find("Свечение экрана");
        assert_eq!(checkbox_node.role(),Role::CheckBox);assert_eq!(checkbox_node.checked(),Some(Checked::False));
        assert_eq!(find("Монитор 1, основной").1.role(),Role::RadioButton);
        assert_eq!(find("Монитор 1, основной").1.checked(),Some(Checked::False));
        assert_eq!(find("Пауза").1.role(),Role::Button);
        assert_eq!(find("Настройки").1.role(),Role::Group);
        for name in ["Свечение экрана","Монитор 1, основной","Пауза","Настройки"] {
            let bounds=find(name).1.bounds().expect("interactive and group nodes have geometry");
            assert!(bounds.x1>bounds.x0 && bounds.y1>bounds.y0,"empty bounds for {name}: {bounds:?}");
        }
        let radio_id=find("Монитор 1, основной").0;let button_id=find("Пауза").0;
        let mut input=RawInput::default();input.events.push(Event::AccessKitActionRequest(ActionRequest {action:Action::Default,target:*checkbox_id,data:None}));
        let output=ctx.run(input,|ctx|render(ctx,&mut checked,&mut selected,&mut paused));
        assert!(checked,"AccessKit default action must activate the custom checkbox");
        let update=output.platform_output.accesskit_update.unwrap();
        let after=update.nodes.iter().find(|(_,n)|n.name()==Some("Свечение экрана")).unwrap();
        assert_eq!(after.1.checked(),Some(Checked::True));
        let mut input=RawInput::default();input.events.push(Event::AccessKitActionRequest(ActionRequest {action:Action::Default,target:radio_id,data:None}));
        let output=ctx.run(input,|ctx|render(ctx,&mut checked,&mut selected,&mut paused));
        assert!(selected,"AccessKit default action must activate the custom radio button");
        let update=output.platform_output.accesskit_update.unwrap();
        assert_eq!(update.nodes.iter().find(|(_,n)|n.name()==Some("Монитор 1, основной")).unwrap().1.checked(),Some(Checked::True));
        let mut input=RawInput::default();input.events.push(Event::AccessKitActionRequest(ActionRequest {action:Action::Default,target:button_id,data:None}));
        let _=ctx.run(input,|ctx|render(ctx,&mut checked,&mut selected,&mut paused));
        assert!(paused,"AccessKit default action must activate the custom push button");
    }
}
