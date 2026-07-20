//! Desenha a região viva no `Viewport::Inline`. A borda envolve **apenas o
//! input** (uma caixa compacta); todo o resto — conteúdo ao vivo em streaming,
//! linha de status/spinner, paleta de comandos e footer — fica FORA da caixa.
//! Blocos finalizados já foram para o scrollback via `insert_before`.

use super::app::App;
use super::footer::{render_footer, spinner_frame, FooterInfo};
use super::overlays::Overlay;
use super::transcript::{self, Status, ToolState};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};
use ratatui::Frame;

const EDITOR_MAX: u16 = 4;

pub fn draw(f: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = f.area();
    let width = area.width;
    if area.height == 0 || width == 0 {
        return;
    }

    let footer_h = 2u16.min(area.height);
    let editor_lines = (app.editor.line_count() as u16).clamp(1, EDITOR_MAX);
    let box_h = (editor_lines + 2).min(area.height.saturating_sub(footer_h));

    // A paleta de slash aparece logo acima da caixa (e esconde a linha de status);
    // os seletores (Select) aparecem na área de conteúdo, no topo.
    let slash_open = matches!(app.overlay, Some(Overlay::Slash(_)));
    let status_h = if slash_open {
        0
    } else {
        1u16.min(area.height.saturating_sub(footer_h + box_h))
    };
    let slash_h = if slash_open {
        overlay_height(app).min(area.height.saturating_sub(footer_h + box_h))
    } else {
        0
    };
    let content_h = area.height.saturating_sub(footer_h + box_h + status_h + slash_h);

    let mut y = area.y;
    let content = Rect::new(area.x, y, width, content_h);
    y += content_h;
    let slash = Rect::new(area.x, y, width, slash_h);
    y += slash_h;
    let status = Rect::new(area.x, y, width, status_h);
    y += status_h;
    let box_area = Rect::new(area.x, y, width, box_h);
    y += box_h;
    let footer_area = Rect::new(area.x, y, width, footer_h);

    // Conteúdo ao vivo (sem borda), ancorado embaixo — ou um seletor.
    if content_h > 0 {
        let lines = match &app.overlay {
            Some(o @ Overlay::Select(_)) => o.render(theme, width, content_h as usize),
            _ => build_active_lines(app),
        };
        let para = Paragraph::new(lines).wrap(Wrap { trim: false });
        let total = para.line_count(width) as u16;
        let scroll = total.saturating_sub(content_h);
        f.render_widget(para.scroll((scroll, 0)), content);
    }

    // Paleta de comandos, logo acima da caixa.
    if slash_h > 0 {
        if let Some(o) = &app.overlay {
            f.render_widget(Paragraph::new(o.render(theme, width, slash_h as usize)), slash);
        }
    }

    // Linha de status/dicas (sem borda).
    if status_h > 0 {
        f.render_widget(Paragraph::new(status_line(app)), status);
    }

    // Caixa do input (a ÚNICA coisa com borda), com prompt "> ".
    if box_h > 0 {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.fg(theme.border_muted))
            .title(Span::styled(
                format!(" {} ", app.mode.as_str()),
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(box_area);
        f.render_widget(block, box_area);
        if inner.height > 0 && inner.width > 2 {
            let gutter = Rect::new(inner.x, inner.y, 2, inner.height);
            let ta_area = Rect::new(inner.x + 2, inner.y, inner.width - 2, inner.height);
            let pc = if app.running { theme.muted } else { theme.accent };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled("> ", theme.fg(pc)))),
                gutter,
            );
            f.render_widget(app.editor.widget(), ta_area);
        }
    }

    // Footer (sem borda), abaixo da caixa.
    if footer_h > 0 {
        let info = FooterInfo {
            cwd: app.cwd_label.clone(),
            mode: app.mode.as_str(),
            model: app.cur_model(),
            effort: app.effort.clone(),
            in_tok: app.in_tok,
            out_tok: app.out_tok,
            cost: app.cost,
            is_sub: app.is_sub,
            context_tokens: app.context_tokens,
            max_context_tokens: app.max_context_tokens,
        };
        f.render_widget(Paragraph::new(render_footer(&info, theme, width)), footer_area);
    }
}

fn overlay_height(app: &App) -> u16 {
    app.overlay.as_ref().map(|o| o.height() as u16).unwrap_or(0)
}

fn status_line(app: &App) -> Line<'static> {
    let theme = &app.theme;
    if let Status::Retrying { attempt, max, secs } = &app.status {
        let sp = spinner_frame(app.spinner_tick);
        return Line::from(vec![
            Span::styled(format!("{sp} "), theme.fg(theme.warning)),
            Span::styled(
                format!("reconectando ({attempt}/{max}) em {secs}s…"),
                theme.muted_style(),
            ),
            Span::styled("  (Ctrl+C cancela)".to_string(), theme.dim_style()),
        ]);
    }
    if app.question.is_some() {
        return Line::from(Span::styled(
            "responda a pergunta acima  ·  Enter envia · dígito escolhe opção".to_string(),
            theme.dim_style(),
        ));
    }
    if app.running {
        let sp = spinner_frame(app.spinner_tick);
        return Line::from(vec![
            Span::styled(format!("{sp} "), theme.fg(theme.accent)),
            Span::styled("trabalhando…".to_string(), theme.muted_style()),
            Span::styled("  (Ctrl+C interrompe · Enter enfileira)".to_string(), theme.dim_style()),
        ]);
    }
    Line::from(Span::styled(
        "Enter enviar · Tab modo · / comandos · Ctrl+C sair".to_string(),
        theme.dim_style(),
    ))
}

/// Concatena o conteúdo em progresso para a área de conteúdo (sem borda).
fn build_active_lines(app: &App) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(t) = &app.thinking {
        lines.extend(transcript::render_thinking(t, theme));
    }
    if let Some(a) = &app.assistant {
        lines.extend(transcript::render_assistant(a, theme));
    }
    for card in &app.tools {
        let max_diff = if card.state == ToolState::AwaitingApproval { 40 } else { 12 };
        lines.extend(transcript::render_tool_card(card, theme, max_diff));
    }
    for sub in &app.subagents {
        lines.push(Line::from(Span::styled(
            format!("⟳ {} …", sub.name),
            theme.fg(theme.subagent),
        )));
    }
    if let Some(q) = &app.question {
        if let Some(item) = q.items.get(q.idx) {
            let counter = if q.items.len() > 1 {
                format!(" ({}/{})", q.idx + 1, q.items.len())
            } else {
                String::new()
            };
            lines.push(Line::from(Span::styled(
                format!("? {}{counter}", item.question),
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
            )));
            for (i, opt) in item.options.iter().enumerate() {
                lines.push(Line::from(Span::styled(
                    format!("  {}) {opt}", i + 1),
                    theme.muted_style(),
                )));
            }
        }
    }
    // Anexos pendentes (pílulas), logo acima do input.
    if !app.attachments.is_empty() {
        let names: Vec<String> = app
            .attachments
            .iter()
            .map(|p| {
                std::path::Path::new(p)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(p)
                    .to_string()
            })
            .collect();
        let pills = names
            .iter()
            .map(|n| format!("📎 {n}"))
            .collect::<Vec<_>>()
            .join("   ");
        lines.push(Line::from(Span::styled(
            format!("anexos: {pills}"),
            theme.fg(theme.accent),
        )));
    }
    lines
}
