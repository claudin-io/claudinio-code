//! Modelo de blocos e renderizadores → `Vec<Line>`. Blocos finalizados são
//! renderizados aqui (com o tema atual) e empurrados para o scrollback nativo
//! via `Terminal::insert_before`; os mesmos renderizadores desenham os blocos
//! "vivos" (em progresso) na região inline. Cada `Line` é `'static` (Strings
//! próprias), então pode ir para o scrollback sem amarrar lifetimes.

use super::diff::{diff_stats, render_diff};
use super::markdown::render_markdown;
use super::theme::Theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// Estado de uma chamada de ferramenta em voo, ou já concluída.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolState {
    AwaitingApproval,
    Running,
    Done,
}

/// Um card de ferramenta: usado tanto vivo (pending/aprovação) quanto ao
/// finalizar (commit para o scrollback).
#[derive(Debug, Clone)]
pub struct ToolCard {
    pub tool_id: String,
    pub name: String,
    pub summary: String,
    pub diff: Option<String>,
    pub output: Option<String>,
    pub is_error: bool,
    pub state: ToolState,
    /// Chave `"{session_id}:{tool_id}"` para responder a aprovação.
    pub approval_key: Option<String>,
}

impl ToolCard {
    pub fn new(tool_id: String, name: String, summary: String) -> Self {
        ToolCard {
            tool_id,
            name,
            summary,
            diff: None,
            output: None,
            is_error: false,
            state: ToolState::Running,
            approval_key: None,
        }
    }
}

/// Subagente em voo (para o resumo final por id).
#[derive(Debug, Clone)]
pub struct SubLive {
    pub id: String,
    pub name: String,
}

/// Status corrente do turno, para a linha de spinner.
#[derive(Debug, Clone)]
pub enum Status {
    Idle,
    Working,
    Retrying { attempt: u32, max: u32, secs: u64 },
}

fn indent(prefix_style: Style, prefix: &str, spans: Vec<Span<'static>>) -> Line<'static> {
    let mut v = vec![Span::styled(prefix.to_string(), prefix_style)];
    v.extend(spans);
    Line::from(v)
}

/// Mensagem do usuário: "❯ …" em accent negrito.
pub fn render_user(text: &str, theme: &Theme) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(String::new())];
    let style = theme.accent_bold();
    for (i, l) in text.lines().enumerate() {
        let prefix = if i == 0 { "❯ " } else { "  " };
        out.push(Line::from(Span::styled(format!("{prefix}{l}"), style)));
    }
    out
}

/// Resposta do assistente, renderizada como markdown.
pub fn render_assistant(src: &str, theme: &Theme) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(String::new())];
    out.extend(render_markdown(src, theme));
    out
}

/// Bloco de "pensando": linhas dim, indentadas.
pub fn render_thinking(text: &str, theme: &Theme) -> Vec<Line<'static>> {
    let style = Style::default().fg(theme.thinking).add_modifier(Modifier::DIM);
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| Line::from(Span::styled(format!("  {}", l.trim_end()), style)))
        .collect()
}

/// Nota curta colorida (handoff, mudança de modo, golden, steering, erro).
pub fn render_notice(text: &str, color: ratatui::style::Color) -> Vec<Line<'static>> {
    vec![Line::from(Span::styled(text.to_string(), Style::default().fg(color)))]
}

/// Card de ferramenta. `max_diff` limita as linhas de diff (0 = sem limite).
pub fn render_tool_card(card: &ToolCard, theme: &Theme, max_diff: usize) -> Vec<Line<'static>> {
    let gutter_color = if card.is_error {
        theme.error
    } else if card.state == ToolState::Done {
        theme.success
    } else {
        theme.accent
    };
    let mut out: Vec<Line<'static>> = Vec::new();

    // Header: ▸ nome  resumo
    let mut header = vec![
        Span::styled("▸ ".to_string(), theme.fg(theme.accent)),
        Span::styled(
            card.name.clone(),
            Style::default().fg(theme.tool_title).add_modifier(Modifier::BOLD),
        ),
    ];
    if !card.summary.is_empty() {
        header.push(Span::styled(format!("  {}", card.summary), theme.muted_style()));
    }
    out.push(Line::from(header));

    // Aprovação pendente
    if card.state == ToolState::AwaitingApproval {
        out.push(Line::from(Span::styled(
            "  aprovar?  [y] sim · [n] não".to_string(),
            Style::default().fg(theme.warning).add_modifier(Modifier::BOLD),
        )));
    }

    // Diff (se houver)
    if let Some(diff) = &card.diff {
        let (add, del) = diff_stats(diff);
        out.push(indent(
            theme.dim_style(),
            "  ",
            vec![
                Span::styled(format!("+{add}"), theme.fg(theme.diff_added)),
                Span::styled(format!(" -{del}"), theme.fg(theme.diff_removed)),
            ],
        ));
        for l in render_diff(diff, theme, max_diff) {
            let mut spans = vec![Span::styled("  │ ".to_string(), theme.fg(gutter_color))];
            spans.extend(l.spans);
            out.push(Line::from(spans));
        }
    } else if let Some(out_text) = &card.output {
        // Saída textual (primeiras linhas), dim.
        for l in out_text.lines().take(8) {
            if l.trim().is_empty() {
                continue;
            }
            out.push(Line::from(Span::styled(
                format!("  {}", truncate_line(l, 200)),
                theme.dim_style(),
            )));
        }
    }

    if card.is_error {
        if let Some(err) = &card.output {
            out.push(Line::from(Span::styled(
                format!("  ✗ {}", truncate_line(err.lines().next().unwrap_or(""), 200)),
                theme.fg(theme.error),
            )));
        }
    }
    out
}

/// Resumo de subagente concluído.
#[allow(clippy::too_many_arguments)]
pub fn render_subagent_done(
    name: &str,
    status: &str,
    rounds: u32,
    in_tok: u32,
    out_tok: u32,
    cost: f64,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let ok = status == "completed" || status == "done" || status == "ok";
    let color = if ok { theme.subagent } else { theme.error };
    vec![Line::from(vec![
        Span::styled("⟳ ".to_string(), theme.fg(color)),
        Span::styled(
            name.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {status} · {rounds} rounds · ↑{in_tok} ↓{out_tok} · ${cost:.3}"),
            theme.dim_style(),
        ),
    ])]
}

/// Pergunta respondida (para o histórico): "? pergunta" + "→ resposta".
pub fn render_question_answered(question: &str, answer: &str, theme: &Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!("? {question}"),
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(format!("  → {answer}"), theme.muted_style())),
    ]
}

pub fn truncate_line(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        format!("{}…", s.chars().take(max).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Resumo de args para o header do card (mesma heurística do `run.rs`).
pub fn tool_summary(args: &serde_json::Value) -> String {
    for key in ["path", "file_path", "command", "query", "pattern", "goal"] {
        if let Some(v) = args.get(key).and_then(|v| v.as_str()) {
            return truncate_line(v.lines().next().unwrap_or(""), 120);
        }
    }
    String::new()
}
