//! Capturing pixels: the whole viewport, the whole page, one element, or an
//! arbitrary region.
//!
//! The geometry is the fiddly part, so it lives in pure functions below the
//! async wrappers, where it can be tested without a browser. Three facts drive
//! all of it:
//!
//! 1. `clip` is in **CSS pixels**, and `clip.scale` multiplies CSS pixels into
//!    output pixels. It is not the display's device pixel ratio — `PageSession`
//!    pins `deviceScaleFactor: 1` so the two never have to be reconciled.
//! 2. With `captureBeyondViewport: true`, `clip` is read in **document**
//!    coordinates (origin at the top of the page), not viewport coordinates.
//! 3. `DOM.getBoxModel` returns quads in **viewport** coordinates, so they have
//!    to be shifted by the visual viewport's page offset before use.

use super::page::PageSession;
use crate::imageutil::{ImageAttachment, MAX_IMAGE_DIM};
use serde_json::{Value, json};

/// Above this the compositor starts returning black or failing outright, and
/// the image would be unreadable anyway.
pub const MAX_FULL_PAGE_HEIGHT: f64 = 8_000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone)]
pub enum Target {
    Viewport,
    FullPage,
    Selector(String),
    Rect(Rect),
}

pub struct Capture {
    pub image: ImageAttachment,
    /// Anything the model should know about how the capture was adjusted.
    pub note: Option<String>,
}

pub async fn capture(page: &PageSession, target: &Target) -> Result<Capture, String> {
    match target {
        Target::Viewport => {
            let data = raw_capture(page, None).await?;
            Ok(Capture {
                image: decode(&data),
                note: None,
            })
        }
        Target::FullPage => {
            let metrics = layout_metrics(page).await?;
            let (height, clamped) = clamp_full_page_height(metrics.content_height);
            let rect = Rect {
                x: 0.0,
                y: 0.0,
                width: metrics.content_width,
                height,
            };
            let data = raw_capture(page, Some(rect)).await?;
            Ok(Capture {
                image: decode(&data),
                note: clamped.then(|| {
                    format!(
                        "the page is {:.0}px tall; captured the top {MAX_FULL_PAGE_HEIGHT:.0}px. \
                         Use a selector or a region to see further down.",
                        metrics.content_height
                    )
                }),
            })
        }
        Target::Selector(selector) => {
            let rect = element_rect(page, selector).await?;
            let data = raw_capture(page, Some(rect)).await?;
            Ok(Capture {
                image: decode(&data),
                note: None,
            })
        }
        Target::Rect(rect) => {
            if rect.width <= 0.0 || rect.height <= 0.0 {
                return Err("the region has zero width or height".into());
            }
            let data = raw_capture(page, Some(*rect)).await?;
            Ok(Capture {
                image: decode(&data),
                note: None,
            })
        }
    }
}

/// Locate an element and return its bounds in document coordinates.
pub async fn element_rect(page: &PageSession, selector: &str) -> Result<Rect, String> {
    let doc = page
        .conn
        .call(page.sid(), "DOM.getDocument", json!({"depth": 0}))
        .await?;
    let root = doc
        .pointer("/root/nodeId")
        .and_then(Value::as_i64)
        .ok_or("could not read the document root")?;

    let found = page
        .conn
        .call(
            page.sid(),
            "DOM.querySelector",
            json!({"nodeId": root, "selector": selector}),
        )
        .await?;
    // A miss is `nodeId: 0`, not a protocol error. Without this check the next
    // call fails with "Could not find node with given id", which tells the
    // model nothing about what actually went wrong.
    let node_id = found.get("nodeId").and_then(Value::as_i64).unwrap_or(0);
    if node_id == 0 {
        return Err(format!("no element matches '{selector}'"));
    }

    // Scroll it in first: an element that was never laid out has no box.
    let _ = page
        .conn
        .call(
            page.sid(),
            "DOM.scrollIntoViewIfNeeded",
            json!({"nodeId": node_id}),
        )
        .await;

    let box_model = page
        .conn
        .call(page.sid(), "DOM.getBoxModel", json!({"nodeId": node_id}))
        .await
        .map_err(|e| format!("'{selector}' has no visible box ({e})"))?;
    let quad: Vec<f64> = box_model
        .pointer("/model/border")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    let viewport_rect =
        quad_to_rect(&quad).ok_or_else(|| format!("'{selector}' returned an unreadable box"))?;
    if viewport_rect.width <= 0.0 || viewport_rect.height <= 0.0 {
        return Err(format!(
            "'{selector}' matches but is not visible (0x0 — display:none, or collapsed)"
        ));
    }

    let metrics = layout_metrics(page).await?;
    Ok(to_document_coords(
        viewport_rect,
        metrics.page_x,
        metrics.page_y,
    ))
}

pub struct LayoutMetrics {
    pub content_width: f64,
    pub content_height: f64,
    pub page_x: f64,
    pub page_y: f64,
}

pub async fn layout_metrics(page: &PageSession) -> Result<LayoutMetrics, String> {
    let m = page
        .conn
        .call(page.sid(), "Page.getLayoutMetrics", json!({}))
        .await?;
    let num = |ptr: &str| m.pointer(ptr).and_then(Value::as_f64).unwrap_or(0.0);
    Ok(LayoutMetrics {
        content_width: num("/cssContentSize/width"),
        content_height: num("/cssContentSize/height"),
        page_x: num("/cssVisualViewport/pageX"),
        page_y: num("/cssVisualViewport/pageY"),
    })
}

async fn raw_capture(page: &PageSession, clip: Option<Rect>) -> Result<String, String> {
    let mut params = json!({
        "format": "jpeg",
        "quality": 80,
        "fromSurface": true,
    });
    if let Some(rect) = clip {
        params["captureBeyondViewport"] = json!(true);
        params["clip"] = json!({
            "x": rect.x,
            "y": rect.y,
            "width": rect.width,
            "height": rect.height,
            "scale": capture_scale(rect.width, rect.height),
        });
    }
    let result = page
        .conn
        .call_with_timeout(
            page.sid(),
            "Page.captureScreenshot",
            params,
            super::cdp::SCREENSHOT_TIMEOUT,
        )
        .await?;
    result
        .get("data")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "Page.captureScreenshot returned no image data".to_string())
}

/// Re-encode through the shared compressor, which also measures the result so
/// the token estimator can price it by pixels.
fn decode(base64_png: &str) -> ImageAttachment {
    ImageAttachment::from_base64(base64_png, "image/jpeg")
}

// ── pure geometry ───────────────────────────────────────────────────

/// The border quad is 8 numbers: four (x, y) corners, clockwise from top-left.
/// Rotated or skewed elements give a non-axis-aligned quad, so take the
/// bounding box rather than assuming corner 0 and corner 2 are opposite.
pub fn quad_to_rect(quad: &[f64]) -> Option<Rect> {
    if quad.len() < 8 {
        return None;
    }
    let xs: Vec<f64> = quad.iter().step_by(2).copied().collect();
    let ys: Vec<f64> = quad.iter().skip(1).step_by(2).copied().collect();
    let min_x = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let max_x = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min_y = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let max_y = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !min_x.is_finite() || !min_y.is_finite() {
        return None;
    }
    Some(Rect {
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    })
}

/// Shift a viewport-relative rect into document coordinates, which is what
/// `clip` expects once `captureBeyondViewport` is on.
pub fn to_document_coords(rect: Rect, page_x: f64, page_y: f64) -> Rect {
    Rect {
        x: rect.x + page_x,
        y: rect.y + page_y,
        ..rect
    }
}

/// Downscale factor keeping the longest edge within the model's useful limit.
///
/// Doing it here rather than after the fact means Chromium never encodes the
/// oversized image in the first place.
pub fn capture_scale(width: f64, height: f64) -> f64 {
    let longest = width.max(height);
    if longest <= 0.0 {
        return 1.0;
    }
    (MAX_IMAGE_DIM as f64 / longest).min(1.0)
}

pub fn clamp_full_page_height(height: f64) -> (f64, bool) {
    if height > MAX_FULL_PAGE_HEIGHT {
        (MAX_FULL_PAGE_HEIGHT, true)
    } else {
        (height.max(1.0), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }

    #[test]
    fn quad_becomes_its_bounding_box() {
        // top-left (10,20), 100 wide, 50 tall
        let quad = [10.0, 20.0, 110.0, 20.0, 110.0, 70.0, 10.0, 70.0];
        let r = quad_to_rect(&quad).unwrap();
        approx(r.x, 10.0);
        approx(r.y, 20.0);
        approx(r.width, 100.0);
        approx(r.height, 50.0);
    }

    /// A rotated element's quad is not axis-aligned; taking corners 0 and 2
    /// would produce a wrong (and possibly negative) size.
    #[test]
    fn a_rotated_quad_uses_the_bounding_box_not_opposite_corners() {
        let quad = [50.0, 0.0, 100.0, 50.0, 50.0, 100.0, 0.0, 50.0];
        let r = quad_to_rect(&quad).unwrap();
        approx(r.x, 0.0);
        approx(r.y, 0.0);
        approx(r.width, 100.0);
        approx(r.height, 100.0);
    }

    #[test]
    fn a_short_quad_is_rejected() {
        assert!(quad_to_rect(&[1.0, 2.0]).is_none());
        assert!(quad_to_rect(&[]).is_none());
    }

    /// The conversion that makes element capture work: getBoxModel is
    /// viewport-relative, clip is document-relative.
    #[test]
    fn viewport_coords_shift_by_the_page_scroll_offset() {
        let viewport_rect = Rect {
            x: 10.0,
            y: 30.0,
            width: 100.0,
            height: 50.0,
        };
        let doc = to_document_coords(viewport_rect, 0.0, 2000.0);
        approx(doc.x, 10.0);
        approx(doc.y, 2030.0);
        // Size must not change — only the origin moves.
        approx(doc.width, 100.0);
        approx(doc.height, 50.0);
    }

    #[test]
    fn small_captures_are_not_upscaled() {
        approx(capture_scale(800.0, 600.0), 1.0);
        approx(capture_scale(MAX_IMAGE_DIM as f64, 100.0), 1.0);
    }

    #[test]
    fn oversized_captures_scale_down_by_the_longest_edge() {
        approx(capture_scale(3136.0, 100.0), 0.5);
        approx(capture_scale(100.0, 3136.0), 0.5);
    }

    #[test]
    fn a_degenerate_size_does_not_divide_by_zero() {
        approx(capture_scale(0.0, 0.0), 1.0);
    }

    #[test]
    fn full_page_height_is_clamped_and_reports_it() {
        assert_eq!(clamp_full_page_height(1_200.0), (1_200.0, false));
        assert_eq!(
            clamp_full_page_height(50_000.0),
            (MAX_FULL_PAGE_HEIGHT, true)
        );
        // A zero-height document still has to produce a legal clip.
        assert_eq!(clamp_full_page_height(0.0), (1.0, false));
    }
}
