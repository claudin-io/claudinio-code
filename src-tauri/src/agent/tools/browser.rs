//! The `browser` and `browser_screenshot` tools.
//!
//! Thin adapters over `crate::browser`: parse args, resolve the workspace's
//! browser handle, call the page API, shape the result. All the protocol work
//! lives in the `browser` module.

use super::{ToolContext, ToolOutput};
use crate::browser::page::{clamp_timeout, validate_url};
use crate::browser::screenshot::{self, Rect, Target};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct BrowserArgs {
    pub action: String,
    pub url: Option<String>,
    pub selector: Option<String>,
    pub text: Option<String>,
    pub key: Option<String>,
    #[serde(default)]
    pub clear_first: bool,
    #[serde(default)]
    pub submit: bool,
    #[serde(default)]
    pub wait_for_load: Option<bool>,
    pub timeout_ms: Option<u64>,
}

impl BrowserArgs {
    fn need_selector(&self, action: &str) -> Result<&str, String> {
        self.selector
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("{action} requires a `selector`"))
    }
}

#[derive(Deserialize)]
pub struct ScreenshotArgs {
    #[serde(default)]
    pub target: Option<String>,
    pub selector: Option<String>,
    pub rect: Option<RectArgs>,
    pub label: Option<String>,
}

#[derive(Deserialize)]
pub struct RectArgs {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Resolve the workspace's browser, reporting the actionable reason when it is
/// not available rather than a bare "None".
async fn manager(
    ctx: &ToolContext,
) -> Result<std::sync::Arc<crate::browser::BrowserManager>, String> {
    let handle = ctx
        .browser
        .as_ref()
        .ok_or("the browser is not available in this session")?;
    let prefs = ctx
        .agent_config
        .as_ref()
        .map(|c| c.browser.clone())
        .unwrap_or_default();
    if !prefs.enabled {
        return Err("the browser tools are disabled in settings".into());
    }
    handle.ensure(&prefs).await
}

pub async fn execute(args: BrowserArgs, ctx: &ToolContext) -> Result<ToolOutput, String> {
    let wait = args.wait_for_load.unwrap_or(true);
    let timeout = clamp_timeout(args.timeout_ms);

    // `close` must not launch a browser just to shut it down.
    if args.action == "close" {
        if let Some(handle) = ctx.browser.as_ref() {
            handle.shutdown().await;
        }
        return Ok(ToolOutput::Text {
            content: "Browser closed.".into(),
        });
    }

    let manager = manager(ctx).await?;
    let page = manager.page().await?;
    let interrupt = ctx.interrupt.as_ref();

    let content = match args.action.as_str() {
        "navigate" => {
            let url = args
                .url
                .as_deref()
                .ok_or("navigate requires a `url`")?
                .trim();
            validate_url(url)?;
            let landed = page.navigate(url, wait, timeout, interrupt).await?;
            format!("Navigated to {landed}")
        }
        "reload" => {
            let landed = page.reload(wait, timeout, interrupt).await?;
            format!("Reloaded {landed}")
        }
        "back" => {
            let landed = page.back(wait, timeout, interrupt).await?;
            format!("Went back to {landed}")
        }
        "url" => format!("Current URL: {}", page.current_url().await?),
        "scroll_to" => {
            let selector = args.need_selector("scroll_to")?;
            page.scroll_to(selector).await?;
            format!("Scrolled {selector} into view")
        }
        "focus" => {
            let selector = args.need_selector("focus")?;
            page.focus(selector).await?;
            format!("Focused {selector}")
        }
        "click" => {
            let selector = args.need_selector("click")?;
            page.click(selector).await?;
            format!("Clicked {selector}")
        }
        "type" => {
            let selector = args.need_selector("type")?;
            let text = args.text.as_deref().ok_or("type requires `text`")?;
            page.type_text(selector, text, args.clear_first, args.submit)
                .await?;
            // Never echo what was typed: this is the path a password goes
            // through, and the transcript is persisted to disk.
            format!(
                "Typed {} character(s) into {selector}{}",
                text.chars().count(),
                if args.submit {
                    " and pressed Enter"
                } else {
                    ""
                }
            )
        }
        "press" => {
            let key = args.key.as_deref().ok_or("press requires a `key`")?;
            if let Some(selector) = args.selector.as_deref().filter(|s| !s.trim().is_empty()) {
                page.focus(selector).await?;
            }
            page.press(key).await?;
            format!("Pressed {key}")
        }
        "wait_for" => {
            let selector = args.need_selector("wait_for")?;
            page.wait_for(selector, timeout, interrupt).await?;
            format!("{selector} appeared")
        }
        other => {
            return Err(format!(
                "unknown browser action '{other}'. Available: navigate, reload, back, url, \
                 click, type, press, scroll_to, focus, wait_for, close."
            ));
        }
    };
    Ok(ToolOutput::Text { content })
}

#[derive(Deserialize)]
pub struct InspectArgs {
    pub what: String,
    pub selector: Option<String>,
    pub filter: Option<String>,
    pub only_new: Option<bool>,
    pub limit: Option<usize>,
}

const MAX_INSPECT_ENTRIES: usize = 200;
/// Enough of the DOM to reason about, not so much that one call eats the turn.
const MAX_PAGE_TEXT: usize = 24_000;

pub async fn inspect(args: InspectArgs, ctx: &ToolContext) -> Result<ToolOutput, String> {
    let manager = manager(ctx).await?;
    let page = manager.page().await?;
    let url = page.current_url().await.unwrap_or_default();
    let limit = args.limit.unwrap_or(50).clamp(1, MAX_INSPECT_ENTRIES);
    // Reading in a loop is the normal pattern, so default to "just what is new".
    let only_new = args.only_new.unwrap_or(true);

    let body = match args.what.as_str() {
        "console" => {
            let entries = {
                let mut buffers = page.buffers.lock().await;
                buffers.console(only_new, args.filter.as_deref(), limit)
            };
            if entries.is_empty() {
                return Ok(ToolOutput::Text {
                    content: if only_new {
                        "No new console output since the last read.".into()
                    } else {
                        "The console is empty.".into()
                    },
                });
            }
            entries
                .iter()
                .map(|e| {
                    let where_ = match (&e.url, e.line) {
                        (Some(u), Some(l)) => format!("  ({u}:{l})"),
                        (Some(u), None) => format!("  ({u})"),
                        _ => String::new(),
                    };
                    format!("[{}] {}{where_}", e.level, e.text)
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        "network" => {
            let entries = {
                let mut buffers = page.buffers.lock().await;
                buffers.network(only_new, args.filter.as_deref(), limit)
            };
            if entries.is_empty() {
                return Ok(ToolOutput::Text {
                    content: if only_new {
                        "No new network activity since the last read.".into()
                    } else {
                        "No network activity recorded.".into()
                    },
                });
            }
            entries
                .iter()
                .map(|e| {
                    let status = match (&e.error, e.status) {
                        (Some(err), _) => format!("FAILED {err}"),
                        (None, Some(s)) => s.to_string(),
                        (None, None) => "pending".into(),
                    };
                    let size = e
                        .size_bytes
                        .map(|b| format!("  {}B", human_bytes(b)))
                        .unwrap_or_default();
                    let ms = e
                        .duration_ms
                        .map(|d| format!("  {d}ms"))
                        .unwrap_or_default();
                    let cached = if e.from_cache { "  (cache)" } else { "" };
                    format!(
                        "{} {} {} [{}]{size}{ms}{cached}",
                        e.method, status, e.url, e.resource_type
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        "text" => truncate_chars(
            &page.inner_text(args.selector.as_deref()).await?,
            MAX_PAGE_TEXT,
        ),
        "html" => truncate_chars(
            &page.outer_html(args.selector.as_deref()).await?,
            MAX_PAGE_TEXT,
        ),
        other => {
            return Err(format!(
                "unknown inspect target '{other}'. Available: console, network, text, html."
            ));
        }
    };

    // Everything above is authored by whoever controls the page.
    Ok(ToolOutput::Text {
        content: crate::browser::page::wrap_untrusted(&url, &body),
    })
}

fn human_bytes(b: u64) -> String {
    if b >= 1_048_576 {
        format!("{:.1}M", b as f64 / 1_048_576.0)
    } else if b >= 1024 {
        format!("{:.1}K", b as f64 / 1024.0)
    } else {
        b.to_string()
    }
}

fn truncate_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let cut: String = s.chars().take(n).collect();
    format!("{cut}\n… (truncated at {n} characters)")
}

pub async fn screenshot(args: ScreenshotArgs, ctx: &ToolContext) -> Result<ToolOutput, String> {
    let target = match args.target.as_deref().unwrap_or("viewport") {
        "viewport" => Target::Viewport,
        "full_page" => Target::FullPage,
        "selector" => Target::Selector(
            args.selector
                .clone()
                .ok_or("target=selector requires a `selector`")?,
        ),
        "rect" => {
            let r = args.rect.as_ref().ok_or("target=rect requires a `rect`")?;
            Target::Rect(Rect {
                x: r.x,
                y: r.y,
                width: r.width,
                height: r.height,
            })
        }
        other => {
            return Err(format!(
                "unknown screenshot target '{other}'. Available: viewport, full_page, selector, rect."
            ));
        }
    };

    let manager = manager(ctx).await?;
    let page = manager.page().await?;
    let url = page.current_url().await.unwrap_or_default();
    if url.is_empty() || url == "about:blank" {
        return Err("no page is open — call the browser tool with action=navigate first".into());
    }

    let capture = screenshot::capture(&page, &target).await?;
    let mut content = format!(
        "Screenshot of {url} ({}x{})",
        capture.image.width, capture.image.height
    );
    if let Some(label) = args.label.as_deref().filter(|l| !l.trim().is_empty()) {
        content = format!("{content} — {label}");
    }
    if let Some(note) = capture.note {
        content = format!("{content}\nNote: {note}");
    }

    Ok(ToolOutput::Rich {
        content,
        images: vec![capture.image],
    })
}
