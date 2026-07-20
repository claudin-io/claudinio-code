//! Editor de input: fino wrapper do `tui-textarea` (cursor, seleção, navegação
//! por palavra, kill-ring, paste — de graça) + histórico de comandos (↑/↓).

use super::theme::Theme;
use crossterm::event::KeyEvent;
use ratatui::style::{Modifier, Style};
use tui_textarea::TextArea;

pub struct Editor {
    ta: TextArea<'static>,
    history: Vec<String>,
    hist_idx: Option<usize>,
    draft: String,
    style: Style,
    ph_style: Style,
    cursor_style: Style,
}

const PLACEHOLDER: &str = "mensagem…  (/ para comandos)";

impl Editor {
    pub fn new(theme: &Theme) -> Self {
        let mut ed = Editor {
            ta: TextArea::default(),
            history: Vec::new(),
            hist_idx: None,
            draft: String::new(),
            style: Style::default(),
            ph_style: Style::default(),
            cursor_style: Style::default().add_modifier(Modifier::REVERSED),
        };
        ed.restyle(theme);
        ed.set_text("");
        ed
    }

    pub fn restyle(&mut self, theme: &Theme) {
        self.style = theme.fg(theme.text);
        self.ph_style = theme.dim_style();
        self.cursor_style = Style::default().add_modifier(Modifier::REVERSED);
        self.apply_styles();
    }

    fn apply_styles(&mut self) {
        self.ta.set_placeholder_text(PLACEHOLDER);
        self.ta.set_cursor_line_style(Style::default());
        self.ta.set_style(self.style);
        self.ta.set_placeholder_style(self.ph_style);
        self.ta.set_cursor_style(self.cursor_style);
    }

    pub fn widget(&self) -> &TextArea<'static> {
        &self.ta
    }

    pub fn text(&self) -> String {
        self.ta.lines().join("\n")
    }

    /// Verdadeiro se o conteúdo é uma única linha (para decidir ↑/↓ = histórico).
    pub fn is_single_line(&self) -> bool {
        self.ta.lines().len() <= 1
    }

    pub fn line_count(&self) -> usize {
        self.ta.lines().len().max(1)
    }

    pub fn clear(&mut self) {
        self.set_text("");
        self.hist_idx = None;
    }

    pub fn set_text(&mut self, s: &str) {
        let lines: Vec<String> = if s.is_empty() {
            vec![String::new()]
        } else {
            s.split('\n').map(|l| l.to_string()).collect()
        };
        self.ta = TextArea::new(lines);
        self.apply_styles();
        // Cursor ao fim.
        self.ta.move_cursor(tui_textarea::CursorMove::Bottom);
        self.ta.move_cursor(tui_textarea::CursorMove::End);
    }

    /// Repassa uma tecla de edição ao textarea.
    pub fn input(&mut self, key: KeyEvent) {
        self.ta.input(key);
        self.hist_idx = None;
    }

    pub fn insert_newline(&mut self) {
        self.ta.insert_newline();
    }

    pub fn push_history(&mut self, entry: String) {
        if entry.trim().is_empty() {
            return;
        }
        if self.history.last().map(|s| s.as_str()) != Some(entry.as_str()) {
            self.history.push(entry);
        }
        self.hist_idx = None;
    }

    /// Navega para trás no histórico (↑). Retorna true se mudou.
    pub fn history_prev(&mut self) -> bool {
        if self.history.is_empty() {
            return false;
        }
        let next = match self.hist_idx {
            None => {
                self.draft = self.text();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.hist_idx = Some(next);
        let text = self.history[next].clone();
        self.set_text(&text);
        self.hist_idx = Some(next);
        true
    }

    /// Navega para frente no histórico (↓). Retorna true se mudou.
    pub fn history_next(&mut self) -> bool {
        match self.hist_idx {
            None => false,
            Some(i) if i + 1 < self.history.len() => {
                self.hist_idx = Some(i + 1);
                let text = self.history[i + 1].clone();
                self.set_text(&text);
                self.hist_idx = Some(i + 1);
                true
            }
            Some(_) => {
                // Volta ao rascunho.
                let d = std::mem::take(&mut self.draft);
                self.set_text(&d);
                self.hist_idx = None;
                true
            }
        }
    }
}
