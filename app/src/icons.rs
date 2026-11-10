//! Original, lightweight vector symbols for Screenmate.
//!
//! The paths below are drawn in egui rather than taken from a font or an icon
//! pack: they remain crisp at any scale and require no external resources.
use eframe::egui::{self, Color32, Painter, Rect, Response, Sense, Stroke, Vec2};

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub enum Icon {
    Monitor, Window, Area, Pause, Play, Check, Palette, Settings,
    ChevronDown, ChevronRight, ArrowUp, Send, Message, Shield, More,
    Close, Minimize, Info, Lock, Expand, Sparkle, Back, Eye, EyeOff,
}

/// Paint an icon within `rect` without allocating layout space.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let scale = rect.width().min(rect.height()) / 24.0;
    if scale <= 0.0 { return; }
    let origin = rect.center() - Vec2::splat(12.0 * scale);
    let pt = |x: f32, y: f32| origin + Vec2::new(x * scale, y * scale);
    let bounds = |x1: f32, y1: f32, x2: f32, y2: f32| Rect::from_min_max(pt(x1, y1), pt(x2, y2));
    let stroke = Stroke::new(1.65 * scale, color);
    let line = |a: (f32, f32), b: (f32, f32)| { painter.line_segment([pt(a.0, a.1), pt(b.0, b.1)], stroke); };
    let poly = |points: &[(f32, f32)]| { for pair in points.windows(2) { line(pair[0], pair[1]); } };
    let outline = |r: Rect, radius: f32| { painter.rect_stroke(r, radius * scale, stroke); };
    let dot = |x: f32, y: f32, radius: f32| { painter.circle_filled(pt(x, y), radius * scale, color); };
    match icon {
        Icon::Monitor => {
            outline(bounds(3.0, 4.5, 21.0, 16.5), 2.2);
            line((12.0, 16.5), (12.0, 20.0)); line((8.5, 20.0), (15.5, 20.0));
        }
        Icon::Window => {
            outline(bounds(3.0, 4.5, 21.0, 19.5), 2.2);
            line((3.4, 9.0), (20.6, 9.0));
            dot(6.3, 6.8, 0.65); dot(8.8, 6.8, 0.65);
        }
        Icon::Area => {
            poly(&[(4.0, 9.0), (4.0, 6.0), (4.0, 4.0), (9.0, 4.0)]);
            poly(&[(15.0, 4.0), (20.0, 4.0), (20.0, 9.0)]);
            poly(&[(20.0, 15.0), (20.0, 20.0), (15.0, 20.0)]);
            poly(&[(9.0, 20.0), (4.0, 20.0), (4.0, 15.0)]);
            dot(12.0, 12.0, 1.3);
        }
        Icon::Pause => { line((9.0, 5.5), (9.0, 18.5)); line((15.0, 5.5), (15.0, 18.5)); }
        Icon::Play => {
            painter.add(egui::Shape::convex_polygon(vec![pt(8.0, 5.5), pt(19.0, 12.0), pt(8.0, 18.5)], color, Stroke::NONE));
        }
        Icon::Check => { poly(&[(4.7, 12.2), (9.4, 16.8), (19.2, 7.2)]); }
        Icon::Palette => {
            poly(&[(12.0, 3.0), (8.0, 3.7), (4.9, 6.0), (3.2, 9.7), (3.4, 13.5), (5.3, 17.2), (8.4, 19.5), (11.7, 20.3)]);
            poly(&[(11.7, 20.3), (13.6, 19.7), (14.1, 18.4), (13.4, 17.1), (13.8, 15.6), (15.2, 15.0), (17.6, 15.3), (19.5, 14.3), (20.7, 12.2), (20.3, 9.4), (18.8, 6.5), (16.1, 4.4), (12.0, 3.0)]);
            dot(8.0, 9.0, 1.0); dot(11.0, 6.6, 1.0); dot(15.0, 7.0, 1.0); dot(17.5, 10.0, 1.0);
        }
        Icon::Settings => {
            let center = pt(12.0, 12.0);
            painter.circle_stroke(center, 6.3 * scale, stroke);
            painter.circle_stroke(center, 2.3 * scale, stroke);
            for &(a, b) in &[((12.0, 2.6), (12.0, 5.2)), ((12.0, 18.8), (12.0, 21.4)), ((2.6, 12.0), (5.2, 12.0)), ((18.8, 12.0), (21.4, 12.0)), ((5.3, 5.3), (7.1, 7.1)), ((16.9, 16.9), (18.7, 18.7)), ((18.7, 5.3), (16.9, 7.1)), ((7.1, 16.9), (5.3, 18.7))] { line(a, b); }
        }
        Icon::ChevronDown => { poly(&[(6.5, 9.0), (12.0, 14.5), (17.5, 9.0)]); }
        Icon::ChevronRight => { poly(&[(9.0, 6.5), (14.5, 12.0), (9.0, 17.5)]); }
        Icon::ArrowUp => { line((12.0, 19.0), (12.0, 5.0)); poly(&[(6.5, 10.5), (12.0, 5.0), (17.5, 10.5)]); }
        Icon::Send => {
            poly(&[(3.5, 4.5), (20.5, 12.0), (3.5, 19.5), (6.0, 12.0), (3.5, 4.5)]);
            line((6.0, 12.0), (14.0, 12.0));
        }
        Icon::Message => {
            poly(&[(6.0, 4.5), (18.0, 4.5), (20.0, 6.5), (20.0, 15.0), (18.0, 17.0), (11.0, 17.0), (7.0, 20.0), (7.0, 17.0), (6.0, 17.0), (4.0, 15.0), (4.0, 6.5), (6.0, 4.5)]);
            line((8.0, 9.0), (16.0, 9.0)); line((8.0, 12.0), (14.0, 12.0));
        }
        Icon::Shield => {
            poly(&[(12.0, 3.0), (19.0, 5.7), (18.5, 12.0), (17.0, 15.5), (14.8, 18.1), (12.0, 20.0), (9.2, 18.1), (7.0, 15.5), (5.5, 12.0), (5.0, 5.7), (12.0, 3.0)]);
            poly(&[(8.7, 11.6), (10.9, 13.8), (15.7, 9.0)]);
        }
        Icon::More => { dot(6.5, 12.0, 1.2); dot(12.0, 12.0, 1.2); dot(17.5, 12.0, 1.2); }
        Icon::Close => { line((6.0, 6.0), (18.0, 18.0)); line((18.0, 6.0), (6.0, 18.0)); }
        Icon::Minimize => { line((5.5, 12.0), (18.5, 12.0)); }
        Icon::Info => { painter.circle_stroke(pt(12.0, 12.0), 8.4 * scale, stroke); dot(12.0, 8.0, 1.0); line((12.0, 11.1), (12.0, 16.2)); }
        Icon::Lock => {
            outline(bounds(5.8, 10.5, 18.2, 20.0), 2.0);
            poly(&[(8.5, 10.5), (8.5, 7.5), (9.2, 5.4), (10.7, 4.2), (12.0, 4.0), (13.3, 4.2), (14.8, 5.4), (15.5, 7.5), (15.5, 10.5)]);
            dot(12.0, 14.5, 1.0); line((12.0, 15.3), (12.0, 16.8));
        }
        Icon::Expand => {
            poly(&[(4.0, 9.0), (4.0, 4.0), (9.0, 4.0)]); poly(&[(15.0, 4.0), (20.0, 4.0), (20.0, 9.0)]);
            poly(&[(20.0, 15.0), (20.0, 20.0), (15.0, 20.0)]); poly(&[(9.0, 20.0), (4.0, 20.0), (4.0, 15.0)]);
        }
        Icon::Sparkle => {
            poly(&[(12.0, 3.0), (13.8, 8.8), (19.5, 10.5), (13.8, 12.2), (12.0, 18.0), (10.2, 12.2), (4.5, 10.5), (10.2, 8.8), (12.0, 3.0)]);
            line((18.0, 16.0), (18.0, 21.0)); line((15.5, 18.5), (20.5, 18.5));
        }
        Icon::Back => { line((19.0, 12.0), (5.0, 12.0)); poly(&[(10.5, 6.5), (5.0, 12.0), (10.5, 17.5)]); }
        Icon::Eye | Icon::EyeOff => {
            poly(&[(3.0, 12.0), (5.0, 9.3), (7.4, 7.5), (9.8, 6.6), (12.0, 6.3), (14.2, 6.6), (16.6, 7.5), (19.0, 9.3), (21.0, 12.0), (19.0, 14.7), (16.6, 16.5), (14.2, 17.4), (12.0, 17.7), (9.8, 17.4), (7.4, 16.5), (5.0, 14.7), (3.0, 12.0)]);
            painter.circle_stroke(pt(12.0, 12.0), 2.6 * scale, stroke);
            if matches!(icon, Icon::EyeOff) { line((4.0, 4.0), (20.0, 20.0)); }
        }
    }
}

/// Reserve a square in the UI and paint an icon into it.
#[allow(dead_code)]
pub fn add(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint(ui.painter(), rect, icon, color);
    response
}
