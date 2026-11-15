use crate::error::{AppError, Result};
use crate::util::format_bytes;
use fast_image_resize::{
    images::Image as FirImage, FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer,
};
use image::{DynamicImage, GenericImageView, ImageBuffer, ImageFormat, ImageReader, Rgb, Rgba};
use imagesize::blob_size;
use std::cell::RefCell;
use std::io::Cursor;

/// Threshold for using faster CatmullRom filter vs higher quality Lanczos3
/// Images with both dimensions <= this value use CatmullRom (faster)
const FAST_RESIZE_THRESHOLD: u32 = 300;

/// Threshold for using fastest Bilinear filter for tiny thumbnails
/// Images with both dimensions <= this value use Bilinear (fastest)
const TINY_RESIZE_THRESHOLD: u32 = 201;

thread_local! {
    static RESIZER: RefCell<Resizer> = RefCell::new(Resizer::new());
    static DECOMPRESSOR: RefCell<Option<turbojpeg::Decompressor>> = const { RefCell::new(None) };
}

/// Fit mode for image resizing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitMode {
    /// Scale to fit within bounds, preserve aspect ratio
    Contain,
    /// Scale to cover bounds, crop excess (center crop)
    Cover,
    /// Stretch to exact dimensions
    Fill,
}

impl FitMode {
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "contain" => Ok(FitMode::Contain),
            "cover" => Ok(FitMode::Cover),
            "fill" => Ok(FitMode::Fill),
            _ => Err(AppError::InvalidFitMode(s.to_string())),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FitMode::Contain => "contain",
            FitMode::Cover => "cover",
            FitMode::Fill => "fill",
        }
    }
}

/// Output format for image encoding
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Jpeg,
    Png,
    WebP,
    Gif,
}

impl OutputFormat {
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "jpg" | "jpeg" => Ok(OutputFormat::Jpeg),
            "png" => Ok(OutputFormat::Png),
            "webp" => Ok(OutputFormat::WebP),
            "gif" => Ok(OutputFormat::Gif),
            _ => Err(AppError::InvalidFormat(s.to_string())),
        }
    }

    pub fn to_extension(self) -> &'static str {
        match self {
            OutputFormat::Jpeg => "jpg",
            OutputFormat::Png => "png",
            OutputFormat::WebP => "webp",
            OutputFormat::Gif => "gif",
        }
    }
}

/// Check image dimensions before full decode to prevent image bomb attacks
pub fn check_image_dimensions(data: &[u8], max_pixels: u64) -> Result<(usize, usize)> {
    let size = blob_size(data)
        .map_err(|e| AppError::DecodeError(format!("Failed to read image dimensions: {}", e)))?;

    if size.width == 0 || size.height == 0 {
        return Err(AppError::DecodeError(
            "source image has a zero dimension".to_string(),
        ));
    }

    let pixels = (size.width as u64) * (size.height as u64);
    if pixels > max_pixels {
        return Err(AppError::ImageDimensionsTooLarge {
            width: size.width,
            height: size.height,
            pixels,
            max_pixels,
        });
    }

    Ok((size.width, size.height))
}

const MAX_REDIRECTS: u8 = 5;

fn map_send_error(url: &str, err: reqwest::Error) -> AppError {
    if crate::net::is_forbidden_address(&err) {
        let host = url::Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(|host| host.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        AppError::SourceNotAllowed(host)
    } else {
        AppError::FetchError {
            message: format!("Request failed: {err}"),
            status: None,
        }
    }
}

/// Fetch an image from a remote URL, re-checking every redirect against `validate`.
///
/// `max_bytes` caps the decompressed body. A declared `Content-Length` above the
/// cap is rejected before the body is read.
pub async fn fetch_image(
    client: &reqwest::Client,
    url: &str,
    enable_compression: bool,
    validate: &(dyn Fn(&str) -> bool + Sync),
    max_bytes: u64,
) -> Result<(Vec<u8>, Option<String>)> {
    let mut current = url.to_string();
    let mut redirects = 0u8;

    loop {
        tracing::info!("Fetching image: {}", current);

        let mut request = client.get(&current);
        if enable_compression {
            request = request.header("Accept-Encoding", "gzip, br");
        }

        let mut response = request
            .send()
            .await
            .map_err(|err| map_send_error(&current, err))?;
        let status = response.status();

        if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
            redirects += 1;
            if redirects > MAX_REDIRECTS {
                return Err(AppError::FetchError {
                    message: "too many redirects".to_string(),
                    status: None,
                });
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| AppError::FetchError {
                    message: "redirect missing location".to_string(),
                    status: None,
                })?
                .to_string();
            let base = url::Url::parse(&current).map_err(|_| AppError::FetchError {
                message: "invalid url".to_string(),
                status: None,
            })?;
            let next = base.join(&location).map_err(|_| AppError::FetchError {
                message: "invalid redirect target".to_string(),
                status: None,
            })?;
            let next_url = next.as_str().to_string();
            if !validate(&next_url) {
                let host = next.host_str().unwrap_or("unknown").to_string();
                return Err(AppError::SourceNotAllowed(host));
            }
            current = next_url;
            continue;
        }

        if !status.is_success() {
            return Err(AppError::FetchError {
                message: format!("HTTP {status} from upstream"),
                status: Some(status.as_u16()),
            });
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        if response.content_length().is_some_and(|len| len > max_bytes) {
            return Err(AppError::SourceTooLarge(max_bytes));
        }

        let mut bytes =
            Vec::with_capacity(response.content_length().unwrap_or(0).min(max_bytes) as usize);
        while let Some(chunk) = response.chunk().await.map_err(|e| AppError::FetchError {
            message: format!("Failed to read response body: {e}"),
            status: None,
        })? {
            if bytes.len() as u64 + chunk.len() as u64 > max_bytes {
                return Err(AppError::SourceTooLarge(max_bytes));
            }
            bytes.extend_from_slice(&chunk);
        }

        let size = bytes.len();
        tracing::info!(
            url = %current,
            size_bytes = size,
            size_human = %format_bytes(size),
            "Downloaded image"
        );

        return Ok((bytes, content_type));
    }
}

/// Check if the data starts with JPEG magic bytes (SOI marker)
fn is_jpeg(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8
}

/// JPEG decode scale factors from smallest to largest (for choosing optimal decode resolution).
/// libjpeg-turbo supports all multiples of 1/8; we list the useful downscale factors.
const JPEG_SCALE_FACTORS: &[(usize, usize)] = &[
    (1, 8),
    (1, 4),
    (3, 8),
    (1, 2),
    (5, 8),
    (3, 4),
    (7, 8),
    (1, 1),
];

/// Choose the smallest JPEG decode scale factor where the decoded image is still
/// at least as large as the target dimensions in both axes.
/// Returns ScalingFactor::ONE if no downscaling is possible.
fn choose_jpeg_scale(
    src_width: usize,
    src_height: usize,
    target_width: u32,
    target_height: u32,
) -> turbojpeg::ScalingFactor {
    let tw = target_width as usize;
    let th = target_height as usize;

    // If both targets are 0 (no resize), decode at full resolution
    if tw == 0 && th == 0 {
        return turbojpeg::ScalingFactor::ONE;
    }

    for &(num, denom) in JPEG_SCALE_FACTORS {
        let sf = turbojpeg::ScalingFactor::new(num, denom);
        let scaled_w = sf.scale(src_width);
        let scaled_h = sf.scale(src_height);

        let w_ok = tw == 0 || scaled_w >= tw;
        let h_ok = th == 0 || scaled_h >= th;

        if w_ok && h_ok {
            return sf;
        }
    }

    turbojpeg::ScalingFactor::ONE
}

/// Decode a JPEG using turbojpeg with optional scaled decoding.
/// When the target is much smaller than the source, libjpeg-turbo decodes at a
/// reduced resolution in the DCT domain — dramatically faster and lower memory.
fn decode_jpeg_scaled(data: &[u8], target_width: u32, target_height: u32) -> Result<DynamicImage> {
    DECOMPRESSOR.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(
                turbojpeg::Decompressor::new()
                    .map_err(|e| AppError::DecodeError(format!("turbojpeg init failed: {e}")))?,
            );
        }
        let decompressor = slot
            .as_mut()
            .ok_or_else(|| AppError::DecodeError("turbojpeg init failed".to_string()))?;

        // Read JPEG header to get source dimensions
        let header = decompressor
            .read_header(data)
            .map_err(|e| AppError::DecodeError(format!("turbojpeg header read failed: {}", e)))?;

        let src_w = header.width;
        let src_h = header.height;

        // Choose optimal scale factor
        let scale = choose_jpeg_scale(src_w, src_h, target_width, target_height);
        decompressor
            .set_scaling_factor(scale)
            .map_err(|e| AppError::DecodeError(format!("turbojpeg set scale failed: {}", e)))?;

        let scaled = header.scaled(scale);
        let width = scaled.width;
        let height = scaled.height;

        if scale != turbojpeg::ScalingFactor::ONE {
            tracing::debug!(
                "JPEG scaled decode: {}x{} -> {}x{} (scale {}) for target {}x{}",
                src_w,
                src_h,
                width,
                height,
                scale,
                target_width,
                target_height
            );
        }

        // Allocate output buffer (RGB, 3 bytes per pixel)
        let pitch = width * 3;
        let mut pixels = vec![0u8; pitch * height];

        let output = turbojpeg::Image {
            pixels: &mut pixels[..],
            width,
            pitch,
            height,
            format: turbojpeg::PixelFormat::RGB,
        };

        decompressor
            .decompress(data, output)
            .map_err(|e| AppError::DecodeError(format!("turbojpeg scaled decode failed: {}", e)))?;

        // Construct RgbImage from the decoded pixels
        let rgb_image: image::RgbImage = ImageBuffer::from_raw(width as u32, height as u32, pixels)
            .ok_or_else(|| AppError::DecodeError("failed to build decoded image".to_string()))?;

        Ok(DynamicImage::ImageRgb8(rgb_image))
    })
}

/// Decode image bytes into a DynamicImage.
/// Uses turbojpeg (libjpeg-turbo with SIMD) for JPEG with scaled decoding
/// when the target is smaller than the source. Falls back to image crate for other formats.
///
/// `target_width` and `target_height` are the desired output dimensions (0 = auto).
/// For JPEG, these are used to choose an optimal decode scale factor.
pub fn decode_image(data: &[u8], target_width: u32, target_height: u32) -> Result<DynamicImage> {
    if is_jpeg(data) {
        return decode_jpeg_scaled(data, target_width, target_height);
    }
    // Fall back to image crate for PNG/GIF/WebP/BMP
    ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| AppError::DecodeError(format!("Failed to guess image format: {}", e)))?
        .decode()
        .map_err(|e| AppError::DecodeError(format!("Failed to decode image: {}", e)))
}

/// Choose the appropriate filter based on target dimensions
/// Uses Bilinear (fastest) for tiny thumbnails, CatmullRom (faster) for small images,
/// and Lanczos3 (highest quality) for larger ones
fn choose_filter(width: u32, height: u32, _src_width: u32, _src_height: u32) -> FilterType {
    if width <= TINY_RESIZE_THRESHOLD && height <= TINY_RESIZE_THRESHOLD {
        tracing::debug!(
            "Using Bilinear filter for tiny thumbnail ({}x{})",
            width,
            height
        );
        FilterType::Bilinear
    } else if width <= FAST_RESIZE_THRESHOLD && height <= FAST_RESIZE_THRESHOLD {
        tracing::debug!(
            "Using CatmullRom filter for small image ({}x{})",
            width,
            height
        );
        FilterType::CatmullRom
    } else {
        tracing::debug!(
            "Using Lanczos3 filter for large image ({}x{})",
            width,
            height
        );
        FilterType::Lanczos3
    }
}

/// Fast SIMD-accelerated resize using fast_image_resize crate
/// Supports both U8x3 (RGB) and U8x4 (RGBA) pixel types
fn fast_resize(
    src_data: &mut [u8],
    src_width: u32,
    src_height: u32,
    dst_width: u32,
    dst_height: u32,
    filter: FilterType,
    pixel_type: PixelType,
) -> Result<Vec<u8>> {
    // Ensure dimensions are at least 1
    let src_w = src_width.max(1);
    let src_h = src_height.max(1);
    let dst_w = dst_width.max(1);
    let dst_h = dst_height.max(1);

    let src_image = FirImage::from_slice_u8(src_w, src_h, src_data, pixel_type)
        .map_err(|e| AppError::DecodeError(format!("failed to view source pixels: {e}")))?;

    let mut dst_image = FirImage::new(dst_w, dst_h, pixel_type);
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(filter));

    RESIZER.with(|r| {
        r.borrow_mut()
            .resize(&src_image, &mut dst_image, &options)
            .map_err(|e| AppError::EncodeError(format!("resize failed: {e}")))
    })?;

    Ok(dst_image.into_vec())
}

fn bytes_per_pixel(pixel_type: PixelType) -> usize {
    match pixel_type {
        PixelType::U8x3 => 3,
        PixelType::U8x4 => 4,
        _ => 4,
    }
}

/// Copy a centered-or-offset rectangle out of a packed pixel buffer.
fn crop_raw(
    src: &[u8],
    src_width: u32,
    crop_w: u32,
    crop_h: u32,
    x: u32,
    y: u32,
    bpp: usize,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(crop_w as usize * crop_h as usize * bpp);
    let row_stride = src_width as usize * bpp;
    for row in y..(y + crop_h) {
        let start = row as usize * row_stride + x as usize * bpp;
        let end = start + crop_w as usize * bpp;
        out.extend_from_slice(&src[start..end]);
    }
    out
}

/// Resize an image according to the specified parameters
/// Uses SIMD-accelerated fast_image_resize for better performance.
/// For RGB8 images (e.g., JPEG via turbojpeg), resizes directly in RGB space
/// to avoid the wasteful RGB→RGBA→RGB round-trip conversion.
pub fn resize_image(
    img: DynamicImage,
    target_width: u32,
    target_height: u32,
    fit_mode: FitMode,
) -> Result<DynamicImage> {
    let (orig_width, orig_height) = img.dimensions();
    if orig_width == 0 || orig_height == 0 {
        return Err(AppError::DecodeError(
            "source image has a zero dimension".to_string(),
        ));
    }

    // The handler has already resolved auto dimensions and applied the caps.
    let (final_width, final_height) = (target_width.max(1), target_height.max(1));

    // Skip resize if dimensions match
    if final_width == orig_width && final_height == orig_height && fit_mode != FitMode::Cover {
        tracing::debug!("No resize needed, dimensions match");
        return Ok(img);
    }

    tracing::debug!(
        "Resizing {}x{} -> {}x{} (mode: {:?})",
        orig_width,
        orig_height,
        final_width,
        final_height,
        fit_mode
    );

    // Choose filter based on output size
    let filter = choose_filter(final_width, final_height, orig_width, orig_height);

    // Use RGB fast path for RGB8 images (e.g., JPEG decoded via turbojpeg)
    // to avoid wasteful RGB→RGBA→RGB round-trip conversion and resize on
    // a 25% smaller buffer (3 bytes/pixel vs 4)
    let (mut src_data, pixel_type, is_rgb) = match img {
        DynamicImage::ImageRgb8(rgb) => {
            tracing::debug!("Using RGB fast path (U8x3) - skipping RGBA conversion");
            (rgb.into_raw(), PixelType::U8x3, true)
        }
        other => {
            let rgba = other.to_rgba8();
            (rgba.into_raw(), PixelType::U8x4, false)
        }
    };

    let make_image = |data: Vec<u8>, w: u32, h: u32| -> Result<DynamicImage> {
        if is_rgb {
            let buffer: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_raw(w, h, data)
                .ok_or_else(|| AppError::EncodeError("failed to build RGB output".to_string()))?;
            Ok(DynamicImage::ImageRgb8(buffer))
        } else {
            let buffer: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_raw(w, h, data)
                .ok_or_else(|| AppError::EncodeError("failed to build RGBA output".to_string()))?;
            Ok(DynamicImage::ImageRgba8(buffer))
        }
    };

    match fit_mode {
        FitMode::Contain | FitMode::Fill => {
            let dst_data = fast_resize(
                &mut src_data,
                orig_width,
                orig_height,
                final_width,
                final_height,
                filter,
                pixel_type,
            )?;
            make_image(dst_data, final_width, final_height)
        }
        FitMode::Cover => {
            // Crop the source to the target aspect, then resize straight to the output.
            // The intermediate buffer is never larger than the output.
            let bpp = bytes_per_pixel(pixel_type);
            let (crop_w, crop_h, x, y) =
                cover_crop(orig_width, orig_height, final_width, final_height);
            let mut cropped = if crop_w == orig_width && crop_h == orig_height {
                src_data
            } else {
                crop_raw(&src_data, orig_width, crop_w, crop_h, x, y, bpp)
            };
            let dst_data = fast_resize(
                &mut cropped,
                crop_w,
                crop_h,
                final_width,
                final_height,
                filter,
                pixel_type,
            )?;
            make_image(dst_data, final_width, final_height)
        }
    }
}

/// Source rectangle that has the target aspect and is centered in the source.
fn cover_crop(
    orig_width: u32,
    orig_height: u32,
    target_width: u32,
    target_height: u32,
) -> (u32, u32, u32, u32) {
    let target_aspect = target_width as f64 / target_height.max(1) as f64;
    let source_aspect = orig_width as f64 / orig_height.max(1) as f64;
    if source_aspect > target_aspect {
        let crop_w = ((orig_height as f64) * target_aspect).round() as u32;
        let crop_w = crop_w.clamp(1, orig_width);
        let x = orig_width.saturating_sub(crop_w) / 2;
        (crop_w, orig_height, x, 0)
    } else {
        let crop_h = ((orig_width as f64) / target_aspect).round() as u32;
        let crop_h = crop_h.clamp(1, orig_height);
        let y = orig_height.saturating_sub(crop_h) / 2;
        (orig_width, crop_h, 0, y)
    }
}

/// Calculate final dimensions based on fit mode
pub(crate) fn calculate_dimensions(
    orig_width: u32,
    orig_height: u32,
    target_width: u32,
    target_height: u32,
    fit_mode: FitMode,
) -> (u32, u32) {
    // Handle auto dimensions (0 means auto-calculate to preserve aspect ratio)
    match (target_width, target_height) {
        (0, 0) => (orig_width, orig_height),
        (0, h) => {
            let scale = h as f64 / orig_height as f64;
            ((orig_width as f64 * scale).round() as u32, h)
        }
        (w, 0) => {
            let scale = w as f64 / orig_width as f64;
            (w, (orig_height as f64 * scale).round() as u32)
        }
        (w, h) => {
            if fit_mode == FitMode::Contain {
                // Scale to fit within bounds
                let scale = f64::min(w as f64 / orig_width as f64, h as f64 / orig_height as f64);
                (
                    (orig_width as f64 * scale).round() as u32,
                    (orig_height as f64 * scale).round() as u32,
                )
            } else {
                (w, h)
            }
        }
    }
}

/// Resolve auto dimensions, then scale the result down so it fits in the effective caps.
pub(crate) fn bound_output_dimensions(
    orig_width: u32,
    orig_height: u32,
    target_width: u32,
    target_height: u32,
    fit_mode: FitMode,
    max_width: u32,
    max_height: u32,
) -> (u32, u32) {
    let (mut w, mut h) = calculate_dimensions(
        orig_width,
        orig_height,
        target_width,
        target_height,
        fit_mode,
    );
    if w > max_width || h > max_height {
        let scale = f64::min(
            max_width as f64 / w.max(1) as f64,
            max_height as f64 / h.max(1) as f64,
        );
        w = ((w as f64) * scale).round() as u32;
        h = ((h as f64) * scale).round() as u32;
    }
    (w.clamp(1, max_width.max(1)), h.clamp(1, max_height.max(1)))
}

/// Encode an image to bytes in the specified format with quality control.
///
/// Takes ownership of the image to enable zero-copy encoding when the
/// pixel format already matches (e.g., RGB8 images skip the clone in to_rgb8).
///
/// Quality (1-100) is used for:
/// - JPEG: Lossy compression quality
/// - WebP: Lossy compression quality (via libwebp)
///
/// For PNG/GIF/BMP: Quality is ignored (lossless/palette-based formats)
pub fn encode_image(img: DynamicImage, extension: &str, quality: u8) -> Result<Vec<u8>> {
    let mut buffer = Cursor::new(Vec::new());

    match extension {
        "jpg" | "jpeg" => {
            // into_rgb8() is zero-copy when img is already ImageRgb8 (JPEG fast path)
            let rgb = img.into_rgb8();
            let jpeg_data =
                turbojpeg::compress_image(&rgb, quality as i32, turbojpeg::Subsamp::Sub2x2)
                    .map_err(|e| {
                        AppError::EncodeError(format!("turbojpeg encode failed: {}", e))
                    })?;
            return Ok(jpeg_data.to_vec());
        }
        "webp" => {
            // Use libwebp for fast lossy encoding with quality control
            let webp_bytes = crate::webp_lossy::encode_webp_lossy(&img, quality)
                .map_err(|e| AppError::EncodeError(format!("Failed to encode WebP: {}", e)))?;
            return Ok(webp_bytes);
        }
        "png" => {
            img.write_to(&mut buffer, ImageFormat::Png)
                .map_err(|e| AppError::EncodeError(format!("Failed to encode PNG: {}", e)))?;
        }
        "gif" => {
            img.write_to(&mut buffer, ImageFormat::Gif)
                .map_err(|e| AppError::EncodeError(format!("Failed to encode GIF: {}", e)))?;
        }
        "bmp" => {
            img.write_to(&mut buffer, ImageFormat::Bmp)
                .map_err(|e| AppError::EncodeError(format!("Failed to encode BMP: {}", e)))?;
        }
        _ => {
            // Default to JPEG via turbojpeg — zero-copy for RGB8
            let rgb = img.into_rgb8();
            let jpeg_data =
                turbojpeg::compress_image(&rgb, quality as i32, turbojpeg::Subsamp::Sub2x2)
                    .map_err(|e| {
                        AppError::EncodeError(format!("turbojpeg encode failed: {}", e))
                    })?;
            return Ok(jpeg_data.to_vec());
        }
    }

    Ok(buffer.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    #[test]
    fn cover_keeps_the_center_of_a_four_quadrant_image() {
        let img = DynamicImage::ImageRgb8(ImageBuffer::from_fn(600, 400, |x, y| {
            let quadrant = if x < 300 {
                if y < 200 {
                    Rgb([255, 0, 0])
                } else {
                    Rgb([0, 255, 0])
                }
            } else if y < 200 {
                Rgb([0, 0, 255])
            } else {
                Rgb([255, 255, 0])
            };
            if (280..320).contains(&x) && (180..220).contains(&y) {
                Rgb([200, 100, 50])
            } else {
                quadrant
            }
        }));
        let out = resize_image(img, 300, 300, FitMode::Cover).expect("cover resize");
        assert_eq!((out.width(), out.height()), (300, 300));
        let px = out.get_pixel(150, 150);
        for (got, expect) in [px[0], px[1], px[2]].into_iter().zip([200u8, 100, 50]) {
            assert!(
                (got as i16 - expect as i16).abs() <= 8,
                "center channel {got} vs {expect}"
            );
        }
    }
}
