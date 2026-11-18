use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Corner {
    TL,
    TR,
    BL,
    BR,
}

/// A question. `n` = required answers, 0 = use the global setting.
#[derive(Serialize, Clone, Default, PartialEq, Debug)]
pub struct Question {
    pub text: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub n: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Question {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            n: 0,
        }
    }
}

// accepts both the old format ("text") and the new one ({"text": "...", "n": 1})
impl<'de> Deserialize<'de> for Question {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Plain(String),
            Full {
                text: String,
                #[serde(default)]
                n: usize,
            },
        }
        Ok(match Repr::deserialize(d)? {
            Repr::Plain(text) => Question { text, n: 0 },
            Repr::Full { text, n } => Question { text, n },
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Category {
    pub name: String,
    pub goal: String,
    pub questions: Vec<Question>,
    /// position of the current question inside this category
    pub cursor: usize,
}

impl Category {
    /// non-empty questions in order
    pub fn live(&self) -> Vec<Question> {
        self.questions
            .iter()
            .filter(|q| !q.text.trim().is_empty())
            .map(|q| Question {
                text: q.text.trim().to_string(),
                n: q.n,
            })
            .collect()
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct AiSettings {
    pub endpoint: String,
    pub key: String,
    pub model: String,
    pub prompt: String,
    pub count: usize,
    /// number of generated questions follows `settings.answers`
    pub count_auto: bool,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            endpoint: "https://openrouter.ai/api/v1".into(),
            key: String::new(),
            model: "deepseek/deepseek-chat".into(),
            prompt: crate::ai::DEFAULT_PROMPT.into(),
            count: 5,
            count_auto: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub answers: usize,
    pub remind_min: u32,
    pub daily_goal: u32,
    pub corner: Corner,
    pub shuffle: bool,
    pub autostart: bool,
    /// free position of the orb (top-left), None = use corner
    pub pos: Option<[f32; 2]>,
    pub panel_w: f32,
    /// manual panel height, 0 = auto
    pub panel_h: f32,
    pub sound: bool,
    /// no reminders once the daily goal is reached
    pub quiet_after_goal: bool,
    /// Ctrl+Alt+Space opens / hides the widget from anywhere
    pub hotkey: bool,
    /// collapse back to the orb after each answered question
    pub collapse_after: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            answers: 5,
            remind_min: 45,
            daily_goal: 30,
            corner: Corner::BR,
            shuffle: false,
            autostart: false,
            pos: None,
            panel_w: 400.0,
            panel_h: 0.0,
            sound: true,
            quiet_after_goal: true,
            hotkey: true,
            collapse_after: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub categories: Vec<Category>,
    pub settings: Settings,
    pub ai: AiSettings,
    pub active: usize,
    /// unfinished answers: "category\u{1f}question" -> lines
    pub drafts: BTreeMap<String, Vec<String>>,
    /// old field, only read for migration
    #[serde(skip_serializing)]
    pub q_idx: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            categories: default_categories(),
            settings: Settings::default(),
            ai: AiSettings::default(),
            active: 0,
            drafts: BTreeMap::new(),
            q_idx: 0,
        }
    }
}

pub fn draft_key(cat: &str, q: &str) -> String {
    format!("{}\u{1f}{}", cat.trim(), q.trim())
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub ts: String,
    pub category: String,
    pub question: String,
    pub answers: Vec<String>,
}

impl Entry {
    pub fn date(&self) -> Option<NaiveDate> {
        self.ts
            .get(..10)
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
    }
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let d = base.join("Assume");
    let _ = fs::create_dir_all(&d);
    d
}

pub fn load_config(dir: &Path) -> Config {
    let raw = fs::read_to_string(dir.join("config.json")).ok();
    let fresh = raw.is_none();
    let mut cfg: Config = raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if cfg.categories.is_empty() {
        cfg.categories = default_categories();
    }
    if cfg.active >= cfg.categories.len() {
        cfg.active = 0;
    }
    cfg.settings.answers = cfg.settings.answers.clamp(1, 20);
    if !fresh {
        migrate(&mut cfg);
    }
    // forget drafts of questions that no longer exist
    let keys: BTreeSet<String> = cfg
        .categories
        .iter()
        .flat_map(|c| c.live().into_iter().map(move |q| draft_key(&c.name, &q.text)))
        .collect();
    cfg.drafts.retain(|k, v| keys.contains(k) && v.iter().any(|a| !a.trim().is_empty()));
    cfg
}

fn migrate(cfg: &mut Config) {
    // prompt from v0.1 mentioned schools / authors: switch to the new default
    if cfg.ai.prompt.trim() == crate::ai::OLD_PROMPT_V1.trim() {
        cfg.ai.prompt = crate::ai::DEFAULT_PROMPT.into();
    }
    // q_idx from v0.1 -> per-category cursor
    if cfg.q_idx > 0 {
        let a = cfg.active;
        if let Some(c) = cfg.categories.get_mut(a) {
            if c.cursor == 0 {
                c.cursor = cfg.q_idx;
            }
        }
        cfg.q_idx = 0;
    }
    // "Порог" from v0.1: 1-10 scale question + follow-up
    if let Some(c) = cfg.categories.iter_mut().find(|c| c.name.trim() == "Порог") {
        if let Some(i) = c
            .questions
            .iter()
            .position(|q| q.text.trim() == "Почему моя готовность меняться уже 10 из 10?")
        {
            c.questions[i] = Question {
                text: "Насколько я готов измениться от 1 до 10?".into(),
                n: 1,
            };
            c.questions
                .insert(i + 1, Question::new("Почему я не назвал более скромную цифру?"));
        }
        for q in c.questions.iter_mut() {
            if q.text.trim() == "Что мне нужно, чтобы начать изменения прямо сейчас?" {
                q.text = "Что мне потребуется, чтобы начать изменения прямо сейчас?".into();
            }
        }
    }
}

pub fn save_config(dir: &Path, cfg: &Config) {
    if let Ok(s) = serde_json::to_string_pretty(cfg) {
        let tmp = dir.join("config.json.tmp");
        if fs::write(&tmp, s).is_ok() {
            let _ = fs::rename(&tmp, dir.join("config.json"));
        }
    }
}

pub fn load_journal(dir: &Path) -> Vec<Entry> {
    let Ok(f) = fs::File::open(dir.join("journal.jsonl")) else {
        return Vec::new();
    };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<Entry>(&l).ok())
        .collect()
}

pub fn append_journal(dir: &Path, e: &Entry) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("journal.jsonl"))
    {
        if let Ok(s) = serde_json::to_string(e) {
            let _ = writeln!(f, "{}", s);
        }
    }
}

pub fn export_markdown(dir: &Path, journal: &[Entry]) -> PathBuf {
    let mut out = String::from("# Assume\n");
    let mut last = String::new();
    for e in journal {
        let d = e.ts.get(..10).unwrap_or("").to_string();
        if d != last {
            out.push_str(&format!("\n## {}\n", d));
            last = d;
        }
        out.push_str(&format!("\n**{}** · _{}_\n", e.question, e.category));
        for a in &e.answers {
            out.push_str(&format!("- {}\n", a));
        }
    }
    let p = dir.join("journal.md");
    let _ = fs::write(&p, out);
    p
}

pub fn today_str() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn streak(days: &BTreeSet<NaiveDate>) -> u32 {
    let mut d = Local::now().date_naive();
    if !days.contains(&d) {
        match d.pred_opt() {
            Some(p) => d = p,
            None => return 0,
        }
    }
    let mut n = 0;
    while days.contains(&d) {
        n += 1;
        match d.pred_opt() {
            Some(p) => d = p,
            None => break,
        }
    }
    n
}

fn cat(name: &str, qs: &[(&str, usize)]) -> Category {
    Category {
        name: name.to_string(),
        goal: String::new(),
        questions: qs
            .iter()
            .map(|(t, n)| Question {
                text: t.to_string(),
                n: *n,
            })
            .collect(),
        cursor: 0,
    }
}

pub fn default_categories() -> Vec<Category> {
    vec![
        cat(
            "Порог",
            &[
                ("Почему я могу измениться прямо сейчас?", 0),
                ("Насколько я готов измениться от 1 до 10?", 1),
                ("Почему я не назвал более скромную цифру?", 0),
                ("Что мне потребуется, чтобы начать изменения прямо сейчас?", 0),
                ("Что я могу сделать уже сегодня?", 0),
                ("Что происходит теперь, когда я уже начал меняться?", 0),
                ("Что я вижу, слышу и чувствую, когда изменения уже запущены?", 0),
            ],
        ),
        cat(
            "Я",
            &[
                ("Почему я уже такой уверенный в себе?", 0),
                ("Как я чувствую себя, будучи тем, кем всегда хотел быть?", 0),
                ("Почему мне так легко быть собой?", 0),
                ("Что я делаю иначе теперь, когда уже стал этим человеком?", 0),
                ("Как я замечаю, что становлюсь лучше каждый день?", 0),
                ("Почему у меня всё получается?", 0),
            ],
        ),
        cat(
            "Деньги",
            &[
                ("Почему деньги приходят ко мне так легко?", 0),
                ("Как я становлюсь богаче уже сегодня?", 0),
                ("Какие возможности заработать я уже замечаю вокруг?", 0),
                ("Что я делаю сегодня как человек, у которого уже много денег?", 0),
                ("Почему мой доход растёт каждый месяц?", 0),
            ],
        ),
        cat(
            "Тело",
            &[
                ("Почему мне так легко заботиться о своём теле?", 0),
                ("Как я чувствую себя в сильном здоровом теле?", 0),
                ("Что я сделаю сегодня, чтобы стать стройнее, чем вчера?", 0),
                ("Почему моя энергия растёт с каждым днём?", 0),
            ],
        ),
        cat(
            "Люди",
            &[
                ("Почему люди так тянутся ко мне?", 0),
                ("Как мне удаётся легко находить общий язык с кем угодно?", 0),
                ("Почему рядом со мной людям хорошо?", 0),
                ("Как мои отношения становятся всё глубже?", 0),
            ],
        ),
        cat(
            "Мастерство",
            &[
                ("Почему я так быстро учусь?", 0),
                ("Как я уже становлюсь мастером в своём деле?", 0),
                ("Что я сделаю сегодня как человек, который уже на вершине?", 0),
                ("Почему мне так легко концентрироваться на главном?", 0),
            ],
        ),
    ]
}
