//! Paleta semântica da TUI. Portada do tema `dark.json` do `pi`
//! (earendil-works/pi) com uma variante `light`. Cores nomeadas por função
//! (não por matiz) para espelhar o `ThemePicker` do app desktop.

use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeKind {
    Dark,
    Light,
}

impl ThemeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeKind::Dark => "dark",
            ThemeKind::Light => "light",
        }
    }
}

/// Cores semânticas resolvidas para um tema. Campos são `Color` do ratatui
/// (truecolor `Rgb`), consumidos pelos renderizadores de markdown/diff/footer.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub text: Color,
    pub accent: Color,
    pub border_accent: Color,
    pub border_muted: Color,
    pub success: Color,
    pub error: Color,
    pub warning: Color,
    pub muted: Color,
    pub dim: Color,

    pub selected_bg: Color,
    pub tool_title: Color,

    pub md_heading: Color,
    pub md_link: Color,
    pub md_code: Color,
    pub md_code_block: Color,
    pub md_quote: Color,
    pub md_list_bullet: Color,

    pub diff_added: Color,
    pub diff_removed: Color,
    pub diff_context: Color,

    pub thinking: Color,
    pub subagent: Color,
}

impl Theme {
    pub fn from_kind(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::Dark => Self::dark(),
            ThemeKind::Light => Self::light(),
        }
    }

    /// Portado de `packages/coding-agent/src/modes/interactive/theme/dark.json`.
    pub fn dark() -> Self {
        Theme {
            text: Color::Rgb(0xd4, 0xd4, 0xd4),
            accent: Color::Rgb(0x8a, 0xbe, 0xb7),
            border_accent: Color::Rgb(0x00, 0xd7, 0xff),
            border_muted: Color::Rgb(0x50, 0x50, 0x50),
            success: Color::Rgb(0xb5, 0xbd, 0x68),
            error: Color::Rgb(0xcc, 0x66, 0x66),
            warning: Color::Rgb(0xf0, 0xc6, 0x74),
            muted: Color::Rgb(0x80, 0x80, 0x80),
            dim: Color::Rgb(0x66, 0x66, 0x66),
            selected_bg: Color::Rgb(0x3a, 0x3a, 0x4a),
            tool_title: Color::Rgb(0xd4, 0xd4, 0xd4),
            md_heading: Color::Rgb(0xf0, 0xc6, 0x74),
            md_link: Color::Rgb(0x81, 0xa2, 0xbe),
            md_code: Color::Rgb(0x8a, 0xbe, 0xb7),
            md_code_block: Color::Rgb(0xb5, 0xbd, 0x68),
            md_quote: Color::Rgb(0x80, 0x80, 0x80),
            md_list_bullet: Color::Rgb(0x8a, 0xbe, 0xb7),
            diff_added: Color::Rgb(0xb5, 0xbd, 0x68),
            diff_removed: Color::Rgb(0xcc, 0x66, 0x66),
            diff_context: Color::Rgb(0x80, 0x80, 0x80),
            thinking: Color::Rgb(0x80, 0x80, 0x80),
            subagent: Color::Rgb(0xb2, 0x94, 0xbb),
        }
    }

    /// Variante clara: mesmo esqueleto semântico, tons legíveis em fundo claro.
    pub fn light() -> Self {
        Theme {
            text: Color::Rgb(0x1a, 0x1a, 0x1a),
            accent: Color::Rgb(0x0f, 0x76, 0x6e),
            border_accent: Color::Rgb(0x0b, 0x7a, 0x9e),
            border_muted: Color::Rgb(0xbf, 0xbf, 0xbf),
            success: Color::Rgb(0x3f, 0x6e, 0x1f),
            error: Color::Rgb(0xb3, 0x2a, 0x2a),
            warning: Color::Rgb(0x9a, 0x6a, 0x00),
            muted: Color::Rgb(0x6a, 0x6a, 0x6a),
            dim: Color::Rgb(0x9a, 0x9a, 0x9a),
            selected_bg: Color::Rgb(0xdf, 0xe4, 0xf2),
            tool_title: Color::Rgb(0x1a, 0x1a, 0x1a),
            md_heading: Color::Rgb(0x9a, 0x6a, 0x00),
            md_link: Color::Rgb(0x2a, 0x54, 0xc8),
            md_code: Color::Rgb(0x0f, 0x76, 0x6e),
            md_code_block: Color::Rgb(0x3f, 0x6e, 0x1f),
            md_quote: Color::Rgb(0x6a, 0x6a, 0x6a),
            md_list_bullet: Color::Rgb(0x0f, 0x76, 0x6e),
            diff_added: Color::Rgb(0x3f, 0x6e, 0x1f),
            diff_removed: Color::Rgb(0xb3, 0x2a, 0x2a),
            diff_context: Color::Rgb(0x6a, 0x6a, 0x6a),
            thinking: Color::Rgb(0x6a, 0x6a, 0x6a),
            subagent: Color::Rgb(0x7a, 0x3f, 0x9a),
        }
    }

    // Helpers de estilo usados por vários renderizadores.
    pub fn fg(&self, c: Color) -> Style {
        Style::default().fg(c)
    }
    pub fn dim_style(&self) -> Style {
        Style::default().fg(self.dim)
    }
    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.muted)
    }
    pub fn accent_bold(&self) -> Style {
        Style::default().fg(self.accent).add_modifier(Modifier::BOLD)
    }
}
