//! Overlays inline (expandem a região viva acima do input, como os seletores do
//! `pi`): paleta de `/comandos` e seletores genéricos (modelo/effort/tema/ajuda).

use super::theme::Theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

#[derive(Debug, Clone, Copy)]
pub struct SlashCmd {
    pub name: &'static str,
    pub desc: &'static str,
}

/// Comandos suportados (subconjunto de `pi_slash.ts` mapeado ao que o core faz).
pub const COMMANDS: &[SlashCmd] = &[
    SlashCmd { name: "model", desc: "escolher modelo do modo atual" },
    SlashCmd { name: "effort", desc: "nível de raciocínio (thinking)" },
    SlashCmd { name: "mode", desc: "alternar brain / builder" },
    SlashCmd { name: "theme", desc: "alternar tema claro / escuro" },
    SlashCmd { name: "new", desc: "iniciar nova sessão" },
    SlashCmd { name: "attach", desc: "anexar arquivo/imagem (ou arraste o caminho)" },
    SlashCmd { name: "copy", desc: "copiar última resposta (OSC52)" },
    SlashCmd { name: "help", desc: "mostrar atalhos" },
    SlashCmd { name: "quit", desc: "sair" },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectKind {
    Model,
    Effort,
    Theme,
    Help,
}

#[derive(Debug, Clone)]
pub struct SelectItem {
    pub label: String,
    pub desc: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct Select {
    pub kind: SelectKind,
    pub title: String,
    pub items: Vec<SelectItem>,
    pub idx: usize,
}

#[derive(Debug, Clone)]
pub struct Slash {
    pub matches: Vec<SlashCmd>,
    pub idx: usize,
}

/// Autocomplete de `@`-mention: lista de arquivos do workspace filtrada.
#[derive(Debug, Clone)]
pub struct Mention {
    pub query: String,
    pub matches: Vec<String>,
    pub idx: usize,
}

impl Mention {
    pub fn selected(&self) -> Option<&String> {
        self.matches.get(self.idx)
    }
}

#[derive(Debug, Clone)]
pub enum Overlay {
    Slash(Slash),
    Mention(Mention),
    Select(Select),
}

impl Slash {
    /// `query` = texto após a `/` (sem a barra). Filtra por prefixo.
    pub fn build(query: &str) -> Slash {
        let q = query.to_lowercase();
        let matches: Vec<SlashCmd> = COMMANDS
            .iter()
            .filter(|c| q.is_empty() || c.name.starts_with(&q))
            .copied()
            .collect();
        Slash { matches, idx: 0 }
    }
    pub fn selected(&self) -> Option<SlashCmd> {
        self.matches.get(self.idx).copied()
    }
}

impl Select {
    pub fn new(kind: SelectKind, title: impl Into<String>, items: Vec<SelectItem>, sel: usize) -> Self {
        let idx = if items.is_empty() { 0 } else { sel.min(items.len() - 1) };
        Select { kind, title: title.into(), items, idx }
    }
    pub fn selected(&self) -> Option<&SelectItem> {
        self.items.get(self.idx)
    }
}

impl Overlay {
    pub fn move_up(&mut self) {
        match self {
            Overlay::Slash(s) => s.idx = s.idx.saturating_sub(1),
            Overlay::Mention(m) => m.idx = m.idx.saturating_sub(1),
            Overlay::Select(s) => s.idx = s.idx.saturating_sub(1),
        }
    }
    pub fn move_down(&mut self) {
        match self {
            Overlay::Slash(s) => {
                if s.idx + 1 < s.matches.len() {
                    s.idx += 1;
                }
            }
            Overlay::Mention(m) => {
                if m.idx + 1 < m.matches.len() {
                    m.idx += 1;
                }
            }
            Overlay::Select(s) => {
                if s.idx + 1 < s.items.len() {
                    s.idx += 1;
                }
            }
        }
    }

    /// Altura desejada (linhas) para dimensionar a região viva.
    pub fn height(&self) -> usize {
        match self {
            Overlay::Slash(s) => (s.matches.len().max(1) + 1).min(9),
            Overlay::Mention(m) => (m.matches.len().max(1) + 1).min(10),
            Overlay::Select(s) => (s.items.len().max(1) + 1).min(12),
        }
    }

    pub fn render(&self, theme: &Theme, width: u16, max_rows: usize) -> Vec<Line<'static>> {
        match self {
            Overlay::Slash(s) => render_slash(s, theme, width, max_rows),
            Overlay::Mention(m) => render_mention(m, theme, width, max_rows),
            Overlay::Select(s) => render_select(s, theme, width, max_rows),
        }
    }
}

fn pad_to(width: u16, s: String) -> String {
    let w = width as usize;
    let len = s.chars().count();
    if len < w {
        format!("{}{}", s, " ".repeat(w - len))
    } else {
        s
    }
}

fn render_slash(s: &Slash, theme: &Theme, width: u16, max_rows: usize) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(Span::styled(
        " comandos ".to_string(),
        Style::default().fg(theme.border_accent).add_modifier(Modifier::BOLD),
    ))];
    if s.matches.is_empty() {
        out.push(Line::from(Span::styled(
            "  sem correspondência".to_string(),
            theme.dim_style(),
        )));
        return out;
    }
    let rows = max_rows.saturating_sub(1).max(1);
    let (start, _) = window(s.idx, s.matches.len(), rows);
    for (i, c) in s.matches.iter().enumerate().skip(start).take(rows) {
        let selected = i == s.idx;
        let text = pad_to(width, format!(" /{}  {}", c.name, c.desc));
        let style = if selected {
            Style::default().fg(theme.text).bg(theme.selected_bg)
        } else {
            theme.muted_style()
        };
        out.push(Line::from(Span::styled(text, style)));
    }
    out
}

fn render_select(s: &Select, theme: &Theme, width: u16, max_rows: usize) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(Span::styled(
        format!(" {} ", s.title),
        Style::default().fg(theme.border_accent).add_modifier(Modifier::BOLD),
    ))];
    let rows = max_rows.saturating_sub(1).max(1);
    let (start, _) = window(s.idx, s.items.len(), rows);
    for (i, it) in s.items.iter().enumerate().skip(start).take(rows) {
        let selected = i == s.idx;
        let marker = if selected { "▸ " } else { "  " };
        let text = if it.desc.is_empty() {
            format!("{marker}{}", it.label)
        } else {
            format!("{marker}{}  —  {}", it.label, it.desc)
        };
        let text = pad_to(width, text);
        let style = if selected {
            Style::default().fg(theme.text).bg(theme.selected_bg).add_modifier(Modifier::BOLD)
        } else {
            theme.muted_style()
        };
        out.push(Line::from(Span::styled(text, style)));
    }
    out
}

fn render_mention(m: &Mention, theme: &Theme, width: u16, max_rows: usize) -> Vec<Line<'static>> {
    let title = if m.query.is_empty() {
        " arquivos ".to_string()
    } else {
        format!(" arquivos: {} ", m.query)
    };
    let mut out = vec![Line::from(Span::styled(
        title,
        Style::default().fg(theme.border_accent).add_modifier(Modifier::BOLD),
    ))];
    if m.matches.is_empty() {
        out.push(Line::from(Span::styled(
            "  nenhum arquivo".to_string(),
            theme.dim_style(),
        )));
        return out;
    }
    let rows = max_rows.saturating_sub(1).max(1);
    let (start, _) = window(m.idx, m.matches.len(), rows);
    for (i, path) in m.matches.iter().enumerate().skip(start).take(rows) {
        let selected = i == m.idx;
        let marker = if selected { "▸ " } else { "  " };
        let text = pad_to(width, format!("{marker}{path}"));
        let style = if selected {
            Style::default().fg(theme.text).bg(theme.selected_bg)
        } else {
            theme.muted_style()
        };
        out.push(Line::from(Span::styled(text, style)));
    }
    out
}

/// Ranqueia caminhos por relevância pra `query` (subsequência case-insensitive,
/// preferindo match no basename e prefixos). Sem dependência de fuzzy.
pub fn rank_files(query: &str, files: &[String], limit: usize) -> Vec<String> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return files.iter().take(limit).cloned().collect();
    }
    let mut scored: Vec<(i64, &String)> = files
        .iter()
        .filter_map(|f| score_file(&q, f).map(|s| (s, f)))
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.len().cmp(&b.1.len())));
    scored.into_iter().take(limit).map(|(_, f)| f.clone()).collect()
}

fn score_file(q: &str, path: &str) -> Option<i64> {
    let pl = path.to_lowercase();
    let base = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    if base.starts_with(q) {
        Some(0)
    } else if base.contains(q) {
        Some(1)
    } else if pl.contains(q) {
        Some(2)
    } else if is_subsequence(q, &pl) {
        Some(3)
    } else {
        None
    }
}

fn is_subsequence(q: &str, hay: &str) -> bool {
    let mut qc = q.chars();
    let mut cur = qc.next();
    for h in hay.chars() {
        match cur {
            Some(c) if h == c => cur = qc.next(),
            Some(_) => {}
            None => break,
        }
    }
    cur.is_none()
}

/// Janela de rolagem: retorna (início, fim) para manter `idx` visível.
fn window(idx: usize, len: usize, rows: usize) -> (usize, usize) {
    if len <= rows {
        return (0, len);
    }
    let half = rows / 2;
    let start = idx.saturating_sub(half).min(len - rows);
    (start, start + rows)
}

/// Itens do seletor de effort (thinking).
pub fn effort_items(current: &str) -> Vec<SelectItem> {
    ["minimal", "low", "medium", "high", "xhigh", "max"]
        .iter()
        .map(|e| SelectItem {
            label: e.to_string(),
            desc: if *e == current { "(atual)".into() } else { String::new() },
            value: e.to_string(),
        })
        .collect()
}

/// Itens do seletor de tema.
pub fn theme_items(current: &str) -> Vec<SelectItem> {
    ["dark", "light"]
        .iter()
        .map(|t| SelectItem {
            label: t.to_string(),
            desc: if *t == current { "(atual)".into() } else { String::new() },
            value: t.to_string(),
        })
        .collect()
}

/// Linhas de ajuda (atalhos) para o overlay Help.
pub fn help_items() -> Vec<SelectItem> {
    let pairs = [
        ("Enter", "enviar (ou enfileirar durante execução)"),
        ("Shift+Enter", "nova linha"),
        ("Tab", "alternar brain / builder"),
        ("/", "paleta de comandos"),
        ("↑/↓", "histórico de mensagens"),
        ("Ctrl+C", "interromper turno / sair"),
        ("Ctrl+W", "apagar palavra"),
        ("y / n", "aprovar / rejeitar ferramenta"),
        ("Esc", "fechar overlay"),
    ];
    pairs
        .iter()
        .map(|(k, d)| SelectItem {
            label: (*k).to_string(),
            desc: (*d).to_string(),
            value: String::new(),
        })
        .collect()
}
