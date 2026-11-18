#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod data;
mod style;
mod win;

use ai::GenResult;
use data::*;
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Id, Key, Layout, Modifiers, Pos2, Rect, RichText,
    Sense, Stroke, Vec2, ViewportCommand,
};
use eframe::egui::viewport::ResizeDirection;
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use style::*;

const ORB: f32 = 56.0;
const MIN_W: f32 = 340.0;
const PANEL_PAD: f32 = 34.0; // inner margin 16*2 + outer 1*2
const MARGIN: f32 = 16.0;
const TASKBAR: f32 = 60.0;
/// max width of the text column in the big modes
const COLUMN: f32 = 760.0;
const ERR: Color32 = Color32::from_rgb(238, 90, 90);

fn main() -> eframe::Result<()> {
    let _lock = match std::net::TcpListener::bind(("127.0.0.1", 47391)) {
        Ok(l) => l,
        Err(_) => return Ok(()),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Assume")
            .with_inner_size([ORB, ORB])
            .with_position([1920.0 - ORB - MARGIN, 1080.0 - ORB - TASKBAR])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_maximize_button(false)
            .with_minimize_button(false)
            .with_icon(std::sync::Arc::new(egui::viewport::IconData {
                rgba: include_bytes!("../assets/icon64.rgba").to_vec(),
                width: 64,
                height: 64,
            }))
            .with_taskbar(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop),
        ..Default::default()
    };
    eframe::run_native(
        "Assume",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

#[derive(PartialEq, Clone, Copy)]
enum View {
    Practice,
    Edit,
    Journal,
}

#[derive(PartialEq, Clone, Copy)]
enum EditTab {
    Questions,
    Practice,
    Ai,
}

#[derive(PartialEq, Clone, Copy)]
enum GenMode {
    Append,
    Replace,
    Fill,
}

/// Normal = widget panel, Max = big window over the work area, Full = real fullscreen
#[derive(PartialEq, Clone, Copy)]
enum SizeMode {
    Normal,
    Max,
    Full,
}

#[derive(Clone, Copy)]
enum Icon {
    Max,
    Unmax,
    Full,
    Unfull,
}

struct App {
    cfg: Config,
    dir: PathBuf,
    journal: Vec<Entry>,
    days: BTreeSet<chrono::NaiveDate>,
    expanded: bool,
    view: View,
    edit_tab: EditTab,
    size_mode: SizeMode,
    // practice
    answers: Vec<String>,
    /// draft key of the question currently on screen
    cur_key: String,
    /// lines added on top of the required amount
    extra: usize,
    focus_line: Option<usize>,
    shown_at: Instant,
    save_at: Option<Instant>,
    // settings
    edit_cat: usize,
    focus_q: Option<usize>,
    focus_name: bool,
    confirm_del: bool,
    confirm_replace: bool,
    confirm_reset: bool,
    bulk: bool,
    bulk_text: String,
    show_key: bool,
    // ai
    gen_rx: Option<Receiver<GenResult>>,
    gen_target: (usize, String, GenMode),
    gen_status: Option<(String, bool)>,
    undo: Option<(usize, Vec<Question>)>,
    // journal
    journal_q: String,
    // misc
    dirty: bool,
    last_activity: Instant,
    was_due: bool,
    hotkey: win::Hotkey,
    // window geometry
    layout_dirty: bool,
    layout_tries: u32,
    mon: Vec2,
    cmd_pos: Pos2,
    cmd_size: Vec2,
    cmd_at: Instant,
    poll_until: Instant,
    settled: bool,
    content_top: f32,
    auto_h: f32,
    m_extra: f32,
    m_row: f32,
}

fn clamp_orb(p: Pos2, mon: Vec2) -> Pos2 {
    egui::pos2(
        p.x.clamp(0.0, (mon.x - ORB).max(0.0)),
        p.y.clamp(0.0, (mon.y - ORB).max(0.0)),
    )
}

fn corner_pos(c: Corner, mon: Vec2) -> Pos2 {
    let x = match c {
        Corner::TL | Corner::BL => MARGIN,
        Corner::TR | Corner::BR => mon.x - ORB - MARGIN,
    };
    let y = match c {
        Corner::TL | Corner::TR => MARGIN,
        Corner::BL | Corner::BR => mon.y - ORB - TASKBAR,
    };
    egui::pos2(x, y)
}

/// panel grows away from the screen edge nearest to the orb
fn panel_from_orb(o: Rect, size: Vec2, mon: Vec2) -> Rect {
    let right = o.center().x > mon.x / 2.0;
    let bottom = o.center().y > mon.y / 2.0;
    let x = if right { o.max.x - size.x } else { o.min.x };
    let y = if bottom { o.max.y - size.y } else { o.min.y };
    let x = x.clamp(0.0, (mon.x - size.x).max(0.0));
    let y = y.clamp(0.0, (mon.y - size.y).max(0.0));
    Rect::from_min_size(egui::pos2(x, y), size)
}

fn orb_from_panel(p: Rect, mon: Vec2) -> Pos2 {
    let right = p.center().x > mon.x / 2.0;
    let bottom = p.center().y > mon.y / 2.0;
    let x = if right { p.max.x - ORB } else { p.min.x };
    let y = if bottom { p.max.y - ORB } else { p.min.y };
    clamp_orb(egui::pos2(x, y), mon)
}

/// row with trailing widgets on the right; main widget fills the rest exactly
fn trailing_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let w = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(w, 32.0),
        Layout::right_to_left(Align::Center),
        add,
    )
    .inner
}

/// an answer counts if it is non-empty and not a repeat of an earlier one
fn validity(answers: &[String]) -> Vec<bool> {
    let mut seen = HashSet::new();
    answers
        .iter()
        .map(|a| {
            let t = a.trim().to_lowercase();
            !t.is_empty() && seen.insert(t)
        })
        .collect()
}

fn hint_for(q: &str) -> &'static str {
    let q = q.trim().to_lowercase();
    if q.starts_with("почему") || q.starts_with("зачем") || q.starts_with("за счёт") {
        "потому что…"
    } else if q.starts_with("насколько") {
        "…"
    } else if q.starts_with("как") {
        "я…"
    } else {
        "…"
    }
}

fn needs_key(ai: &AiSettings) -> bool {
    let e = ai.endpoint.to_lowercase();
    ai.key.trim().is_empty() && !(e.contains("localhost") || e.contains("127.0.0.1"))
}

fn set_autostart(on: bool) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        let mut cmd = std::process::Command::new("reg");
        if on {
            let Ok(exe) = std::env::current_exe() else {
                return;
            };
            let val = format!("\"{}\"", exe.display());
            let v: &str = &val;
            cmd.args(["add", key, "/v", "Assume", "/t", "REG_SZ", "/d", v, "/f"]);
        } else {
            cmd.args(["delete", key, "/v", "Assume", "/f"]);
        }
        let _ = cmd.creation_flags(0x0800_0000).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = on;
    }
}

fn open_path(p: &std::path::Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> bool) -> bool {
    let mut c = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(13.0).color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            c = add(ui);
        });
    });
    c
}

fn check(ui: &mut egui::Ui, v: &mut bool, label: &str) -> bool {
    ui.checkbox(v, RichText::new(label).size(13.0).color(MUTED))
        .changed()
}

fn hint(ui: &mut egui::Ui, t: &str) {
    ui.label(RichText::new(t).size(11.0).color(DIM));
}

fn line_edit<'t>(text: &'t mut String) -> egui::TextEdit<'t> {
    egui::TextEdit::singleline(text)
        .clip_text(true)
        .desired_width(f32::INFINITY)
        .margin(egui::vec2(8.0, 6.0))
}

/// small icon button drawn with lines, so it never depends on font glyphs
fn icon_btn(ui: &mut egui::Ui, icon: Icon, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::click());
    let col = if resp.hovered() { FG } else { MUTED };
    let st = Stroke::new(1.2_f32, col);
    let p = ui.painter();
    let c = rect.center();
    let v = egui::vec2;
    match icon {
        Icon::Max => {
            p.rect_stroke(Rect::from_center_size(c, v(10.0, 9.0)), 1.5, st);
        }
        Icon::Unmax => {
            let front = Rect::from_center_size(c + v(-1.5, 1.5), v(8.0, 7.0));
            p.rect_stroke(front, 1.5, st);
            let back = Rect::from_center_size(c + v(1.5, -1.5), v(8.0, 7.0));
            p.line_segment([back.left_top(), back.right_top()], st);
            p.line_segment([back.right_top(), back.right_bottom()], st);
        }
        Icon::Full | Icon::Unfull => {
            let full = matches!(icon, Icon::Full);
            for (sx, sy) in [(-1.0_f32, -1.0_f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                if full {
                    let k = c + v(sx * 5.5, sy * 5.0);
                    p.line_segment([k, k - v(sx * 3.5, 0.0)], st);
                    p.line_segment([k, k - v(0.0, sy * 3.5)], st);
                } else {
                    let k = c + v(sx * 2.0, sy * 1.8);
                    p.line_segment([k, k + v(sx * 3.5, 0.0)], st);
                    p.line_segment([k, k + v(0.0, sy * 3.5)], st);
                }
            }
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip)
}

/// small clickable monospace label (‹ ›)
fn tiny_btn(ui: &mut egui::Ui, t: &str, tip: &str) -> egui::Response {
    ui.add(
        egui::Label::new(RichText::new(t).monospace().size(12.0).color(MUTED))
            .sense(Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .on_hover_text(tip)
}

const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+Alt+Space", "открыть / спрятать из любого окна"),
    ("Enter", "следующая строка, на полной форме: далее"),
    ("Ctrl+Enter", "далее"),
    ("Shift+Enter", "ещё одна строка ответа"),
    ("↑ ↓", "переход между строками"),
    ("Alt+← →", "предыдущий / следующий вопрос"),
    ("Ctrl+1…9", "категория"),
    ("Ctrl+,", "настройки"),
    ("Ctrl+J", "журнал"),
    ("Ctrl+M", "развернуть в окне"),
    ("F11", "полный экран"),
    ("Esc", "назад / выйти из полного экрана / свернуть"),
];

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        setup_style(&cc.egui_ctx);
        let dir = data_dir();
        let cfg = load_config(&dir);
        let journal = load_journal(&dir);
        let days = journal.iter().filter_map(|e| e.date()).collect();
        let hotkey = win::Hotkey::start(cc.egui_ctx.clone(), cfg.settings.hotkey);
        let active = cfg.active;
        Self {
            cfg,
            dir,
            journal,
            days,
            expanded: false,
            view: View::Practice,
            edit_tab: EditTab::Questions,
            size_mode: SizeMode::Normal,
            answers: Vec::new(),
            cur_key: String::new(),
            extra: 0,
            focus_line: None,
            shown_at: Instant::now(),
            save_at: None,
            edit_cat: active,
            focus_q: None,
            focus_name: false,
            confirm_del: false,
            confirm_replace: false,
            confirm_reset: false,
            bulk: false,
            bulk_text: String::new(),
            show_key: false,
            gen_rx: None,
            gen_target: (0, String::new(), GenMode::Replace),
            gen_status: None,
            undo: None,
            journal_q: String::new(),
            dirty: false,
            last_activity: Instant::now(),
            was_due: false,
            hotkey,
            layout_dirty: true,
            layout_tries: 0,
            mon: egui::vec2(1920.0, 1080.0),
            cmd_pos: egui::pos2(0.0, 0.0),
            cmd_size: egui::vec2(ORB, ORB),
            cmd_at: Instant::now(),
            poll_until: Instant::now(),
            settled: false,
            content_top: 0.0,
            auto_h: 600.0,
            m_extra: 262.0,
            m_row: 37.0,
        }
    }

    fn today_count(&self) -> usize {
        let t = today_str();
        self.journal
            .iter()
            .rev()
            .take_while(|e| e.ts.starts_with(&t))
            .map(|e| e.answers.len())
            .sum()
    }

    fn reminder_due(&self) -> bool {
        let s = &self.cfg.settings;
        let m = s.remind_min;
        if m == 0 || self.last_activity.elapsed() < Duration::from_secs(m as u64 * 60) {
            return false;
        }
        !(s.quiet_after_goal && self.today_count() >= s.daily_goal.max(1) as usize)
    }

    // ---------- current question / drafts ----------

    fn active_name(&self) -> String {
        self.cfg
            .categories
            .get(self.cfg.active)
            .map(|c| c.name.clone())
            .unwrap_or_default()
    }

    /// (index, total, question) of the question on screen
    fn current(&self) -> Option<(usize, usize, Question)> {
        let c = self.cfg.categories.get(self.cfg.active)?;
        let live = c.live();
        if live.is_empty() {
            return None;
        }
        let qi = c.cursor % live.len();
        Some((qi, live.len(), live[qi].clone()))
    }

    fn need(&self, q: &Question) -> usize {
        if q.n > 0 {
            q.n.min(20)
        } else {
            self.cfg.settings.answers.clamp(1, 20)
        }
    }

    /// keep unfinished answers of the question on screen
    fn stash(&mut self) {
        if self.cur_key.is_empty() {
            return;
        }
        if self.answers.iter().any(|a| !a.trim().is_empty()) {
            self.cfg
                .drafts
                .insert(self.cur_key.clone(), self.answers.clone());
        } else {
            self.cfg.drafts.remove(&self.cur_key);
        }
    }

    /// load the right answer lines whenever the question on screen changes
    fn sync_current(&mut self) {
        let cur = self.current();
        let key = cur
            .as_ref()
            .map(|(_, _, q)| draft_key(&self.active_name(), &q.text))
            .unwrap_or_default();
        let switched = key != self.cur_key;
        if switched {
            self.stash();
            self.cur_key = key;
            self.answers = self
                .cfg
                .drafts
                .get(&self.cur_key)
                .cloned()
                .unwrap_or_default();
            self.extra = 0;
            self.shown_at = Instant::now();
        }
        if let Some((_, _, q)) = cur {
            let n = self.need(&q);
            let last = self
                .answers
                .iter()
                .rposition(|a| !a.trim().is_empty())
                .map(|i| i + 1)
                .unwrap_or(0);
            let want = (n + self.extra).max(last);
            if self.answers.len() != want {
                self.answers.resize(want, String::new());
            }
        } else {
            self.answers.clear();
        }
        if switched {
            self.focus_line = Some(self.first_open_line());
        }
    }

    fn first_open_line(&self) -> usize {
        validity(&self.answers)
            .iter()
            .position(|v| !*v)
            .unwrap_or(0)
    }

    /// (valid answers, required, complete)
    fn progress(&self) -> (usize, usize, bool) {
        let Some((_, _, q)) = self.current() else {
            return (0, 0, false);
        };
        let n = self.need(&q);
        let done = validity(&self.answers).iter().filter(|v| **v).count();
        (done, n, done >= n)
    }

    fn step(&mut self, d: i32) {
        let Some((qi, total, _)) = self.current() else {
            return;
        };
        let a = self.cfg.active;
        self.cfg.categories[a].cursor = (qi as i32 + d).rem_euclid(total as i32) as usize;
        self.last_activity = Instant::now();
        self.dirty = true;
    }

    fn switch_cat(&mut self, i: usize) {
        if i < self.cfg.categories.len() && i != self.cfg.active {
            self.cfg.active = i;
            self.dirty = true;
        }
    }

    fn add_line(&mut self) {
        self.extra += 1;
        let i = self.answers.len();
        self.answers.push(String::new());
        self.focus_line = Some(i);
    }

    fn submit(&mut self, ctx: &egui::Context) {
        let Some((qi, total, q)) = self.current() else {
            return;
        };
        let valid = validity(&self.answers);
        let answers: Vec<String> = self
            .answers
            .iter()
            .zip(valid.iter())
            .filter(|(_, ok)| **ok)
            .map(|(a, _)| a.trim().to_string())
            .collect();
        let e = Entry {
            ts: chrono::Local::now().to_rfc3339(),
            category: self.active_name(),
            question: q.text.clone(),
            answers,
        };
        append_journal(&self.dir, &e);
        if let Some(d) = e.date() {
            self.days.insert(d);
        }
        self.journal.push(e);
        self.cfg.drafts.remove(&self.cur_key);
        self.answers.clear();
        self.extra = 0;
        let next = if self.cfg.settings.shuffle && total > 1 {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as usize)
                .unwrap_or(0);
            (qi + 1 + nanos % (total - 1)) % total
        } else {
            (qi + 1) % total
        };
        let a = self.cfg.active;
        self.cfg.categories[a].cursor = next;
        self.sync_current();
        self.focus_line = Some(0);
        self.shown_at = Instant::now();
        self.last_activity = Instant::now();
        self.dirty = true;
        if self.cfg.settings.collapse_after {
            self.collapse(ctx);
        }
    }

    // ---------- views ----------

    fn set_view(&mut self, v: View) {
        if v == self.view {
            return;
        }
        if v == View::Edit {
            self.edit_cat = self.cfg.active.min(self.cfg.categories.len().saturating_sub(1));
            self.bulk = false;
        }
        if self.view == View::Edit {
            // practice continues in the category that was open in the settings
            if self.edit_cat < self.cfg.categories.len() {
                self.switch_cat(self.edit_cat);
            }
            self.bulk = false;
        }
        if v == View::Practice {
            self.sync_current();
            self.focus_line = Some(self.first_open_line());
            self.shown_at = Instant::now();
        }
        self.confirm_del = false;
        self.confirm_replace = false;
        self.view = v;
    }

    fn toggle_view(&mut self, v: View) {
        if self.view == v {
            self.set_view(View::Practice);
        } else {
            self.set_view(v);
        }
    }

    fn set_size_mode(&mut self, ctx: &egui::Context, m: SizeMode) {
        if m == self.size_mode {
            return;
        }
        if self.size_mode == SizeMode::Full {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }
        self.size_mode = m;
        self.layout_dirty = true;
        self.focus_line = Some(self.first_open_line());
    }

    fn expand(&mut self, ctx: &egui::Context) {
        self.expanded = true;
        self.layout_dirty = true;
        self.view = View::Practice;
        self.last_activity = Instant::now();
        self.sync_current();
        self.shown_at = Instant::now();
        self.focus_line = Some(self.first_open_line());
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    fn collapse(&mut self, ctx: &egui::Context) {
        if self.size_mode == SizeMode::Full {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }
        if self.view == View::Edit && self.edit_cat < self.cfg.categories.len() {
            self.switch_cat(self.edit_cat);
        }
        self.view = View::Practice;
        self.size_mode = SizeMode::Normal;
        self.expanded = false;
        self.layout_dirty = true;
        self.confirm_del = false;
        self.confirm_replace = false;
        self.bulk = false;
        self.last_activity = Instant::now();
        self.dirty = true;
    }

    // ---------- geometry ----------

    fn orb_rect(&self) -> Rect {
        let p = match self.cfg.settings.pos {
            Some([x, y]) => clamp_orb(egui::pos2(x, y), self.mon),
            None => corner_pos(self.cfg.settings.corner, self.mon),
        };
        Rect::from_min_size(p, egui::vec2(ORB, ORB))
    }

    fn panel_size(&self) -> Vec2 {
        let s = &self.cfg.settings;
        let max_h = (self.mon.y - TASKBAR - MARGIN).max(300.0);
        let max_w = (self.mon.x - 2.0 * MARGIN).max(MIN_W);
        egui::vec2(
            s.panel_w.clamp(MIN_W, max_w),
            self.auto_h.max(s.panel_h).min(max_h),
        )
    }

    /// big window: work area (screen minus taskbar) with a small gap
    fn max_rect(&self, ctx: &egui::Context) -> Rect {
        let mon = Rect::from_min_size(Pos2::ZERO, self.mon);
        match win::work_area(ctx.pixels_per_point()) {
            Some(r) if mon.expand(2.0).contains_rect(r) => r.shrink(8.0),
            _ => Rect::from_min_max(
                egui::pos2(MARGIN, MARGIN),
                egui::pos2(self.mon.x - MARGIN, self.mon.y - TASKBAR),
            ),
        }
    }

    fn recompute_auto_h(&mut self) {
        let rows = self.cfg.settings.answers.clamp(8, 10) as f32;
        let h = (self.m_extra + rows * self.m_row + PANEL_PAD).round();
        if (h - self.auto_h).abs() > 1.0 {
            self.auto_h = h;
            if self.expanded && self.size_mode == SizeMode::Normal {
                self.layout_dirty = true;
            }
        }
    }

    fn apply_layout(&mut self, ctx: &egui::Context) {
        let mon = ctx
            .input(|i| i.viewport().monitor_size)
            .filter(|m| m.x > 200.0 && m.y > 200.0);
        match mon {
            Some(m) => self.mon = m,
            None => {
                self.layout_tries += 1;
                if self.layout_tries < 30 {
                    return;
                }
            }
        }
        if self.expanded && self.size_mode == SizeMode::Full {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
            self.cmd_pos = Pos2::ZERO;
            self.cmd_size = self.mon;
        } else {
            let rect = if !self.expanded {
                self.orb_rect()
            } else if self.size_mode == SizeMode::Max {
                self.max_rect(ctx)
            } else {
                panel_from_orb(self.orb_rect(), self.panel_size(), self.mon)
            };
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(rect.size()));
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(rect.min));
            self.cmd_pos = rect.min;
            self.cmd_size = rect.size();
        }
        self.cmd_at = Instant::now();
        self.layout_dirty = false;
    }

    /// pick up user drags / resizes done by the OS
    fn observe_window(&mut self, ctx: &egui::Context) {
        if self.expanded && self.size_mode != SizeMode::Normal {
            return;
        }
        if self.cmd_at.elapsed() < Duration::from_millis(400) {
            return;
        }
        let (outer, inner) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
        let Some(outer) = outer else {
            return;
        };
        let size = inner.map(|r| r.size()).unwrap_or(outer.size());
        let moved = (outer.min - self.cmd_pos).length() > 1.5;
        let resized = self.expanded && (size - self.cmd_size).length() > 2.0;
        if !moved && !resized {
            return;
        }
        let rect = Rect::from_min_size(outer.min, size);
        if resized {
            self.cfg.settings.panel_w = size.x.max(MIN_W);
            self.cfg.settings.panel_h = size.y;
            if self.cfg.settings.pos.is_some() {
                let o = orb_from_panel(rect, self.mon);
                self.cfg.settings.pos = Some([o.x, o.y]);
            }
            ctx.send_viewport_cmd(ViewportCommand::Resizable(false));
            self.layout_dirty = true; // snap to limits
        } else {
            let o = if self.expanded {
                orb_from_panel(rect, self.mon)
            } else {
                clamp_orb(outer.min, self.mon)
            };
            self.cfg.settings.pos = Some([o.x, o.y]);
        }
        self.cmd_pos = outer.min;
        self.cmd_size = size;
        self.dirty = true;
    }

    // ---------- AI ----------

    fn gen_count(&self) -> usize {
        if self.cfg.ai.count_auto {
            self.cfg.settings.answers.clamp(1, 30)
        } else {
            self.cfg.ai.count.clamp(1, 30)
        }
    }

    fn start_gen(&mut self, mode: GenMode, ctx: &egui::Context) {
        if self.gen_rx.is_some() {
            return;
        }
        if needs_key(&self.cfg.ai) {
            self.gen_status = Some(("нужен API ключ: вкладка ИИ".into(), true));
            return;
        }
        let count_default = self.gen_count();
        let Some(cat) = self.cfg.categories.get(self.edit_cat) else {
            return;
        };
        let live: Vec<String> = cat.live().into_iter().map(|q| q.text).collect();
        let count = match mode {
            GenMode::Fill => cat
                .questions
                .iter()
                .filter(|q| q.text.trim().is_empty())
                .count(),
            _ => count_default,
        };
        if count == 0 {
            self.gen_status = Some(("пустых полей нет".into(), true));
            return;
        }
        let existing = if mode == GenMode::Replace { Vec::new() } else { live };
        let topic = if cat.name.trim().is_empty() {
            "общая".to_string()
        } else {
            cat.name.clone()
        };
        let prompt = ai::build_prompt(&self.cfg.ai.prompt, &topic, &cat.goal, count, &existing);
        self.gen_target = (self.edit_cat, cat.name.clone(), mode);
        self.gen_rx = Some(ai::spawn(self.cfg.ai.clone(), prompt, count, ctx.clone()));
        self.gen_status = Some((format!("генерирую {}…", count), false));
        self.confirm_replace = false;
        self.undo = None;
    }

    fn poll_gen(&mut self) {
        let polled = match &self.gen_rx {
            Some(rx) => rx.try_recv(),
            None => return,
        };
        let res = match polled {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("поток генерации упал".into()),
        };
        self.gen_rx = None;
        let qs = match res {
            Ok(qs) => qs,
            Err(e) => {
                self.gen_status = Some((e, true));
                return;
            }
        };
        let (ci, name, mode) = self.gen_target.clone();
        let Some(cat) = self.cfg.categories.get_mut(ci).filter(|c| c.name == name) else {
            self.gen_status = Some(("категория изменилась, результат отброшен".into(), true));
            return;
        };
        let have: HashSet<String> = cat
            .questions
            .iter()
            .map(|q| q.text.trim().to_lowercase())
            .collect();
        let fresh: Vec<String> = qs
            .into_iter()
            .filter(|q| mode == GenMode::Replace || !have.contains(&q.trim().to_lowercase()))
            .collect();
        let n = fresh.len();
        match mode {
            GenMode::Replace => {
                self.undo = Some((ci, cat.questions.clone()));
                cat.questions = fresh.into_iter().map(Question::new).collect();
                cat.cursor = 0;
            }
            GenMode::Append => {
                cat.questions.retain(|q| !q.text.trim().is_empty());
                cat.questions.extend(fresh.into_iter().map(Question::new));
            }
            GenMode::Fill => {
                let mut it = fresh.into_iter();
                for q in cat.questions.iter_mut() {
                    if q.text.trim().is_empty() {
                        if let Some(g) = it.next() {
                            q.text = g;
                        }
                    }
                }
            }
        }
        self.gen_status = Some((
            if n == 0 {
                "ничего нового, попробуй ещё раз".to_string()
            } else {
                format!("готово: +{}", n)
            },
            n == 0,
        ));
        self.dirty = true;
    }

    // ---------- orb ----------
    fn ui_orb(&mut self, ctx: &egui::Context) {
        let due = self.reminder_due();
        let today = self.today_count();
        let goal = self.cfg.settings.daily_goal.max(1) as f32;
        let frac = (today as f32 / goal).min(1.0);
        let t = ctx.input(|i| i.time) as f32;
        let mut open = false;
        let mut start_drag = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let resp = ui.interact(rect, Id::new("orb"), Sense::click_and_drag());
                if resp.drag_started() {
                    start_drag = true;
                }
                let c = rect.center();
                let r = 17.0_f32;
                let p = ui.painter();
                if due {
                    let a = (t * 2.2).sin() * 0.5 + 0.5;
                    for k in 0..8 {
                        let kk = k as f32;
                        let alpha = ((1.0 - kk / 8.0).powi(2) * 90.0 * a) as u8;
                        p.circle_stroke(
                            c,
                            r + 1.0 + kk * 1.2,
                            Stroke::new(1.4_f32, Color32::from_white_alpha(alpha)),
                        );
                    }
                }
                p.circle_filled(c, r, BG);
                p.circle_stroke(
                    c,
                    r,
                    Stroke::new(1.0_f32, if resp.hovered() { BORDER_HI } else { BORDER }),
                );
                if frac > 0.0 {
                    let steps = 64;
                    let pts: Vec<egui::Pos2> = (0..=steps)
                        .map(|s| {
                            let ang = -std::f32::consts::FRAC_PI_2
                                + frac * std::f32::consts::TAU * s as f32 / steps as f32;
                            c + egui::vec2(ang.cos(), ang.sin()) * r
                        })
                        .collect();
                    p.add(egui::Shape::line(pts, Stroke::new(1.6_f32, FG)));
                }
                if today > 0 {
                    p.text(
                        c,
                        Align2::CENTER_CENTER,
                        today.to_string(),
                        FontId::monospace(11.0),
                        if due { FG } else { MUTED },
                    );
                } else {
                    p.circle_filled(c, 2.5, if due { FG } else { MUTED });
                }
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    open = true;
                }
            });
        if start_drag {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            self.poll_until = Instant::now() + Duration::from_secs(15);
        } else if open {
            self.expand(ctx);
        }
    }

    // ---------- expanded ----------

    /// keyboard shortcuts, read before any widget so text fields don't eat them
    fn handle_keys(&mut self, ctx: &egui::Context) -> bool {
        let cmd = Modifiers::COMMAND;
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            if self.view != View::Practice {
                self.set_view(View::Practice);
            } else if self.size_mode == SizeMode::Full {
                self.set_size_mode(ctx, SizeMode::Normal);
            } else {
                self.collapse(ctx);
                return false;
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F11)) {
            let m = if self.size_mode == SizeMode::Full {
                SizeMode::Normal
            } else {
                SizeMode::Full
            };
            self.set_size_mode(ctx, m);
        }
        if ctx.input_mut(|i| i.consume_key(cmd, Key::M)) {
            let m = if self.size_mode == SizeMode::Max {
                SizeMode::Normal
            } else {
                SizeMode::Max
            };
            self.set_size_mode(ctx, m);
        }
        if ctx.input_mut(|i| i.consume_key(cmd, Key::Comma)) {
            self.toggle_view(View::Edit);
        }
        if ctx.input_mut(|i| i.consume_key(cmd, Key::J)) {
            self.toggle_view(View::Journal);
        }
        let digits = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ];
        for (i, k) in digits.into_iter().enumerate() {
            if ctx.input_mut(|inp| inp.consume_key(cmd, k)) && i < self.cfg.categories.len() {
                if self.view == View::Edit {
                    self.edit_cat = i;
                    self.bulk = false;
                    self.confirm_del = false;
                } else {
                    self.set_view(View::Practice);
                    self.switch_cat(i);
                }
            }
        }
        if self.view != View::Practice {
            return true;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowLeft)) {
            self.step(-1);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowRight)) {
            self.step(1);
        }
        self.sync_current();
        if ctx.input_mut(|i| i.consume_key(cmd, Key::Enter)) {
            if self.progress().2 {
                self.submit(ctx);
                if !self.expanded {
                    return false;
                }
            } else {
                self.focus_line = Some(self.first_open_line());
            }
        }
        let focused = ctx.memory(|m| m.focused());
        let line = focused.and_then(|f| (0..self.answers.len()).find(|i| Id::new(("ans", *i)) == f));
        if let Some(i) = line {
            if ctx.input_mut(|inp| inp.consume_key(Modifiers::SHIFT, Key::Enter)) {
                self.add_line();
            } else if ctx.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                if i + 1 < self.answers.len() {
                    self.focus_line = Some(i + 1);
                }
            } else if ctx.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::ArrowUp)) && i > 0 {
                self.focus_line = Some(i - 1);
            }
        }
        true
    }

    fn ui_expanded(&mut self, ctx: &egui::Context) {
        if !self.handle_keys(ctx) {
            return;
        }
        let big = self.size_mode != SizeMode::Normal;
        let full = self.size_mode == SizeMode::Full;
        let sw = ctx.screen_rect().width();
        let pad_x = if big { ((sw - COLUMN) / 2.0).max(24.0) } else { 16.0 };
        let pad_y = if big { 28.0 } else { 16.0 };
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(BG)
                    .stroke(if full {
                        Stroke::NONE
                    } else {
                        Stroke::new(1.0_f32, BORDER)
                    })
                    .rounding(if full { 0.0 } else { 12.0 })
                    .outer_margin(if full { 0.0 } else { 1.0 })
                    .inner_margin(egui::Margin {
                        left: pad_x,
                        right: pad_x,
                        top: pad_y,
                        bottom: pad_y,
                    })
                    .show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        ui.set_max_width(ui.available_width());
                        self.content_top = ui.cursor().min.y;
                        self.header(ui);
                        match self.view {
                            View::Practice => self.practice(ui),
                            View::Edit => self.edit(ui),
                            View::Journal => self.journal_view(ui),
                        }
                    });
                if !big {
                    self.resize_grip(ui);
                }
            });
    }

    fn resize_grip(&mut self, ui: &mut egui::Ui) {
        let screen = ui.ctx().screen_rect();
        let c = Rect::from_min_size(self.cmd_pos, self.cmd_size).center();
        let anchor_right = c.x > self.mon.x / 2.0;
        let anchor_bottom = c.y > self.mon.y / 2.0;
        // grip sits in the corner opposite to the anchor
        let left = anchor_right;
        let top = anchor_bottom;
        let (pt, dir, cursor) = match (left, top) {
            (true, true) => (screen.left_top(), ResizeDirection::NorthWest, egui::CursorIcon::ResizeNorthWest),
            (false, true) => (screen.right_top(), ResizeDirection::NorthEast, egui::CursorIcon::ResizeNorthEast),
            (true, false) => (screen.left_bottom(), ResizeDirection::SouthWest, egui::CursorIcon::ResizeSouthWest),
            (false, false) => (screen.right_bottom(), ResizeDirection::SouthEast, egui::CursorIcon::ResizeSouthEast),
        };
        let g = Rect::from_center_size(pt, egui::vec2(32.0, 32.0)).intersect(screen);
        let resp = ui.interact(g, Id::new("grip"), Sense::drag());
        let sx = if left { 1.0 } else { -1.0 };
        let sy = if top { 1.0 } else { -1.0 };
        let o = pt + egui::vec2(sx * 5.0, sy * 5.0);
        let col = if resp.hovered() || resp.dragged() { FG } else { BORDER_HI };
        let st = Stroke::new(1.5_f32, col);
        ui.painter().line_segment([o + egui::vec2(sx * 4.0, 0.0), o + egui::vec2(sx * 11.0, 0.0)], st);
        ui.painter().line_segment([o + egui::vec2(0.0, sy * 4.0), o + egui::vec2(0.0, sy * 11.0)], st);
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(cursor);
        }
        if resp.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::Resizable(true));
            ui.ctx().send_viewport_cmd(ViewportCommand::BeginResize(dir));
            self.poll_until = Instant::now() + Duration::from_secs(30);
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let today = self.today_count();
        let st = streak(&self.days);
        let goal = self.cfg.settings.daily_goal.max(1);
        let frac = (today as f32 / goal as f32).min(1.0);
        let mut go: Option<View> = None;
        let mut close = false;
        let mut mode: Option<SizeMode> = None;
        let normal = self.size_mode == SizeMode::Normal;
        let hdr = Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 22.0));
        let drag = ui.interact(hdr, Id::new("hdr_drag"), Sense::click_and_drag());
        if drag.hovered() && normal {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if drag.drag_started() && normal {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
            self.poll_until = Instant::now() + Duration::from_secs(15);
        }
        if drag.double_clicked() {
            mode = Some(if self.size_mode == SizeMode::Normal {
                SizeMode::Max
            } else {
                SizeMode::Normal
            });
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("assume").family(semi()).size(14.0).color(FG));
            ui.label(
                RichText::new(format!("{}/{}", today, goal))
                    .monospace()
                    .size(11.0)
                    .color(MUTED),
            )
            .on_hover_text("ответов сегодня / цель на день");
            if st > 0 {
                ui.label(
                    RichText::new(format!("{}д", st))
                        .monospace()
                        .size(11.0)
                        .color(MUTED),
                )
                .on_hover_text("дней подряд");
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ghost(ui, "—", false).on_hover_text("свернуть · Esc").clicked() {
                    close = true;
                }
                let full = self.size_mode == SizeMode::Full;
                let (ic, tip) = if full {
                    (Icon::Unfull, "выйти из полного экрана · F11")
                } else {
                    (Icon::Full, "полный экран · F11")
                };
                if icon_btn(ui, ic, tip).clicked() {
                    mode = Some(if full { SizeMode::Normal } else { SizeMode::Full });
                }
                let max = self.size_mode == SizeMode::Max;
                let (ic, tip) = if max {
                    (Icon::Unmax, "обычный размер · Ctrl+M")
                } else {
                    (Icon::Max, "развернуть в окне · Ctrl+M")
                };
                if icon_btn(ui, ic, tip).clicked() {
                    mode = Some(if max { SizeMode::Normal } else { SizeMode::Max });
                }
                if ghost(ui, "⚙", self.view == View::Edit)
                    .on_hover_text("настройки · Ctrl+,")
                    .clicked()
                {
                    go = Some(View::Edit);
                }
                if ghost(ui, "журнал", self.view == View::Journal)
                    .on_hover_text("Ctrl+J")
                    .clicked()
                {
                    go = Some(View::Journal);
                }
            });
        });
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, BORDER);
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, egui::vec2(rect.width() * frac, 1.0)),
            0.0,
            FG,
        );
        if let Some(v) = go {
            self.toggle_view(v);
        }
        if let Some(m) = mode {
            let ctx = ui.ctx().clone();
            self.set_size_mode(&ctx, m);
        }
        if close {
            let ctx = ui.ctx().clone();
            self.collapse(&ctx);
        }
    }

    // ---------- practice ----------
    fn practice(&mut self, ui: &mut egui::Ui) {
        let big = self.size_mode != SizeMode::Normal;
        ui.add_space(10.0);
        if self.cfg.active >= self.cfg.categories.len() {
            self.cfg.active = 0;
        }
        let mut switch_to = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            for (i, c) in self.cfg.categories.iter().enumerate() {
                let name = if c.name.trim().is_empty() { "—" } else { c.name.as_str() };
                let r = chip(ui, name, i == self.cfg.active);
                let r = if i < 9 {
                    r.on_hover_text(format!("Ctrl+{}", i + 1))
                } else {
                    r
                };
                if r.clicked() {
                    switch_to = Some(i);
                }
            }
        });
        if let Some(i) = switch_to {
            self.switch_cat(i);
        }
        self.sync_current();

        let Some((qi, total, q)) = self.current() else {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("в этой категории пока нет вопросов").color(DIM));
                ui.add_space(4.0);
                if ghost(ui, "добавить →", true).clicked() {
                    let a = self.cfg.active;
                    if let Some(c) = self.cfg.categories.get_mut(a) {
                        if c.questions.is_empty() {
                            c.questions.push(Question::default());
                        }
                    }
                    self.edit_tab = EditTab::Questions;
                    self.set_view(View::Edit);
                    self.bulk = false;
                    self.focus_q = Some(0);
                }
            });
            return;
        };
        let n = self.need(&q);
        let hint = hint_for(&q.text);
        let q_size = if big { 30.0 } else { 20.0 };
        let a_size = if big { 17.0 } else { 14.5 };

        ui.add_space(if big { 28.0 } else { 16.0 });
        let fade = (self.shown_at.elapsed().as_secs_f32() / 0.35).min(1.0);
        let mut step = 0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if total > 1 && tiny_btn(ui, "‹", "предыдущий · Alt+←").clicked() {
                step = -1;
            }
            ui.label(
                RichText::new(format!("{:02} / {:02}", qi + 1, total))
                    .monospace()
                    .size(11.0)
                    .color(DIM),
            );
            if total > 1 && tiny_btn(ui, "›", "пропустить · Alt+→").clicked() {
                step = 1;
            }
        });
        if step != 0 {
            self.step(step);
            self.sync_current();
            ui.ctx().request_repaint();
            return;
        }
        let q_resp = ui.add(egui::Label::new(
            RichText::new(&q.text)
                .family(semi())
                .size(q_size)
                .color(FG.gamma_multiply(fade)),
        ));
        let q_h = q_resp.rect.height();
        let two_lines = ui.fonts(|f| f.row_height(&FontId::new(q_size, semi()))) * 2.0;
        ui.add_space(if big { 20.0 } else { 12.0 });

        let focus_req = self.focus_line.take();
        let mut enter_on: Option<usize> = None;
        let mut changed = false;
        let list_h = (ui.available_height() - 52.0).max(80.0);
        let mut row_h_m = 0.0_f32;
        let valid_before = validity(&self.answers);
        let lines = self.answers.len();
        let out = egui::ScrollArea::vertical()
            .max_height(list_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for i in 0..lines {
                    let ok = valid_before[i];
                    let dup = !ok && !self.answers[i].trim().is_empty();
                    let hr = ui.horizontal(|ui| {
                        let num = ui.label(
                            RichText::new(format!("{:02}", i + 1))
                                .monospace()
                                .size(11.0)
                                .color(if ok {
                                    FG
                                } else if dup {
                                    ERR
                                } else if i >= n {
                                    BORDER_HI
                                } else {
                                    DIM
                                }),
                        );
                        if dup {
                            num.on_hover_text("повтор, не считается");
                        }
                        let r = ui.add(
                            line_edit(&mut self.answers[i])
                                .id(Id::new(("ans", i)))
                                .hint_text(if i >= n { "ещё…" } else { hint })
                                .font(FontId::proportional(a_size)),
                        );
                        if focus_req == Some(i) {
                            r.request_focus();
                            r.scroll_to_me(None);
                        }
                        if r.changed() {
                            changed = true;
                        }
                        if r.lost_focus() && ui.input(|inp| inp.key_pressed(Key::Enter)) {
                            enter_on = Some(i);
                        }
                    });
                    if i == 0 {
                        row_h_m = hr.response.rect.height() + ui.spacing().item_spacing.y;
                    }
                }
            });
        let list_used = out.inner_rect.height();
        if changed {
            self.save_at = Some(Instant::now() + Duration::from_millis(1500));
            self.last_activity = Instant::now();
            ui.ctx().request_repaint();
        }

        let valid = validity(&self.answers);
        let done = valid.iter().filter(|v| **v).count();
        let complete = done >= n;
        let mut submit = false;
        let mut more = false;
        if let Some(i) = enter_on {
            if complete {
                submit = true;
            } else {
                let len = valid.len();
                let next = (i + 1..len).chain(0..=i).find(|j| !valid[*j]).unwrap_or(0);
                self.focus_line = Some(next);
            }
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let seg_w = ((140.0 - (n as f32 - 1.0) * 3.0) / n as f32).clamp(3.0, 18.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for k in 0..n {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(seg_w, 2.0), Sense::hover());
                    ui.painter().rect_filled(r, 1.0, if k < done { FG } else { BORDER });
                }
            });
            ui.label(
                RichText::new(format!("{}/{}", done, n))
                    .monospace()
                    .size(11.0)
                    .color(if complete { FG } else { DIM }),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if complete {
                    if primary(ui, "Далее  →")
                        .on_hover_text("Enter · Ctrl+Enter")
                        .clicked()
                    {
                        submit = true;
                    }
                } else {
                    ui.allocate_exact_size(egui::vec2(0.0, 34.0), Sense::hover());
                }
                if ghost(ui, "+ ещё", false)
                    .on_hover_text("ещё одна строка · Shift+Enter")
                    .clicked()
                {
                    more = true;
                }
            });
        });
        // auto height: measure everything except the answer list
        if self.settled && !big {
            let bottom = ui.cursor().min.y;
            let extra = bottom - self.content_top - list_used - q_h + q_h.max(two_lines);
            if extra > 80.0 && extra < 1200.0 {
                self.m_extra = extra;
            }
            if row_h_m > 10.0 && row_h_m < 100.0 {
                self.m_row = row_h_m;
            }
            self.recompute_auto_h();
        }
        if more {
            self.add_line();
        }
        if submit {
            let ctx = ui.ctx().clone();
            self.submit(&ctx);
        }
    }

    // ---------- settings ----------
    fn edit(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let mut tab = self.edit_tab;
        ui.horizontal(|ui| {
            for (t, name) in [
                (EditTab::Questions, "Вопросы"),
                (EditTab::Practice, "Практика"),
                (EditTab::Ai, "ИИ"),
            ] {
                if ghost(ui, name, tab == t).clicked() {
                    tab = t;
                }
            }
        });
        if tab != self.edit_tab {
            self.edit_tab = tab;
            self.confirm_del = false;
            self.confirm_replace = false;
            self.confirm_reset = false;
        }
        let mut changed = false;
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                changed |= match self.edit_tab {
                    EditTab::Questions => self.edit_questions(ui, &ctx),
                    EditTab::Practice => self.edit_practice(ui),
                    EditTab::Ai => self.edit_ai(ui),
                };
                ui.add_space(8.0);
            });
        if changed {
            self.dirty = true;
        }
    }

    fn edit_questions(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> bool {
        let mut changed = false;
        if self.cfg.categories.is_empty() {
            self.cfg.categories.push(Category::default());
        }
        self.edit_cat = self.edit_cat.min(self.cfg.categories.len() - 1);

        section(ui, "КАТЕГОРИЯ");
        let mut pick = None;
        let mut add_cat = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            for (i, c) in self.cfg.categories.iter().enumerate() {
                let name = if c.name.trim().is_empty() { "—" } else { c.name.as_str() };
                if chip(ui, name, i == self.edit_cat).clicked() {
                    pick = Some(i);
                }
            }
            if chip(ui, "+", false).on_hover_text("новая категория").clicked() {
                add_cat = true;
            }
        });
        if let Some(i) = pick {
            if i != self.edit_cat {
                self.edit_cat = i;
                self.confirm_del = false;
                self.confirm_replace = false;
                self.bulk = false;
            }
        }
        if add_cat {
            self.cfg.categories.push(Category {
                questions: vec![Question::default()],
                ..Default::default()
            });
            self.edit_cat = self.cfg.categories.len() - 1;
            self.focus_name = true;
            self.bulk = false;
            self.confirm_del = false;
            self.confirm_replace = false;
            changed = true;
        }
        let ci = self.edit_cat;
        let focus_name = std::mem::take(&mut self.focus_name);
        let mut name_enter = false;
        ui.add_space(6.0);
        field_label(ui, "Название");
        {
            let c = &mut self.cfg.categories[ci];
            let r = ui.add(
                line_edit(&mut c.name)
                    .id(Id::new(("cat_name", ci)))
                    .hint_text("например: Деньги"),
            );
            if focus_name {
                r.request_focus();
            }
            changed |= r.changed();
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                name_enter = true;
            }
            field_label(ui, "Цель / контекст (для ИИ)");
            changed |= ui
                .add(
                    egui::TextEdit::multiline(&mut c.goal)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .hint_text("например: доход 300к в месяц к лету")
                        .margin(egui::vec2(8.0, 6.0)),
                )
                .changed();
        }
        if name_enter {
            self.focus_q = Some(0);
        }

        // questions
        let live_n = self.cfg.categories[ci].live().len();
        let mut toggle_bulk = false;
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("ВОПРОСЫ").monospace().size(10.5).color(DIM));
            ui.label(
                RichText::new(live_n.to_string())
                    .monospace()
                    .size(10.5)
                    .color(DIM),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ghost(ui, if self.bulk { "по одному" } else { "списком" }, self.bulk)
                    .on_hover_text("все вопросы одним текстом, по одному на строку")
                    .clicked()
                {
                    toggle_bulk = true;
                }
            });
        });
        ui.add_space(2.0);
        if toggle_bulk {
            if !self.bulk {
                self.bulk_text = self.cfg.categories[ci]
                    .live()
                    .iter()
                    .map(|q| q.text.clone())
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            self.bulk = !self.bulk;
        }

        if self.bulk {
            let r = ui.add(
                egui::TextEdit::multiline(&mut self.bulk_text)
                    .desired_rows(10)
                    .desired_width(f32::INFINITY)
                    .font(FontId::proportional(13.5))
                    .hint_text("один вопрос на строку")
                    .margin(egui::vec2(8.0, 6.0)),
            );
            if r.changed() {
                let c = &mut self.cfg.categories[ci];
                let old: std::collections::HashMap<String, usize> = c
                    .questions
                    .iter()
                    .map(|q| (q.text.trim().to_string(), q.n))
                    .collect();
                c.questions = self
                    .bulk_text
                    .lines()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty())
                    .map(|t| Question {
                        text: t.to_string(),
                        n: old.get(t).copied().unwrap_or(0),
                    })
                    .collect();
                changed = true;
            }
            hint(ui, "можно вставить готовый список · пустые строки игнорируются");
        } else {
            let focus_q = self.focus_q.take();
            let mut remove = None;
            let mut enter_on = None;
            let c = &mut self.cfg.categories[ci];
            for (i, q) in c.questions.iter_mut().enumerate() {
                trailing_row(ui, |ui| {
                    if ghost(ui, "×", false).on_hover_text("удалить").clicked() {
                        remove = Some(i);
                    }
                    let r = ui
                        .add(
                            egui::DragValue::new(&mut q.n)
                                .range(0..=20)
                                .speed(0.05)
                                .custom_formatter(|v, _| {
                                    if v < 0.5 {
                                        "·".to_string()
                                    } else {
                                        format!("{}", v as usize)
                                    }
                                }),
                        )
                        .on_hover_text("ответов на этот вопрос\n· = как в настройках");
                    changed |= r.changed();
                    let w = ui.available_width();
                    let r = ui.add(
                        line_edit(&mut q.text)
                            .id(Id::new(("q", ci, i)))
                            .hint_text("Почему я уже…?")
                            .desired_width(w),
                    );
                    if focus_q == Some(i) {
                        r.request_focus();
                    }
                    changed |= r.changed();
                    if r.lost_focus() && ui.input(|inp| inp.key_pressed(Key::Enter)) {
                        enter_on = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                c.questions.remove(i);
                changed = true;
            } else if let Some(i) = enter_on {
                // Enter: jump to the next empty field or create one
                let next_empty = c
                    .questions
                    .get(i + 1)
                    .map(|q| q.text.trim().is_empty())
                    .unwrap_or(false);
                if next_empty {
                    self.focus_q = Some(i + 1);
                } else if !c.questions[i].text.trim().is_empty() {
                    c.questions.insert(i + 1, Question::default());
                    self.focus_q = Some(i + 1);
                    changed = true;
                }
            }
            if c.questions.is_empty() && remove.is_none() {
                hint(ui, "пока пусто: добавь вручную или сгенерируй ниже");
            }
        }

        ui.add_space(4.0);
        let can_delete = self.cfg.categories.len() > 1;
        let mut del = false;
        let mut add_q = false;
        ui.horizontal(|ui| {
            if !self.bulk && ghost(ui, "+ вопрос", false).on_hover_text("или Enter в поле вопроса").clicked() {
                add_q = true;
            }
            if can_delete {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.confirm_del {
                        if ghost(ui, "отмена", false).clicked() {
                            self.confirm_del = false;
                        }
                        if ui
                            .add(
                                egui::Button::new(RichText::new("удалить категорию").size(12.5).color(ERR))
                                    .frame(false),
                            )
                            .clicked()
                        {
                            del = true;
                        }
                    } else if ghost(ui, "удалить категорию", false).clicked() {
                        self.confirm_del = true;
                    }
                });
            }
        });
        if add_q {
            let c = &mut self.cfg.categories[ci];
            let at = match c.questions.iter().position(|q| q.text.trim().is_empty()) {
                Some(p) => p,
                None => {
                    c.questions.push(Question::default());
                    c.questions.len() - 1
                }
            };
            self.focus_q = Some(at);
            changed = true;
        }
        if del {
            self.cfg.categories.remove(ci);
            let len = self.cfg.categories.len();
            if self.cfg.active > ci {
                self.cfg.active -= 1;
            }
            self.cfg.active = self.cfg.active.min(len - 1);
            self.edit_cat = ci.min(len - 1);
            self.confirm_del = false;
            self.undo = None;
            changed = true;
            return changed;
        }

        // generation
        section(ui, "ГЕНЕРАЦИЯ ИИ");
        let has = !self.cfg.categories[ci].live().is_empty();
        let empties = self.cfg.categories[ci]
            .questions
            .iter()
            .filter(|q| q.text.trim().is_empty())
            .count();
        let busy = self.gen_rx.is_some();
        let mut mode = None;
        let mut cnt = self.gen_count();
        let auto = self.cfg.ai.count_auto;
        let mut set_auto = false;
        let mut cnt_changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            ui.add_enabled_ui(!busy, |ui| {
                if has {
                    if outline(ui, "✦ добавить")
                        .on_hover_text("новые вопросы в конец списка")
                        .clicked()
                    {
                        mode = Some(GenMode::Append);
                    }
                } else if outline(ui, "✦ сгенерировать").clicked() {
                    mode = Some(GenMode::Replace);
                }
                let r = ui
                    .add(egui::DragValue::new(&mut cnt).range(1..=30).suffix(" шт"))
                    .on_hover_text(if auto {
                        "сколько вопросов: сейчас = ответов на вопрос"
                    } else {
                        "сколько вопросов"
                    });
                cnt_changed = r.changed();
                if !auto
                    && ghost(ui, "авто", false)
                        .on_hover_text("как «ответов на вопрос» в настройках")
                        .clicked()
                {
                    set_auto = true;
                }
                if has && empties > 0 && outline(ui, &format!("заполнить пустые ({})", empties)).clicked() {
                    mode = Some(GenMode::Fill);
                }
            });
        });
        if cnt_changed {
            self.cfg.ai.count = cnt;
            self.cfg.ai.count_auto = false;
            changed = true;
        }
        if set_auto {
            self.cfg.ai.count_auto = true;
            changed = true;
        }
        if has && !busy {
            ui.horizontal(|ui| {
                if self.confirm_replace {
                    ui.label(RichText::new("заменить весь список?").size(12.5).color(MUTED));
                    if ui
                        .add(egui::Button::new(RichText::new("да, заменить").size(12.5).color(ERR)).frame(false))
                        .clicked()
                    {
                        mode = Some(GenMode::Replace);
                    }
                    if ghost(ui, "отмена", false).clicked() {
                        self.confirm_replace = false;
                    }
                } else if ghost(ui, "заменить все", false)
                    .on_hover_text("сгенерировать новый список вместо текущего")
                    .clicked()
                {
                    self.confirm_replace = true;
                }
            });
        }
        if let Some(m) = mode {
            self.start_gen(m, ctx);
        }
        let status = self.gen_status.clone();
        let mut restore = false;
        let mut to_ai = false;
        if busy {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                if let Some((s, _)) = &status {
                    hint(ui, s);
                }
            });
        } else if let Some((s, err)) = &status {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(s).size(12.0).color(if *err { ERR } else { MUTED }));
                if needs_key(&self.cfg.ai) && ghost(ui, "настроить ИИ →", true).clicked() {
                    to_ai = true;
                }
                if matches!(self.undo, Some((u, _)) if u == ci) && ghost(ui, "отменить", false).clicked() {
                    restore = true;
                }
            });
        } else if needs_key(&self.cfg.ai) {
            ui.horizontal_wrapped(|ui| {
                hint(ui, "для генерации нужен API ключ");
                if ghost(ui, "настроить →", true).clicked() {
                    to_ai = true;
                }
            });
        }
        if to_ai {
            self.edit_tab = EditTab::Ai;
        }
        if restore {
            if let Some((u, qs)) = self.undo.take() {
                if let Some(c) = self.cfg.categories.get_mut(u) {
                    c.questions = qs;
                    self.gen_status = Some(("вернул прежний список".into(), false));
                    changed = true;
                }
            }
        }
        changed
    }

    fn edit_practice(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        let mut answers_changed = false;
        let mut hotkey_changed = false;
        let mut test_sound = false;
        section(ui, "ПРАКТИКА");
        {
            let s = &mut self.cfg.settings;
            if row(ui, "Ответов на вопрос", |ui: &mut egui::Ui| {
                ui.add(egui::Slider::new(&mut s.answers, 1..=20)).changed()
            }) {
                s.panel_h = 0.0;
                answers_changed = true;
                changed = true;
            }
            hint(ui, "у отдельного вопроса можно задать своё число: поле рядом с вопросом");
            changed |= row(ui, "Цель в день", |ui: &mut egui::Ui| {
                ui.add(egui::Slider::new(&mut s.daily_goal, 5..=300)).changed()
            });
            changed |= check(ui, &mut s.shuffle, "Случайный порядок вопросов");
            changed |= check(ui, &mut s.collapse_after, "Сворачивать после ответа");
        }

        section(ui, "НАПОМИНАНИЯ");
        {
            let s = &mut self.cfg.settings;
            changed |= row(ui, "Каждые, мин", |ui: &mut egui::Ui| {
                ui.add(egui::Slider::new(&mut s.remind_min, 0..=240)).changed()
            });
            hint(ui, "0 = без напоминаний · кружок начинает мягко пульсировать");
            ui.horizontal(|ui| {
                changed |= check(ui, &mut s.sound, "Звук");
                if ghost(ui, "проверить", false).clicked() {
                    test_sound = true;
                }
            });
            changed |= check(ui, &mut s.quiet_after_goal, "Не напоминать, когда цель дня выполнена");
        }
        if test_sound {
            win::chime();
        }

        section(ui, "ОКНО");
        {
            let s = &mut self.cfg.settings;
            if check(ui, &mut s.hotkey, "Горячая клавиша Ctrl+Alt+Space") {
                hotkey_changed = true;
                changed = true;
            }
            if check(ui, &mut s.autostart, "Запуск вместе с Windows") {
                set_autostart(s.autostart);
                changed = true;
            }
        }
        if hotkey_changed {
            self.hotkey.set_enabled(self.cfg.settings.hotkey);
        }
        if self.cfg.settings.hotkey && !self.hotkey.registered() && cfg!(windows) {
            ui.label(
                RichText::new("Ctrl+Alt+Space занята другой программой")
                    .size(11.0)
                    .color(ERR),
            );
        }
        if answers_changed {
            self.recompute_auto_h();
        }

        let mut new_corner = None;
        let cur = self.cfg.settings.corner;
        let free = self.cfg.settings.pos.is_some();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Угол экрана").size(13.0).color(MUTED));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 30.0), Sense::hover());
                ui.painter()
                    .rect_stroke(rect, 5.0, Stroke::new(1.0_f32, BORDER));
                let cell = egui::vec2(12.0, 8.0);
                for (corner, align) in [
                    (Corner::TL, Align2::LEFT_TOP),
                    (Corner::TR, Align2::RIGHT_TOP),
                    (Corner::BL, Align2::LEFT_BOTTOM),
                    (Corner::BR, Align2::RIGHT_BOTTOM),
                ] {
                    let cr = align.align_size_within_rect(cell, rect.shrink(4.0));
                    let resp = ui.interact(
                        cr.expand(3.0),
                        Id::new(("corner", corner as u8)),
                        Sense::click(),
                    );
                    let col = if cur == corner && !free {
                        FG
                    } else if resp.hovered() {
                        MUTED
                    } else {
                        BORDER
                    };
                    ui.painter().rect_filled(cr, 2.0, col);
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if resp.clicked() {
                        new_corner = Some(corner);
                    }
                }
            });
        });
        hint(
            ui,
            if free {
                "сейчас: своё место (перетащи кружок или шапку окна)"
            } else {
                "или просто перетащи кружок куда угодно"
            },
        );
        if let Some(c) = new_corner {
            self.cfg.settings.corner = c;
            self.cfg.settings.pos = None;
            self.layout_dirty = true;
            changed = true;
        }

        section(ui, "КЛАВИШИ");
        egui::Grid::new("shortcuts")
            .num_columns(2)
            .spacing(egui::vec2(14.0, 5.0))
            .show(ui, |ui| {
                for (k, d) in SHORTCUTS {
                    ui.label(RichText::new(*k).monospace().size(11.0).color(FG));
                    ui.label(RichText::new(*d).size(12.0).color(MUTED));
                    ui.end_row();
                }
            });

        section(ui, "ДАННЫЕ");
        let mut reset = false;
        ui.horizontal_wrapped(|ui| {
            if outline(ui, "папка").clicked() {
                open_path(&self.dir);
            }
            if outline(ui, "экспорт .md").clicked() {
                let p = export_markdown(&self.dir, &self.journal);
                open_path(&p);
            }
            if outline(ui, "выход").clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if self.confirm_reset {
                ui.label(
                    RichText::new("вернуть стандартные вопросы? свои пропадут")
                        .size(12.5)
                        .color(MUTED),
                );
                if ui
                    .add(
                        egui::Button::new(RichText::new("да, сбросить").size(12.5).color(ERR))
                            .frame(false),
                    )
                    .clicked()
                {
                    reset = true;
                }
                if ghost(ui, "отмена", false).clicked() {
                    self.confirm_reset = false;
                }
            } else if ghost(ui, "сбросить вопросы", false).clicked() {
                self.confirm_reset = true;
            }
        });
        if reset {
            self.cfg.categories = default_categories();
            self.cfg.active = 0;
            self.cfg.drafts.clear();
            self.edit_cat = 0;
            self.undo = None;
            self.answers.clear();
            self.cur_key.clear();
            self.confirm_reset = false;
            changed = true;
        }
        changed
    }

    fn edit_ai(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        section(ui, "ПОДКЛЮЧЕНИЕ");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            for (name, url, model) in [
                ("OpenRouter", "https://openrouter.ai/api/v1", "deepseek/deepseek-chat"),
                ("DeepSeek", "https://api.deepseek.com", "deepseek-chat"),
                ("OpenAI", "https://api.openai.com/v1", "gpt-4o-mini"),
                ("Ollama", "http://localhost:11434/v1", "llama3.1"),
            ] {
                let on = self.cfg.ai.endpoint.trim().trim_end_matches('/') == url;
                if chip(ui, name, on).clicked() && !on {
                    self.cfg.ai.endpoint = url.into();
                    self.cfg.ai.model = model.into();
                    changed = true;
                }
            }
        });
        ui.add_space(4.0);
        let a = &mut self.cfg.ai;
        field_label(ui, "Endpoint (OpenAI-совместимый)");
        changed |= ui
            .add(line_edit(&mut a.endpoint).hint_text("https://openrouter.ai/api/v1"))
            .changed();
        field_label(ui, "API ключ");
        let mut show = self.show_key;
        trailing_row(ui, |ui| {
            if ghost(ui, if show { "скрыть" } else { "показать" }, false).clicked() {
                show = !show;
            }
            let w = ui.available_width();
            changed |= ui
                .add(
                    line_edit(&mut a.key)
                        .password(!show)
                        .hint_text("sk-…")
                        .desired_width(w),
                )
                .changed();
        });
        self.show_key = show;
        field_label(ui, "Модель");
        changed |= ui
            .add(line_edit(&mut a.model).hint_text("deepseek/deepseek-chat"))
            .changed();

        section(ui, "КОЛИЧЕСТВО");
        let auto_now = a.count_auto;
        if check(ui, &mut a.count_auto, "Столько же, сколько ответов на вопрос") {
            changed = true;
        }
        if !auto_now && !a.count_auto {
            changed |= row(ui, "Вопросов за раз", |ui: &mut egui::Ui| {
                ui.add(egui::Slider::new(&mut a.count, 1..=30)).changed()
            });
        }

        section(ui, "ПРОМПТ");
        changed |= ui
            .add(
                egui::TextEdit::multiline(&mut a.prompt)
                    .desired_rows(14)
                    .desired_width(f32::INFINITY)
                    .font(FontId::proportional(12.5))
                    .margin(egui::vec2(8.0, 6.0)),
            )
            .changed();
        let is_default = a.prompt.trim() == ai::DEFAULT_PROMPT.trim();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("{topic} {goal} {count} {existing}")
                    .monospace()
                    .size(10.5)
                    .color(DIM),
            );
            if !is_default {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ghost(ui, "вернуть стандартный", false).clicked() {
                        a.prompt = ai::DEFAULT_PROMPT.into();
                        changed = true;
                    }
                });
            }
        });
        hint(ui, "ключ хранится только локально: %APPDATA%\\Assume\\config.json");
        changed
    }

    // ---------- journal ----------
    fn journal_view(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        let total: usize = self.journal.iter().map(|e| e.answers.len()).sum();
        let mut export = false;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} ответов · {} дней", total, self.days.len()))
                    .monospace()
                    .size(11.0)
                    .color(DIM),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !self.journal.is_empty()
                    && ghost(ui, "экспорт .md", false)
                        .on_hover_text("весь журнал одним файлом")
                        .clicked()
                {
                    export = true;
                }
            });
        });
        if export {
            let p = export_markdown(&self.dir, &self.journal);
            open_path(&p);
        }
        ui.add_space(4.0);
        ui.add(
            line_edit(&mut self.journal_q)
                .id(Id::new("journal_q"))
                .hint_text("поиск"),
        );
        let q = self.journal_q.trim().to_lowercase();
        ui.add_space(4.0);
        let big = self.size_mode != SizeMode::Normal;
        let (q_size, a_size) = if big { (16.0, 15.0) } else { (13.5, 13.0) };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                if self.journal.is_empty() {
                    ui.add_space(20.0);
                    ui.label(RichText::new("пока пусто: ответы появятся здесь").color(DIM));
                    return;
                }
                let matches = |e: &Entry| {
                    q.is_empty()
                        || e.question.to_lowercase().contains(&q)
                        || e.category.to_lowercase().contains(&q)
                        || e.answers.iter().any(|a| a.to_lowercase().contains(&q))
                };
                let mut shown = 0;
                let mut last_date = String::new();
                for e in self.journal.iter().rev().filter(|e| matches(e)).take(300) {
                    shown += 1;
                    let d = e.ts.get(..10).unwrap_or("");
                    if d != last_date {
                        ui.add_space(8.0);
                        ui.label(RichText::new(d).monospace().size(11.0).color(DIM));
                        last_date = d.to_string();
                    }
                    ui.add(egui::Label::new(
                        RichText::new(&e.question)
                            .family(semi())
                            .size(q_size)
                            .color(FG),
                    ));
                    ui.label(RichText::new(&e.category).size(11.0).color(DIM));
                    for a in &e.answers {
                        ui.add(egui::Label::new(
                            RichText::new(format!("· {}", a)).size(a_size).color(MUTED),
                        ));
                    }
                    ui.add_space(4.0);
                }
                if shown == 0 {
                    ui.add_space(20.0);
                    ui.label(RichText::new("ничего не найдено").color(DIM));
                }
            });
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_gen();
        if self.hotkey.take() {
            if self.expanded {
                self.collapse(ctx);
            } else {
                self.expand(ctx);
            }
        }
        if let Some(m) = ctx
            .input(|i| i.viewport().monitor_size)
            .filter(|m| m.x > 200.0 && m.y > 200.0)
        {
            self.mon = m;
        }
        if self.layout_dirty {
            self.apply_layout(ctx);
        } else {
            self.observe_window(ctx);
        }
        let sr = ctx.screen_rect();
        self.settled = self.expanded
            && self.size_mode == SizeMode::Normal
            && !self.layout_dirty
            && self.cmd_at.elapsed() > Duration::from_millis(150)
            && (sr.size() - self.cmd_size).length() < 2.0;

        // soft sound once, when a reminder becomes due
        let due = !self.expanded && self.reminder_due();
        if due && !self.was_due && self.cfg.settings.sound {
            win::chime();
        }
        self.was_due = due;

        if self.expanded {
            self.ui_expanded(ctx);
        } else {
            self.ui_orb(ctx);
        }

        // drafts are written a moment after typing stops
        if let Some(t) = self.save_at {
            if Instant::now() >= t {
                self.save_at = None;
                self.dirty = true;
            }
        }
        let closing = ctx.input(|i| i.viewport().close_requested());
        if self.dirty || closing {
            self.stash();
            save_config(&self.dir, &self.cfg);
            self.dirty = false;
            if self.save_at.is_some() && closing {
                self.save_at = None;
            }
        }

        let fading = self.expanded && self.shown_at.elapsed() < Duration::from_millis(400);
        if self.layout_dirty || fading {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if self.gen_rx.is_some() || Instant::now() < self.poll_until {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else if !self.expanded && due {
            ctx.request_repaint_after(Duration::from_millis(33));
        } else if let Some(t) = self.save_at {
            ctx.request_repaint_after(t.saturating_duration_since(Instant::now()) + Duration::from_millis(20));
        } else {
            ctx.request_repaint_after(Duration::from_secs(5));
        }
    }
}
