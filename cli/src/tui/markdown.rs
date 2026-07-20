//! Renderizador de markdown mínimo → `Vec<Line>`, temático. Cobre o que o
//! agente realmente emite: blocos de código cercados, títulos, listas,
//! citações, regras horizontais e inline (`**negrito**`, `*itálico*`,
//! `` `código` ``, `[label](url)`). Inspirado em `pi-tui`'s markdown.ts, porém
//! sem highlight de sintaxe (decisão: binário enxuto).

use super::theme::Theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

fn push_text(spans: &mut Vec<Span<'static>>, buf: &mut String, base: Style) {
    if !buf.is_empty() {
        spans.push(Span::styled(std::mem::take(buf), base));
    }
}

fn find_char(chars: &[char], from: usize, target: char) -> Option<usize> {
    (from..chars.len()).find(|&i| chars[i] == target)
}

fn find_double(chars: &[char], from: usize, m: char) -> Option<usize> {
    let mut i = from;
    while i + 1 < chars.len() {
        if chars[i] == m && chars[i + 1] == m {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Tokeniza inline: negrito/itálico/código/links. Marcadores sem par fecham
/// caem no texto literal (não quebra).
pub fn inline_spans(text: &str, base: Style, theme: &Theme) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    let len = chars.len();

    while i < len {
        let c = chars[i];
        // `código`
        if c == '`' {
            if let Some(close) = find_char(&chars, i + 1, '`') {
                push_text(&mut spans, &mut buf, base);
                let code: String = chars[i + 1..close].iter().collect();
                spans.push(Span::styled(code, Style::default().fg(theme.md_code)));
                i = close + 1;
                continue;
            }
        }
        // **negrito**
        if (c == '*' || c == '_') && i + 1 < len && chars[i + 1] == c {
            if let Some(close) = find_double(&chars, i + 2, c) {
                push_text(&mut spans, &mut buf, base);
                let inner: String = chars[i + 2..close].iter().collect();
                spans.push(Span::styled(inner, base.add_modifier(Modifier::BOLD)));
                i = close + 2;
                continue;
            }
        }
        // *itálico* / _itálico_
        if c == '*' || c == '_' {
            if let Some(close) = find_char(&chars, i + 1, c) {
                if close > i + 1 {
                    push_text(&mut spans, &mut buf, base);
                    let inner: String = chars[i + 1..close].iter().collect();
                    spans.push(Span::styled(inner, base.add_modifier(Modifier::ITALIC)));
                    i = close + 1;
                    continue;
                }
            }
        }
        // [label](url)
        if c == '[' {
            if let Some(rb) = find_char(&chars, i + 1, ']') {
                if rb + 1 < len && chars[rb + 1] == '(' {
                    if let Some(rp) = find_char(&chars, rb + 2, ')') {
                        push_text(&mut spans, &mut buf, base);
                        let label: String = chars[i + 1..rb].iter().collect();
                        spans.push(Span::styled(
                            label,
                            Style::default()
                                .fg(theme.md_link)
                                .add_modifier(Modifier::UNDERLINED),
                        ));
                        i = rp + 1;
                        continue;
                    }
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    push_text(&mut spans, &mut buf, base);
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base));
    }
    spans
}

fn heading(trimmed: &str) -> Option<(usize, &str)> {
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        Some((hashes, trimmed[hashes + 1..].trim_start()))
    } else {
        None
    }
}

fn is_hr(trimmed: &str) -> bool {
    let t = trimmed.trim_end();
    t.len() >= 3
        && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*') || t.chars().all(|c| c == '_'))
}

fn list_marker(trimmed: &str) -> Option<&str> {
    for m in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(m) {
            return Some(rest);
        }
    }
    None
}

/// `src` → linhas estilizadas. Sem wrap (o chamador aplica `Wrap` ao renderizar
/// e usa `Paragraph::line_count` para dimensionar).
pub fn render_markdown(src: &str, theme: &Theme) -> Vec<Line<'static>> {
    let base = Style::default().fg(theme.text);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut in_code = false;

    for raw in src.lines() {
        let trimmed = raw.trim_start();

        // Cerca de bloco de código ```lang
        if let Some(rest) = trimmed.strip_prefix("```") {
            in_code = !in_code;
            let label = if in_code {
                let lang = rest.trim();
                if lang.is_empty() {
                    "┌─".to_string()
                } else {
                    format!("┌─ {lang}")
                }
            } else {
                "└─".to_string()
            };
            out.push(Line::from(Span::styled(label, theme.dim_style())));
            continue;
        }
        if in_code {
            out.push(Line::from(vec![
                Span::styled("│ ".to_string(), theme.dim_style()),
                Span::styled(raw.to_string(), Style::default().fg(theme.md_code_block)),
            ]));
            continue;
        }

        // Título
        if let Some((level, text)) = heading(trimmed) {
            let prefix = "#".repeat(level);
            let mut spans = vec![Span::styled(format!("{prefix} "), theme.dim_style())];
            spans.extend(inline_spans(
                text,
                Style::default().fg(theme.md_heading).add_modifier(Modifier::BOLD),
                theme,
            ));
            out.push(Line::from(spans));
            continue;
        }

        // Regra horizontal
        if is_hr(trimmed) {
            out.push(Line::from(Span::styled("──────────".to_string(), theme.dim_style())));
            continue;
        }

        // Citação
        if let Some(rest) = trimmed.strip_prefix("> ").or_else(|| trimmed.strip_prefix('>')) {
            let mut spans = vec![Span::styled("│ ".to_string(), Style::default().fg(theme.md_quote))];
            spans.extend(inline_spans(rest, Style::default().fg(theme.md_quote), theme));
            out.push(Line::from(spans));
            continue;
        }

        // Lista não ordenada
        if let Some(rest) = list_marker(trimmed) {
            let indent = raw.len() - trimmed.len();
            let mut spans = vec![Span::styled(
                format!("{}• ", " ".repeat(indent)),
                Style::default().fg(theme.md_list_bullet),
            )];
            spans.extend(inline_spans(rest, base, theme));
            out.push(Line::from(spans));
            continue;
        }

        // Parágrafo comum (ou linha em branco)
        if raw.trim().is_empty() {
            out.push(Line::from(String::new()));
        } else {
            out.push(Line::from(inline_spans(raw, base, theme)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn renders_heading_without_breaking() {
        let t = Theme::dark();
        let lines = render_markdown("# Título", &t);
        assert_eq!(lines.len(), 1);
        assert!(plain(&lines[0]).contains("Título"));
    }

    #[test]
    fn code_fence_toggles() {
        let t = Theme::dark();
        let lines = render_markdown("```rust\nlet x = 1;\n```", &t);
        assert_eq!(lines.len(), 3); // abre, conteúdo, fecha
        assert!(plain(&lines[1]).contains("let x = 1;"));
    }

    #[test]
    fn inline_bold_code_link() {
        let t = Theme::dark();
        let spans = inline_spans("a **b** `c` [d](http://x)", Style::default(), &t);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "a b c d");
        // negrito é o span "b"
        let bold = spans.iter().find(|s| s.content.as_ref() == "b").unwrap();
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
        // link "d" tem sublinhado
        let link = spans.iter().find(|s| s.content.as_ref() == "d").unwrap();
        assert!(link.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn unmatched_marker_is_literal() {
        let t = Theme::dark();
        let spans = inline_spans("a * b", Style::default(), &t);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "a * b");
    }

    #[test]
    fn bullet_list() {
        let t = Theme::dark();
        let lines = render_markdown("- um\n- dois", &t);
        assert_eq!(lines.len(), 2);
        assert!(plain(&lines[0]).contains("• "));
        assert!(plain(&lines[0]).contains("um"));
    }
}
