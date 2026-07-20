//! Footer de status (2 linhas) e helpers de formatação, portados de
//! `packages/coding-agent/src/modes/interactive/components/footer.ts` do `pi`:
//! `formatTokens`, custo acumulado e graduação de cor do uso de contexto.
//! Alimentado pelo `SessionStats` que o core já emite.

use super::theme::Theme;
use ratatui::style::Color;
use ratatui::text::{Line, Span};

/// Compacta contagens de token: 999, 1.2k, 15k, 1.5M, 15M.
pub fn format_tokens(count: u64) -> String {
    if count < 1000 {
        count.to_string()
    } else if count < 10_000 {
        format!("{:.1}k", count as f64 / 1000.0)
    } else if count < 1_000_000 {
        format!("{}k", (count as f64 / 1000.0).round() as u64)
    } else if count < 10_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else {
        format!("{}M", (count as f64 / 1_000_000.0).round() as u64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtxSeverity {
    Normal,
    Warn,
    Error,
}

/// Graduação do uso de contexto: >90% erro, >70% aviso, senão normal
/// (mesmos limiares do footer.ts do `pi`).
pub fn ctx_severity(percent: f64) -> CtxSeverity {
    if percent > 90.0 {
        CtxSeverity::Error
    } else if percent > 70.0 {
        CtxSeverity::Warn
    } else {
        CtxSeverity::Normal
    }
}

/// Frames de spinner braille (mesmo estilo do `pi`/muitos TUIs).
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub fn spinner_frame(tick: u64) -> &'static str {
    SPINNER[(tick as usize) % SPINNER.len()]
}

/// Dados escalares que o footer precisa, montados a cada frame pelo `render`.
pub struct FooterInfo {
    pub cwd: String, // já relativizado (~) e com "(branch)" se houver
    pub mode: &'static str,
    pub model: String,
    pub effort: String,
    pub in_tok: u64,
    pub out_tok: u64,
    pub cost: Option<f64>,
    pub is_sub: bool,
    pub context_tokens: u64,
    pub max_context_tokens: u64,
}

fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else if width <= 1 {
        s.chars().take(width).collect()
    } else {
        let take = width.saturating_sub(1);
        format!("{}…", s.chars().take(take).collect::<String>())
    }
}

/// Renderiza as 2 linhas do footer: cwd(+branch) / stats + modelo·effort.
pub fn render_footer(info: &FooterInfo, theme: &Theme, width: u16) -> Vec<Line<'static>> {
    let width = width as usize;

    let l1 = Line::from(Span::styled(truncate(&info.cwd, width), theme.dim_style()));

    let mut left: Vec<Span<'static>> = Vec::new();
    left.push(Span::styled(format!("{} ", info.mode), theme.fg(theme.accent)));
    left.push(Span::styled(
        format!("↑{} ", format_tokens(info.in_tok)),
        theme.dim_style(),
    ));
    left.push(Span::styled(
        format!("↓{} ", format_tokens(info.out_tok)),
        theme.dim_style(),
    ));
    if let Some(cost) = info.cost {
        let sub = if info.is_sub { " (sub)" } else { "" };
        left.push(Span::styled(
            format!("${cost:.3}{sub} "),
            theme.dim_style(),
        ));
    }
    if info.max_context_tokens > 0 {
        let pct = info.context_tokens as f64 / info.max_context_tokens as f64 * 100.0;
        let color: Color = match ctx_severity(pct) {
            CtxSeverity::Error => theme.error,
            CtxSeverity::Warn => theme.warning,
            CtxSeverity::Normal => theme.muted,
        };
        left.push(Span::styled(
            format!("{}%/{}", pct.round() as u64, format_tokens(info.max_context_tokens)),
            theme.fg(color),
        ));
    }

    let right = format!("{}·{}", info.model, info.effort);
    let left_w: usize = left.iter().map(|s| s.content.chars().count()).sum();
    let right_w = right.chars().count();

    let mut spans = left;
    if left_w + right_w < width {
        let pad = width - left_w - right_w;
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(right, theme.muted_style()));
    }
    let l2 = Line::from(spans);
    vec![l1, l2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_tokens() {
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1000), "1.0k");
        assert_eq!(format_tokens(1500), "1.5k");
        assert_eq!(format_tokens(15_000), "15k");
        assert_eq!(format_tokens(1_500_000), "1.5M");
        assert_eq!(format_tokens(15_000_000), "15M");
    }

    #[test]
    fn context_severity_thresholds() {
        assert_eq!(ctx_severity(0.0), CtxSeverity::Normal);
        assert_eq!(ctx_severity(70.0), CtxSeverity::Normal);
        assert_eq!(ctx_severity(70.1), CtxSeverity::Warn);
        assert_eq!(ctx_severity(90.0), CtxSeverity::Warn);
        assert_eq!(ctx_severity(90.1), CtxSeverity::Error);
    }

    #[test]
    fn spinner_cycles() {
        assert_eq!(spinner_frame(0), "⠋");
        assert_eq!(spinner_frame(10), "⠋");
        assert_ne!(spinner_frame(0), spinner_frame(1));
    }

    #[test]
    fn footer_two_lines_and_model_on_right() {
        let t = Theme::dark();
        let info = FooterInfo {
            cwd: "~/proj (main)".into(),
            mode: "brain",
            model: "opus".into(),
            effort: "high".into(),
            in_tok: 1200,
            out_tok: 340,
            cost: Some(0.014),
            is_sub: false,
            context_tokens: 24_000,
            max_context_tokens: 200_000,
        };
        let lines = render_footer(&info, &t, 80);
        assert_eq!(lines.len(), 2);
        let l2: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(l2.contains("opus·high"), "linha 2 = {l2:?}");
        assert!(l2.contains("↑1.2k"));
        assert!(l2.contains("$0.014"));
        assert!(l2.contains("12%/200k"));
    }
}
