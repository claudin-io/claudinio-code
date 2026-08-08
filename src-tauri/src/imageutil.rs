//! Image handling shared by every producer of model-visible images.
//!
//! This lives at the crate root rather than in `commands/` because the core
//! layers need it too: `agent::mcp` re-hydrates images returned by MCP servers,
//! and the browser tools compress screenshots. `architecture_tests` in `lib.rs`
//! forbids `agent/` from importing `crate::commands`, so a helper used by both
//! sides cannot stay in the adapter layer.

use base64::Engine;
use image::GenericImageView;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

/// The Anthropic server-side resize threshold. Beyond this the provider
/// downsizes anyway, so sending more pixels costs bandwidth with no quality or
/// accuracy gain.
pub const MAX_IMAGE_DIM: u32 = 1568;

/// An image ready to be handed to a model: already compressed, already base64.
///
/// `width`/`height` are the *post-compression* pixel dimensions and exist so
/// the token estimator can price the image as `w*h/750` instead of measuring
/// the base64 string (which overestimates by ~50x).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAttachment {
    pub media_type: String,
    /// Standard-alphabet base64, padded, with no `data:` URI prefix.
    pub data: String,
    pub width: u32,
    pub height: u32,
}

impl ImageAttachment {
    /// Compress `bytes` and base64-encode the result.
    ///
    /// `ext` steers the output format the same way it does for user
    /// attachments: png/bmp become JPEG, everything else keeps its type.
    pub fn from_bytes(bytes: &[u8], media_type: &str, ext: &str) -> Self {
        let (out, media_type, width, height) = compress_image(bytes, media_type, ext);
        ImageAttachment {
            media_type,
            data: base64::engine::general_purpose::STANDARD.encode(&out),
            width,
            height,
        }
    }

    /// Wrap already-base64 data whose dimensions are unknown.
    ///
    /// Used for images that arrive pre-encoded (MCP tool results). The bytes
    /// are decoded once so they can be compressed and measured — an MCP server
    /// is free to hand back a 4000px PNG, and passing that through untouched
    /// would blow both the context and the token estimate.
    pub fn from_base64(data: &str, media_type: &str) -> Self {
        let ext = media_type.rsplit('/').next().unwrap_or("png");
        match base64::engine::general_purpose::STANDARD.decode(data) {
            Ok(bytes) => Self::from_bytes(&bytes, media_type, ext),
            // Undecodable payload: pass it through rather than dropping it, but
            // leave the dimensions at 0 so the estimator uses its conservative
            // fallback instead of pricing it at zero.
            Err(_) => ImageAttachment {
                media_type: media_type.to_string(),
                data: data.to_string(),
                width: 0,
                height: 0,
            },
        }
    }
}

/// Compress an image to reduce its token footprint before base64-encoding.
///
/// Rules (in order):
/// 1. Decode the image to get dimensions.
/// 2. If the longest edge exceeds 1568 px (the Anthropic server-side resize
///    threshold — beyond this the provider resizes server-side without quality
///    gain), resize down to 1568 px.
/// 3. Re-encode: JPEG at quality 80; convert large PNG/BMP to JPEG.
/// 4. On any error, fall back to the original bytes silently.
///
/// Returns (compressed_bytes, media_type, width, height).
pub fn compress_image(bytes: &[u8], media_type: &str, ext: &str) -> (Vec<u8>, String, u32, u32) {
    let img = match image::load_from_memory(bytes) {
        Ok(img) => img,
        Err(_) => return (bytes.to_vec(), media_type.to_string(), 0, 0),
    };
    let (w, h) = img.dimensions();
    let max_dim = MAX_IMAGE_DIM;
    let (new_w, new_h) = if w > max_dim || h > max_dim {
        let ratio = (w as f64).max(h as f64) / max_dim as f64;
        (
            (w as f64 / ratio).round() as u32,
            (h as f64 / ratio).round() as u32,
        )
    } else {
        (w, h)
    };
    let resized = if (new_w, new_h) != (w, h) {
        img.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let final_w = resized.width();
    let final_h = resized.height();
    let encode_as_jpeg = ext == "png" || ext == "bmp";
    let out_type = if encode_as_jpeg {
        "image/jpeg"
    } else {
        media_type
    };
    let mut out = Vec::new();
    let result = if out_type == "image/jpeg" {
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80);
        enc.encode(
            &resized.to_rgb8(),
            resized.width(),
            resized.height(),
            image::ColorType::Rgb8.into(),
        )
    } else if out_type == "image/webp" {
        let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
        enc.encode(
            &resized.to_rgba8(),
            resized.width(),
            resized.height(),
            image::ColorType::Rgba8.into(),
        )
    } else {
        resized.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
    };
    match result {
        Ok(_) => (out, out_type.to_string(), final_w, final_h),
        _ => (bytes.to_vec(), media_type.to_string(), w, h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a solid-colour PNG of the given size.
    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::DynamicImage::new_rgb8(w, h);
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn png_is_transcoded_to_jpeg_and_kept_when_small() {
        let (out, media_type, w, h) = compress_image(&png(64, 32), "image/png", "png");
        assert_eq!(media_type, "image/jpeg");
        assert_eq!((w, h), (64, 32));
        assert!(!out.is_empty());
    }

    #[test]
    fn oversized_image_is_capped_to_the_long_edge_preserving_aspect() {
        let (_, _, w, h) = compress_image(&png(3136, 1568), "image/png", "png");
        assert_eq!(w, MAX_IMAGE_DIM);
        assert_eq!(h, MAX_IMAGE_DIM / 2);
    }

    #[test]
    fn undecodable_bytes_pass_through_with_zero_dimensions() {
        let (out, media_type, w, h) = compress_image(b"not an image", "image/png", "png");
        assert_eq!(out, b"not an image");
        assert_eq!(media_type, "image/png");
        assert_eq!((w, h), (0, 0));
    }

    #[test]
    fn from_base64_decodes_compresses_and_measures() {
        let data = base64::engine::general_purpose::STANDARD.encode(png(2000, 1000));
        let att = ImageAttachment::from_base64(&data, "image/png");
        assert_eq!(att.media_type, "image/jpeg");
        assert_eq!(att.width, MAX_IMAGE_DIM);
        assert_eq!(att.height, MAX_IMAGE_DIM / 2);
    }

    #[test]
    fn from_base64_keeps_undecodable_payload_but_flags_unknown_size() {
        let att = ImageAttachment::from_base64("!!!not base64!!!", "image/png");
        assert_eq!(att.data, "!!!not base64!!!");
        assert_eq!((att.width, att.height), (0, 0));
    }
}
