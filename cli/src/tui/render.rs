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
const CONTENT_CAP: u16 = 12;
/// Máximo de tarefas listadas no painel fixo antes de colapsar em "+N more".
const TASKS_CAP: u16 = 6;

/// Altura do "cromo" (fora do conteúdo): painel de tarefas + status + caixa do
/// input + footer. É a altura do viewport quando ocioso — sem buraco. O painel
/// de tarefas entra aqui (não no conteúdo) pra ficar fixo, mesmo ocioso.
pub fn chrome_height(app: &App) -> u16 {
    let editor = (app.editor.line_count() as u16).clamp(1, EDITOR_MAX);
    1 + (editor + 2) + 2 + tasks_height(app)
}

/// Altura desejada do viewport inline: cromo + overlay + conteúdo ativo (capado).
/// O loop redimensiona o viewport pra bater com isso.
pub fn desired_height(app: &App, width: u16) -> u16 {
    let overlay_h = match &app.overlay {
        Some(o @ (Overlay::Slash(_) | Overlay::Mention(_))) => o.height() as u16,
        _ => 0,
    };
    let content_h = match &app.overlay {
        Some(o @ Overlay::Select(_)) => o.height() as u16,
        _ => {
            let lines = build_active_lines(app);
            if lines.is_empty() {
                0
            } else {
                let para = Paragraph::new(lines).wrap(Wrap { trim: false });
                (para.line_count(width.max(1)) as u16).min(CONTENT_CAP)
            }
        }
    };
    chrome_height(app) + overlay_h + content_h
}

/// Verdadeiro quando NADA precisa de espaço extra — o viewport encolhe pro cromo
/// (sem buraco). Só encolhe fora de um turno pra não piscar no meio do streaming.
pub fn is_idle(app: &App) -> bool {
    !app.running
        && app.overlay.is_none()
        && app.thinking.is_none()
        && app.assistant.is_none()
        && app.tools.is_empty()
        && app.subagents.is_empty()
        && app.question.is_none()
        && app.attachments.is_empty()
}

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

    // A paleta de slash e o autocomplete de @ aparecem logo acima da caixa (e
    // escondem a linha de status); os seletores (Select) aparecem no conteúdo.
    let slash_open = matches!(app.overlay, Some(Overlay::Slash(_)) | Some(Overlay::Mention(_)));
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
    // Painel de tarefas: fixo logo acima do slash/status/input. Cede espaço a
    // eles (e ao footer/box) e fica com o que sobra; o conteúdo cede ao painel.
    let tasks_h = tasks_height(app)
        .min(area.height.saturating_sub(footer_h + box_h + status_h + slash_h));
    let content_h = area
        .height
        .saturating_sub(footer_h + box_h + status_h + slash_h + tasks_h);

    let mut y = area.y;
    let content = Rect::new(area.x, y, width, content_h);
    y += content_h;
    let tasks = Rect::new(area.x, y, width, tasks_h);
    y += tasks_h;
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

    // Painel fixo de tarefas (sem borda, sem wrap → clipa títulos longos, então
    // a altura renderizada bate com `tasks_height`).
    if tasks_h > 0 {
        f.render_widget(Paragraph::new(tasks_panel_lines(app)), tasks);
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
                format!("reconnecting ({attempt}/{max}) in {secs}s…"),
                theme.muted_style(),
            ),
            Span::styled("  (Ctrl+C cancels)".to_string(), theme.dim_style()),
        ]);
    }
    if app.question.is_some() {
        return Line::from(Span::styled(
            "answer the question above  ·  Enter sends · digit picks an option".to_string(),
            theme.dim_style(),
        ));
    }
    if app.running {
        let sp = spinner_frame(app.spinner_tick);
        // Enquanto raciocina, o indicador fixo é "Thinking…" (igual à barra de
        // thinking do app); depois vira "working…".
        let (label, label_style) = if app.thinking.is_some() {
            ("Thinking…", theme.fg(theme.thinking))
        } else {
            ("working…", theme.muted_style())
        };
        return Line::from(vec![
            Span::styled(format!("{sp} "), theme.fg(theme.accent)),
            Span::styled(label.to_string(), label_style),
            Span::styled("  (Ctrl+C interrupts · Enter queues)".to_string(), theme.dim_style()),
        ]);
    }
    Line::from(Span::styled(
        "Enter send · Tab mode · / commands · @ files · Ctrl+C quit".to_string(),
        theme.dim_style(),
    ))
}

/// Altura do painel de tarefas: header (1) + até TASKS_CAP tarefas + "+N more".
/// Zero quando não há tarefas (o painel some).
fn tasks_height(app: &App) -> u16 {
    let n = app.tasks.len() as u16;
    if n == 0 {
        return 0;
    }
    let overflow = if n > TASKS_CAP { 1 } else { 0 };
    1 + n.min(TASKS_CAP) + overflow
}

/// Painel fixo de tarefas (acima do input): uma linha de contagem por status +
/// até TASKS_CAP tarefas (em-andamento primeiro: doing → todo → done), com
/// "+N more" no overflow. Renderizado sem wrap — títulos longos são clipados,
/// então a altura bate com `tasks_height`. Cores espelham o TasksPanel do app:
/// done=verde, doing=âmbar, todo=cinza; tarefas golden ganham um ★ de destaque.
fn tasks_panel_lines(app: &App) -> Vec<Line<'static>> {
    let theme = &app.theme;
    if app.tasks.is_empty() {
        return Vec::new();
    }

    let (mut done, mut doing, mut todo) = (0u16, 0u16, 0u16);
    for t in &app.tasks {
        match t.status.as_str() {
            "done" => done += 1,
            "doing" => doing += 1,
            _ => todo += 1,
        }
    }

    let header = Line::from(vec![
        Span::styled("Tasks  ".to_string(), theme.muted_style()),
        Span::styled(format!("✓ {done}"), theme.fg(theme.success)),
        Span::styled("   ".to_string(), theme.dim_style()),
        Span::styled(format!("● {doing}"), theme.fg(theme.warning)),
        Span::styled("   ".to_string(), theme.dim_style()),
        Span::styled(format!("○ {todo}"), theme.muted_style()),
    ]);

    // Em-andamento primeiro pra caber o acionável no cap; a ordenação estável
    // preserva a ordem original dentro de cada status.
    let rank = |s: &str| match s {
        "doing" => 0u8,
        "done" => 2,
        _ => 1, // todo
    };
    let mut idx: Vec<usize> = (0..app.tasks.len()).collect();
    idx.sort_by_key(|&i| rank(app.tasks[i].status.as_str()));

    let mut lines = vec![header];
    for &i in idx.iter().take(TASKS_CAP as usize) {
        let t = &app.tasks[i];
        let (glyph, glyph_style) = match t.status.as_str() {
            "done" => ("✓", theme.fg(theme.success)),
            "doing" => ("●", theme.fg(theme.warning)),
            _ => ("○", theme.muted_style()),
        };
        let mut spans = vec![Span::styled(format!("  {glyph} "), glyph_style)];
        if t.id.starts_with("golden-") {
            spans.push(Span::styled("★ ".to_string(), theme.fg(theme.accent)));
        }
        let title_style = if t.status == "done" {
            theme.dim_style()
        } else {
            theme.fg(theme.text)
        };
        spans.push(Span::styled(t.title.replace('\n', " "), title_style));
        lines.push(Line::from(spans));
    }

    let n = app.tasks.len() as u16;
    if n > TASKS_CAP {
        lines.push(Line::from(Span::styled(
            format!("  +{} more", n - TASKS_CAP),
            theme.dim_style(),
        )));
    }
    lines
}

/// Concatena o conteúdo em progresso para a área de conteúdo (sem borda):
/// assistant em streaming, cards de ferramenta, subagentes em voo, o card de
/// pergunta e as pílulas de anexo. (Thinking é um indicador fixo na status
/// line, não entra aqui.) Ancorado embaixo (tail) pelo `draw`.
fn build_active_lines(app: &App) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Thinking não entra no conteúdo — vira um indicador fixo na status line.
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
            format!("attachments: {pills}"),
            theme.fg(theme.accent),
        )));
    }
    lines
}
