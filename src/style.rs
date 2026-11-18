use eframe::egui::{self, Color32, FontFamily, RichText, Stroke};

pub const BG: Color32 = Color32::from_rgb(10, 10, 10);
pub const BG2: Color32 = Color32::from_rgb(17, 17, 17);
pub const BORDER: Color32 = Color32::from_rgb(38, 38, 38);
pub const BORDER_HI: Color32 = Color32::from_rgb(70, 70, 70);
pub const FG: Color32 = Color32::from_rgb(237, 237, 237);
pub const MUTED: Color32 = Color32::from_rgb(140, 140, 140);
pub const DIM: Color32 = Color32::from_rgb(85, 85, 85);

pub fn semi() -> FontFamily {
    FontFamily::Name("semi".into())
}

pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut load = |name: &str, path: &str| -> bool {
        match std::fs::read(path) {
            Ok(bytes) => {
                fonts
                    .font_data
                    .insert(name.to_owned(), egui::FontData::from_owned(bytes));
                true
            }
            Err(_) => false,
        }
    };
    let has_ui = load("segoe", r"C:\Windows\Fonts\segoeui.ttf");
    let has_sb = load("segoe_sb", r"C:\Windows\Fonts\seguisb.ttf");
    let has_sym = load("segoe_sym", r"C:\Windows\Fonts\seguisym.ttf");
    let has_mono = load("consolas", r"C:\Windows\Fonts\consola.ttf");

    let mut prop = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    if has_ui {
        prop.insert(0, "segoe".to_owned());
    }
    if has_sym {
        prop.push("segoe_sym".to_owned());
    }
    let mut semi_fam = Vec::new();
    if has_sb {
        semi_fam.push("segoe_sb".to_owned());
    }
    semi_fam.extend(prop.iter().cloned());

    let mut mono = fonts
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    if has_mono {
        mono.insert(0, "consolas".to_owned());
    }

    fonts.families.insert(FontFamily::Proportional, prop);
    fonts.families.insert(FontFamily::Monospace, mono);
    fonts.families.insert(semi(), semi_fam);
    ctx.set_fonts(fonts);
}

pub fn setup_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = BG;
    v.extreme_bg_color = BG2;
    v.faint_bg_color = BG2;
    v.selection.bg_fill = Color32::from_rgb(48, 48, 48);
    v.selection.stroke = Stroke::new(1.0_f32, FG);
    v.hyperlink_color = FG;
    let r = egui::Rounding::same(8.0);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = r;
        w.expansion = 0.0;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, MUTED);

    v.widgets.inactive.bg_fill = BG2;
    v.widgets.inactive.weak_bg_fill = BG2;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, FG);

    v.widgets.hovered.bg_fill = Color32::from_rgb(24, 24, 24);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(24, 24, 24);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER_HI);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, FG);

    v.widgets.active.bg_fill = Color32::from_rgb(32, 32, 32);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(32, 32, 32);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, MUTED);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, FG);

    style.visuals = v;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.interaction.selectable_labels = false;
    style.spacing.slider_width = 110.0;
    let mut sc = egui::style::ScrollStyle::solid();
    sc.bar_width = 4.0;
    sc.bar_inner_margin = 6.0;
    sc.bar_outer_margin = 0.0;
    style.spacing.scroll = sc;
    ctx.set_style(style);
}

pub fn ghost(ui: &mut egui::Ui, text: &str, on: bool) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(12.5).color(if on { FG } else { MUTED }))
            .frame(false),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn chip(ui: &mut egui::Ui, text: &str, on: bool) -> egui::Response {
    let rt = RichText::new(text)
        .size(12.5)
        .color(if on { Color32::BLACK } else { MUTED });
    ui.add(
        egui::Button::new(rt)
            .fill(if on { FG } else { Color32::TRANSPARENT })
            .stroke(Stroke::new(1.0_f32, if on { FG } else { BORDER }))
            .rounding(999.0)
            .min_size(egui::vec2(0.0, 24.0)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn primary(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(text)
                .size(14.0)
                .family(semi())
                .color(Color32::BLACK),
        )
        .fill(FG)
        .stroke(Stroke::NONE)
        .rounding(8.0)
        .min_size(egui::vec2(104.0, 34.0)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn section(ui: &mut egui::Ui, t: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(t).monospace().size(10.5).color(DIM));
    ui.add_space(2.0);
}

pub fn outline(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(12.5).color(FG))
            .fill(BG2)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .rounding(6.0)
            .min_size(egui::vec2(0.0, 26.0)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn field_label(ui: &mut egui::Ui, t: &str) {
    ui.label(RichText::new(t).size(12.0).color(MUTED));
}
