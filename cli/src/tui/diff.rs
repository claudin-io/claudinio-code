//! Renderiza um unified diff (o `edit_proposal.unified_diff` do core, produzido
//! por `diffy::create_patch`) em `Line`s coloridas — verde/vermelho para
//! adições/remoções, dim para contexto. Inspirado em
//! `packages/coding-agent/src/modes/interactive/components/diff.ts` do `pi`,
//! adaptado ao formato unified padrão.

use super::theme::Theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// Cabeçalho de arquivo (`--- original` / `+++ modified`) — descartado.
    FileHeader,
    /// Cabeçalho de hunk (`@@ -a,b +c,d @@`).
    Hunk,
    Added,
    Removed,
    Context,
    /// `\ No newline at end of file`.
    NoNewline,
}

/// Classifica uma linha do unified diff pelo seu primeiro caractere.
pub fn classify(line: &str) -> DiffLineKind {
    if line.starts_with("--- ") || line.starts_with("+++ ") {
        DiffLineKind::FileHeader
    } else if line.starts_with("@@") {
        DiffLineKind::Hunk
    } else if line.starts_with('\\') {
        DiffLineKind::NoNewline
    } else if line.starts_with('+') {
        DiffLineKind::Added
    } else if line.starts_with('-') {
        DiffLineKind::Removed
    } else {
        DiffLineKind::Context
    }
}

/// Renderiza o diff inteiro. `max_lines == 0` = sem limite; caso contrário,
/// trunca e acrescenta uma nota "… (+N linhas)".
pub fn render_diff(unified: &str, theme: &Theme, max_lines: usize) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let total = unified.lines().count();
    for raw in unified.lines() {
        if max_lines != 0 && out.len() >= max_lines {
            let hidden = total.saturating_sub(out.len());
            if hidden > 0 {
                out.push(Line::from(Span::styled(
                    format!("  … (+{hidden} linhas)"),
                    theme.dim_style(),
                )));
            }
            break;
        }
        match classify(raw) {
            DiffLineKind::FileHeader => continue,
            DiffLineKind::Hunk => out.push(Line::from(Span::styled(
                raw.to_string(),
                Style::default().fg(theme.border_accent).add_modifier(Modifier::DIM),
            ))),
            DiffLineKind::Added => out.push(Line::from(Span::styled(
                raw.to_string(),
                Style::default().fg(theme.diff_added),
            ))),
            DiffLineKind::Removed => out.push(Line::from(Span::styled(
                raw.to_string(),
                Style::default().fg(theme.diff_removed),
            ))),
            DiffLineKind::NoNewline => out.push(Line::from(Span::styled(
                raw.to_string(),
                theme.dim_style(),
            ))),
            DiffLineKind::Context => {
                let content = raw.strip_prefix(' ').unwrap_or(raw);
                out.push(Line::from(Span::styled(
                    format!(" {content}"),
                    Style::default().fg(theme.diff_context),
                )));
            }
        }
    }
    out
}

/// Conta (adições, remoções) de um unified diff — para o resumo do card.
pub fn diff_stats(unified: &str) -> (usize, usize) {
    let mut add = 0;
    let mut del = 0;
    for raw in unified.lines() {
        match classify(raw) {
            DiffLineKind::Added => add += 1,
            DiffLineKind::Removed => del += 1,
            _ => {}
        }
    }
    (add, del)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "--- original\n+++ modified\n@@ -1,3 +1,4 @@\n foo\n-bar\n+baz\n+qux\n context";

    #[test]
    fn classifies_each_prefix() {
        assert_eq!(classify("--- original"), DiffLineKind::FileHeader);
        assert_eq!(classify("+++ modified"), DiffLineKind::FileHeader);
        assert_eq!(classify("@@ -1,3 +1,4 @@"), DiffLineKind::Hunk);
        assert_eq!(classify("+added"), DiffLineKind::Added);
        assert_eq!(classify("-removed"), DiffLineKind::Removed);
        assert_eq!(classify(" context"), DiffLineKind::Context);
        assert_eq!(classify("plain"), DiffLineKind::Context);
        assert_eq!(classify("\\ No newline at end of file"), DiffLineKind::NoNewline);
    }

    #[test]
    fn counts_stats() {
        assert_eq!(diff_stats(SAMPLE), (2, 1));
    }

    #[test]
    fn drops_file_headers_keeps_the_rest() {
        let theme = Theme::dark();
        let lines = render_diff(SAMPLE, &theme, 0);
        // 2 file headers dropped; hunk + " foo" + -bar + +baz + +qux + " context" = 6
        assert_eq!(lines.len(), 6);
    }

    #[test]
    fn caps_and_notes_hidden() {
        let theme = Theme::dark();
        let lines = render_diff(SAMPLE, &theme, 3);
        // 3 rendered + 1 "… (+N linhas)" note
        assert_eq!(lines.len(), 4);
        let last = &lines[3];
        let txt: String = last.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(txt.contains("linhas"), "esperava nota de truncamento, veio: {txt}");
    }
}
