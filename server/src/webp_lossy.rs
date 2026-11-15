//! Lossy WebP encoding via libwebp.
//!
//! The `image` crate's WebP encoder is lossless only. This calls
//! `WebPEncodeRGBA` directly and copies the result into an owned buffer.

use image::DynamicImage;

/// Encode an image to lossy WebP. `quality` is 0-100.
///
/// Returns an error when libwebp reports a zero-length result.
pub fn encode_webp_lossy(img: &DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let rgba = img.to_rgba8();
    let width = rgba.width() as i32;
    let height = rgba.height() as i32;
    let stride = width.saturating_mul(4);
    let mut out: *mut u8 = std::ptr::null_mut();

    // SAFETY: `rgba` is a contiguous width*height*4 buffer that outlives the
    // call. libwebp allocates `out` and the caller frees it with `WebPFree`.
    let size = unsafe {
        libwebp_sys::WebPEncodeRGBA(
            rgba.as_ptr(),
            width,
            height,
            stride,
            quality as f32,
            &mut out,
        )
    };
    if size == 0 || out.is_null() {
        return Err("webp encode failed".to_string());
    }
    // SAFETY: `out` points at `size` bytes allocated by libwebp.
    let bytes = unsafe { std::slice::from_raw_parts(out, size).to_vec() };
    // SAFETY: `out` was allocated by libwebp and is not used again.
    unsafe { libwebp_sys::WebPFree(out.cast()) };
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    fn solid(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgba8(ImageBuffer::from_fn(w, h, |_, _| Rgba([20, 80, 140, 255])))
    }

    #[test]
    fn test_encode_webp_lossy_basic() {
        let result = encode_webp_lossy(&solid(10, 10), 80);
        assert!(result.is_ok());
        let bytes = result.unwrap();
        assert!(bytes.len() > 12);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WEBP");
    }

    #[test]
    fn test_encode_webp_quality_affects_size() {
        let img = DynamicImage::ImageRgba8(ImageBuffer::from_fn(100, 100, |x, y| {
            Rgba([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255])
        }));
        let low_quality = encode_webp_lossy(&img, 10).unwrap();
        let high_quality = encode_webp_lossy(&img, 95).unwrap();
        assert!(low_quality.len() < high_quality.len());
    }

    #[test]
    fn encodes_tiny_and_wide_images() {
        for (w, h) in [(1, 1), (4096, 1)] {
            let bytes =
                encode_webp_lossy(&solid(w, h), 75).unwrap_or_else(|e| panic!("{w}x{h}: {e}"));
            assert_eq!(&bytes[0..4], b"RIFF");
            assert_eq!(&bytes[8..12], b"WEBP");
        }
    }
}
