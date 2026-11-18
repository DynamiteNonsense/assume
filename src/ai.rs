use crate::data::AiSettings;
use eframe::egui;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

pub const DEFAULT_PROMPT: &str = "Ты помогаешь с практикой пресуппозиционных вопросов. Такой вопрос исходит из того, что желаемое уже стало правдой, и заставляет искать этому доказательства. Так перенастраивается ретикулярная формация: мозг начинает замечать причины, ресурсы и возможности.

Тема: {topic}
Цель / контекст: {goal}

Сгенерируй {count} новых вопросов на русском, от первого лица.
Правила:
- вопрос уже предполагает, что желаемое состояние достигнуто: «Почему я уже…?», «Как я…?», «Что я чувствую теперь, когда…?», «Что я делаю сегодня как человек, который уже…?»;
- «почему» ищет причины успеха, а не оправдания проблем;
- коротко, конкретно, ярко, без отрицаний и без слов «хочу», «надо», «попробую»;
- каждый вопрос заставляет искать много разных ответов;
- не повторяй уже существующие:
{existing}

Формат ответа: только вопросы, по одному на строку, без нумерации и пояснений.";

/// default prompt of v0.1, replaced automatically if the user never edited it
pub const OLD_PROMPT_V1: &str = "Ты помогаешь с практикой пресуппозиционных вопросов (НЛП + Невилл Годдард: «предположи, что ты уже им являешься»). Такие вопросы перенастраивают ретикулярную формацию: мозг начинает искать доказательства и ресурсы.

Тема: {topic}
Цель / контекст: {goal}

Сгенерируй {count} новых вопросов на русском, от первого лица.
Правила:
- вопрос уже предполагает, что желаемое состояние достигнуто: «Почему я уже…?», «Как я…?», «Что я чувствую теперь, когда…?», «Что я делаю сегодня как человек, который уже…?»;
- «почему» ищет причины успеха, а не оправдания проблем;
- коротко, конкретно, ярко, без отрицаний и без слов «хочу», «надо», «попробую»;
- каждый вопрос заставляет искать много разных ответов;
- не повторяй уже существующие:
{existing}

Формат ответа: только вопросы, по одному на строку, без нумерации и пояснений.";

pub type GenResult = Result<Vec<String>, String>;

pub fn endpoint_url(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.ends_with("/chat/completions") {
        b.to_string()
    } else {
        format!("{}/chat/completions", b)
    }
}

pub fn build_prompt(tpl: &str, topic: &str, goal: &str, count: usize, existing: &[String]) -> String {
    let ex = if existing.is_empty() {
        "—".to_string()
    } else {
        existing
            .iter()
            .map(|s| format!("- {}", s))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let goal = if goal.trim().is_empty() { "—" } else { goal.trim() };
    tpl.replace("{topic}", topic.trim())
        .replace("{goal}", goal)
        .replace("{count}", &count.to_string())
        .replace("{existing}", &ex)
}

pub fn spawn(ai: AiSettings, prompt: String, count: usize, ctx: egui::Context) -> Receiver<GenResult> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let r = request(&ai, &prompt).and_then(|t| {
            let qs = parse(&t, count);
            if qs.is_empty() {
                Err(format!("пустой ответ: {}", short(&t)))
            } else {
                Ok(qs)
            }
        });
        let _ = tx.send(r);
        ctx.request_repaint();
    });
    rx
}

fn short(s: &str) -> String {
    let t: String = s.chars().take(180).collect();
    t.replace('\n', " ")
}

fn request(ai: &AiSettings, prompt: &str) -> Result<String, String> {
    if ai.endpoint.trim().is_empty() || ai.model.trim().is_empty() {
        return Err("укажи endpoint и модель во вкладке ИИ".into());
    }
    let body = serde_json::json!({
        "model": ai.model.trim(),
        "messages": [{ "role": "user", "content": prompt }],
        "temperature": 0.9
    });
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(120))
        .build();
    let mut req = agent
        .post(&endpoint_url(&ai.endpoint))
        .set("Content-Type", "application/json")
        .set("X-Title", "Assume");
    let key = ai.key.trim();
    if !key.is_empty() {
        let auth = format!("Bearer {}", key);
        req = req.set("Authorization", &auth);
    }
    let resp = match req.send_string(&body.to_string()) {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let t = r.into_string().unwrap_or_default();
            return Err(format!("HTTP {}: {}", code, short(&t)));
        }
        Err(e) => return Err(format!("сеть: {}", e)),
    };
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| format!("не JSON: {}", short(&text)))?;
    v["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("нет ответа: {}", short(&text)))
}

fn clean(line: &str) -> String {
    let s = line.trim().trim_start_matches(|c: char| {
        c.is_ascii_digit() || matches!(c, '.' | ')' | '-' | '•' | '*' | '—' | '–' | '#' | ' ' | '\t')
    });
    s.trim()
        .trim_matches(|c: char| matches!(c, '"' | '«' | '»' | '“' | '”' | '*' | '`'))
        .trim()
        .to_string()
}

pub fn parse(text: &str, count: usize) -> Vec<String> {
    let mut t = text.to_string();
    while let (Some(a), Some(b)) = (t.find("<think>"), t.find("</think>")) {
        if b > a {
            t.replace_range(a..b + "</think>".len(), "");
        } else {
            break;
        }
    }
    let lines: Vec<String> = t
        .lines()
        .map(clean)
        .filter(|l| l.chars().count() > 3)
        .collect();
    let with_q: Vec<String> = lines.iter().filter(|l| l.contains('?')).cloned().collect();
    let pick = if with_q.is_empty() { lines } else { with_q };
    pick.into_iter().take(count).collect()
}
