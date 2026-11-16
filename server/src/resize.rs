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
///
/// libjpeg rounds scaled dimensions up, so the last column and row of a scaled
/// decode can be partly padding. The returned extent is the exact, possibly
/// fractional, size of the picture inside the decoded buffer.
fn decode_jpeg_scaled(
    data: &[u8],
    target_width: u32,
    target_height: u32,
) -> Result<(DynamicImage, Extent)> {
    decode_jpeg_with(data, |src_w, src_h| {
        Ok(choose_jpeg_scale(src_w, src_h, target_width, target_height))
    })
}

/// Prefix of a stored reduced source copy: magic, the original width and height
/// (little-endian u32), then the decode scale as numerator and denominator
/// (little-endian u16).
const PROXY_MAGIC: &[u8; 4] = b"SWP2";
pub(crate) const PROXY_HEADER_LEN: usize = 16;

/// What a stored reduced copy says about the original.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProxyInfo {
    pub orig_width: u32,
    pub orig_height: u32,
    pub width: u32,
    pub height: u32,
    scale: (u16, u16),
}

impl ProxyInfo {
    /// The extent of the original picture inside a decode of this copy, given the
    /// extent that decode reported for the copy's own JPEG.
    pub(crate) fn picture_extent(&self, decoded: Extent) -> Extent {
        let (num, denom) = (self.scale.0 as f64, self.scale.1 as f64);
        Extent {
            width: decoded.width * (self.orig_width as f64 * num / denom) / self.width as f64,
            height: decoded.height * (self.orig_height as f64 * num / denom) / self.height as f64,
        }
    }

    /// Whether an output of this size may be made from the copy. Outputs close to
    /// the copy's own size would show its compression, so they use the original.
    pub(crate) fn can_serve(&self, out_w: u32, out_h: u32, max_fill_percent: u8) -> bool {
        proxy_can_serve(self.width, self.height, out_w, out_h, max_fill_percent)
    }
}

pub(crate) fn proxy_can_serve(
    proxy_w: u32,
    proxy_h: u32,
    out_w: u32,
    out_h: u32,
    max_fill_percent: u8,
) -> bool {
    let limit = |side: u32| side as u64 * max_fill_percent.min(100) as u64;
    out_w as u64 * 100 <= limit(proxy_w) && out_h as u64 * 100 <= limit(proxy_h)
}

/// A reduced libjpeg scale from the header, or nothing if the pair is not one we write.
fn known_reduced_scale(num: u16, denom: u16) -> Option<turbojpeg::ScalingFactor> {
    JPEG_SCALE_FACTORS.iter().find_map(|&(n, d)| {
        if n < d && n == num as usize && d == denom as usize {
            Some(turbojpeg::ScalingFactor::new(n, d))
        } else {
            None
        }
    })
}

/// Parse a stored reduced copy. The JPEG starts at `PROXY_HEADER_LEN`.
/// A header whose JPEG is not exactly the claimed scale of the original is rejected,
/// so a mixed or truncated file cannot be decoded as if it were the picture.
pub(crate) fn read_proxy(data: &[u8]) -> Option<ProxyInfo> {
    if data.len() <= PROXY_HEADER_LEN || data[..4] != *PROXY_MAGIC {
        return None;
    }
    let orig_width = u32::from_le_bytes(data[4..8].try_into().ok()?);
    let orig_height = u32::from_le_bytes(data[8..12].try_into().ok()?);
    let scale_num = u16::from_le_bytes(data[12..14].try_into().ok()?);
    let scale_denom = u16::from_le_bytes(data[14..16].try_into().ok()?);
    let scale = known_reduced_scale(scale_num, scale_denom)?;
    let size = blob_size(&data[PROXY_HEADER_LEN..]).ok()?;
    let expect_w = scale.scale(orig_width as usize);
    let expect_h = scale.scale(orig_height as usize);
    if orig_width == 0
        || orig_height == 0
        || size.width == 0
        || size.height == 0
        || size.width != expect_w
        || size.height != expect_h
    {
        return None;
    }
    Some(ProxyInfo {
        orig_width,
        orig_height,
        width: size.width as u32,
        height: size.height as u32,
        scale: (scale_num, scale_denom),
    })
}

/// The smallest reduced decode scale whose long side is still at least `min_side`.
/// `None` when the source is too small for a copy to save anything, or when the
/// feature is off (`min_side == 0`).
fn proxy_scale(src_w: usize, src_h: usize, min_side: u32) -> Option<turbojpeg::ScalingFactor> {
    if min_side == 0 {
        return None;
    }
    for &(num, denom) in JPEG_SCALE_FACTORS {
        if num == denom {
            break;
        }
        let sf = turbojpeg::ScalingFactor::new(num, denom);
        if sf.scale(src_w).max(sf.scale(src_h)) >= min_side as usize {
            return Some(sf);
        }
    }
    None
}

/// Dimensions the reduced copy of this source would have. `None` for non-JPEG
/// data and for sources that are not large enough.
pub(crate) fn proxy_dimensions(
    data: &[u8],
    src_w: usize,
    src_h: usize,
    min_side: u32,
) -> Option<(u32, u32)> {
    if !is_jpeg(data) {
        return None;
    }
    let sf = proxy_scale(src_w, src_h, min_side)?;
    Some((sf.scale(src_w) as u32, sf.scale(src_h) as u32))
}

/// Decode a JPEG at its reduced-copy scale.
/// Refuses a source that is too small instead of decoding it at full size.
pub(crate) fn decode_jpeg_for_proxy(data: &[u8], min_side: u32) -> Result<(DynamicImage, Extent)> {
    decode_jpeg_with(data, |src_w, src_h| {
        proxy_scale(src_w, src_h, min_side).ok_or_else(|| {
            AppError::DecodeError("source is too small for a reduced copy".to_string())
        })
    })
}

/// Encode pixels for the source cache: a 16-byte header, then a 4:2:0 JPEG.
/// Quality outside 1–100 is clamped. The buffer must be exactly the scaled size
/// of the original; a mismatch is an error rather than a header that lies.
pub(crate) fn encode_proxy(
    rgb: &image::RgbImage,
    orig_width: u32,
    orig_height: u32,
    min_side: u32,
    quality: u8,
) -> Result<Vec<u8>> {
    let scale =
        proxy_scale(orig_width as usize, orig_height as usize, min_side).ok_or_else(|| {
            AppError::EncodeError("source is too small for a reduced copy".to_string())
        })?;
    let expect_w = scale.scale(orig_width as usize) as u32;
    let expect_h = scale.scale(orig_height as usize) as u32;
    if rgb.width() != expect_w || rgb.height() != expect_h {
        return Err(AppError::EncodeError(
            "reduced copy pixels do not match the original scale".to_string(),
        ));
    }
    let quality = quality.clamp(1, 100) as i32;
    let jpeg = turbojpeg::compress_image(rgb, quality, turbojpeg::Subsamp::Sub2x2)
        .map_err(|e| AppError::EncodeError(format!("turbojpeg proxy encode failed: {e}")))?;
    let mut out = Vec::with_capacity(PROXY_HEADER_LEN + jpeg.len());
    out.extend_from_slice(PROXY_MAGIC);
    out.extend_from_slice(&orig_width.to_le_bytes());
    out.extend_from_slice(&orig_height.to_le_bytes());
    out.extend_from_slice(&(scale.num() as u16).to_le_bytes());
    out.extend_from_slice(&(scale.denom() as u16).to_le_bytes());
    out.extend_from_slice(&jpeg);
    Ok(out)
}

fn decode_jpeg_with(
    data: &[u8],
    pick_scale: impl FnOnce(usize, usize) -> Result<turbojpeg::ScalingFactor>,
) -> Result<(DynamicImage, Extent)> {
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

        let scale = pick_scale(src_w, src_h)?;
        decompressor
            .set_scaling_factor(scale)
            .map_err(|e| AppError::DecodeError(format!("turbojpeg set scale failed: {}", e)))?;

        let scaled = header.scaled(scale);
        let width = scaled.width;
        let height = scaled.height;

        if scale != turbojpeg::ScalingFactor::ONE {
            tracing::debug!(
                "JPEG scaled decode: {}x{} -> {}x{} (scale {})",
                src_w,
                src_h,
                width,
                height,
                scale
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

        // Picture size in the decoded buffer. libjpeg's buffer is this value
        // rounded up, so the last column and row can be padding.
        let num = scale.num() as f64;
        let denom = scale.denom() as f64;
        let extent = Extent {
            width: src_w as f64 * num / denom,
            height: src_h as f64 * num / denom,
        };
        Ok((DynamicImage::ImageRgb8(rgb_image), extent))
    })
}

/// The part of a decoded buffer that holds picture, in decoded pixels from the
/// top-left corner. Equal to the buffer size except after a scaled JPEG decode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extent {
    pub width: f64,
    pub height: f64,
}

impl Extent {
    pub fn of(img: &DynamicImage) -> Self {
        let (w, h) = img.dimensions();
        Extent {
            width: w as f64,
            height: h as f64,
        }
    }

    fn clamped_to(self, width: u32, height: u32) -> Self {
        let width_px = width as f64;
        let height_px = height as f64;
        let w = if self.width.is_finite() {
            self.width
        } else {
            width_px
        };
        let h = if self.height.is_finite() {
            self.height
        } else {
            height_px
        };
        Extent {
            width: w.clamp(f64::MIN_POSITIVE, width_px),
            height: h.clamp(f64::MIN_POSITIVE, height_px),
        }
    }
}

/// Decode image bytes and report the exact extent of the picture inside the buffer.
///
/// Uses turbojpeg (libjpeg-turbo with SIMD) for JPEG, with scaled decoding when
/// the target is smaller than the source. Other formats use the `image` crate,
/// whose extent is the whole buffer. Pass both values to `resize_image_exact`
/// so a scaled JPEG is not stretched.
///
/// `target_width` and `target_height` are the desired output dimensions (0 = auto).
pub fn decode_image_exact(
    data: &[u8],
    target_width: u32,
    target_height: u32,
) -> Result<(DynamicImage, Extent)> {
    if is_jpeg(data) {
        return decode_jpeg_scaled(data, target_width, target_height);
    }
    // Fall back to image crate for PNG/GIF/WebP/BMP
    let img = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| AppError::DecodeError(format!("Failed to guess image format: {}", e)))?
        .decode()
        .map_err(|e| AppError::DecodeError(format!("Failed to decode image: {}", e)))?;
    let extent = Extent::of(&img);
    Ok((img, extent))
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

struct FastResize {
    src_width: u32,
    src_height: u32,
    dst_width: u32,
    dst_height: u32,
    filter: FilterType,
    pixel_type: PixelType,
    source_box: Option<(f64, f64, f64, f64)>,
}

/// Fast SIMD-accelerated resize using fast_image_resize crate.
/// Supports both U8x3 (RGB) and U8x4 (RGBA) pixel types.
fn fast_resize(src_data: &mut [u8], spec: FastResize) -> Result<Vec<u8>> {
    // Ensure dimensions are at least 1
    let src_w = spec.src_width.max(1);
    let src_h = spec.src_height.max(1);
    let dst_w = spec.dst_width.max(1);
    let dst_h = spec.dst_height.max(1);

    let src_image = FirImage::from_slice_u8(src_w, src_h, src_data, spec.pixel_type)
        .map_err(|e| AppError::DecodeError(format!("failed to view source pixels: {e}")))?;

    let mut dst_image = FirImage::new(dst_w, dst_h, spec.pixel_type);
    let mut options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(spec.filter));
    if let Some((left, top, width, height)) = spec.source_box {
        let (left, top, width, height) = crop_inside(src_w, src_h, left, top, width, height)?;
        options = options.crop(left, top, width, height);
    }

    RESIZER.with(|r| {
        r.borrow_mut()
            .resize(&src_image, &mut dst_image, &options)
            .map_err(|e| AppError::EncodeError(format!("resize failed: {e}")))
    })?;

    Ok(dst_image.into_vec())
}

/// Keep a source crop inside the decoded buffer. `fast_image_resize` rejects a box
/// that crosses the edge, including one that only does so by a rounding error.
fn crop_inside(
    src_w: u32,
    src_h: u32,
    left: f64,
    top: f64,
    width: f64,
    height: f64,
) -> Result<(f64, f64, f64, f64)> {
    if ![left, top, width, height].into_iter().all(f64::is_finite) {
        return Err(AppError::DecodeError(
            "source crop is not finite".to_string(),
        ));
    }
    let max_w = src_w as f64;
    let max_h = src_h as f64;
    let left = left.clamp(0.0, max_w);
    let top = top.clamp(0.0, max_h);
    let width = width.min(max_w - left).max(0.0);
    let height = height.min(max_h - top).max(0.0);
    if width <= 0.0 || height <= 0.0 {
        return Err(AppError::DecodeError("source crop is empty".to_string()));
    }
    Ok((left, top, width, height))
}

/// Resize `img`, sampling only `extent` of it. A scaled JPEG decode leaves
/// padding in the last column and row; sampling the exact extent keeps the
/// output registered with the original. RGB8 images stay in RGB so the
/// resize does not detour through RGBA.
pub fn resize_image_exact(
    img: DynamicImage,
    extent: Extent,
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
    let extent = extent.clamped_to(orig_width, orig_height);
    let exact = extent.width == orig_width as f64 && extent.height == orig_height as f64;

    // The handler has already resolved auto dimensions and applied the caps.
    let (final_width, final_height) = (target_width.max(1), target_height.max(1));

    // Skip resize if dimensions match
    if exact
        && final_width == orig_width
        && final_height == orig_height
        && fit_mode != FitMode::Cover
    {
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

    // The resizer samples the source box directly, so neither mode copies pixels first.
    let source_box = match fit_mode {
        FitMode::Contain | FitMode::Fill => {
            if exact {
                None
            } else {
                Some((0.0, 0.0, extent.width, extent.height))
            }
        }
        FitMode::Cover => Some(cover_box(extent, final_width, final_height)),
    };
    let dst_data = fast_resize(
        &mut src_data,
        FastResize {
            src_width: orig_width,
            src_height: orig_height,
            dst_width: final_width,
            dst_height: final_height,
            filter,
            pixel_type,
            source_box,
        },
    )?;
    make_image(dst_data, final_width, final_height)
}

/// Source box that has the target aspect and is centered in the picture.
fn cover_box(extent: Extent, target_width: u32, target_height: u32) -> (f64, f64, f64, f64) {
    let target_aspect = target_width as f64 / target_height.max(1) as f64;
    let source_aspect = extent.width / extent.height;
    if source_aspect > target_aspect {
        let width = (extent.height * target_aspect).clamp(f64::MIN_POSITIVE, extent.width);
        ((extent.width - width) / 2.0, 0.0, width, extent.height)
    } else {
        let height = (extent.width / target_aspect).clamp(f64::MIN_POSITIVE, extent.height);
        (0.0, (extent.height - height) / 2.0, extent.width, height)
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
        let extent = Extent::of(&img);
        let out = resize_image_exact(img, extent, 300, 300, FitMode::Cover).expect("cover resize");
        assert_eq!((out.width(), out.height()), (300, 300));
        let px = out.get_pixel(150, 150);
        for (got, expect) in [px[0], px[1], px[2]].into_iter().zip([200u8, 100, 50]) {
            assert!(
                (got as i16 - expect as i16).abs() <= 8,
                "center channel {got} vs {expect}"
            );
        }
    }

    /// A vertical edge at a known place in an odd-width source must land where the
    /// scale says it should, whichever reduced decode scale is used.
    #[test]
    fn scaled_decode_keeps_an_edge_registered() {
        let (src_w, src_h) = (1001u32, 601u32);
        let edge = 600u32;
        let rgb: image::RgbImage = ImageBuffer::from_fn(src_w, src_h, |x, _| {
            if x < edge {
                Rgb([20, 20, 20])
            } else {
                Rgb([230, 230, 230])
            }
        });
        let jpeg = turbojpeg::compress_image(&rgb, 95, turbojpeg::Subsamp::Sub2x2)
            .expect("encode fixture")
            .to_vec();

        let (out_w, out_h) = (500u32, 300u32);
        let (img, extent) = decode_image_exact(&jpeg, out_w, out_h).expect("decode");
        assert_eq!((img.width(), img.height()), (501, 301));
        assert_eq!(
            extent,
            Extent {
                width: 500.5,
                height: 300.5
            }
        );

        let out = resize_image_exact(img, extent, out_w, out_h, FitMode::Fill).expect("resize");
        assert_eq!((out.width(), out.height()), (out_w, out_h));
        let row = out.to_rgb8();
        // Sub-pixel edge position from the luminance crossing on the middle row.
        let level = |x: u32| row.get_pixel(x, out_h / 2)[0] as f64;
        let x1 = (0..out_w)
            .find(|&x| level(x) >= 125.0)
            .expect("edge present");
        let x0 = x1 - 1;
        let crossing = x0 as f64 + (125.0 - level(x0)) / (level(x1) - level(x0)) + 0.5;
        let expected = edge as f64 * out_w as f64 / src_w as f64;
        assert!(
            (crossing - expected).abs() < 0.1,
            "edge at {crossing:.3}, expected {expected:.3}"
        );
    }

    #[test]
    fn cover_box_is_centered_in_the_extent() {
        let wide = cover_box(
            Extent {
                width: 600.5,
                height: 400.0,
            },
            300,
            300,
        );
        assert_eq!(wide, (100.25, 0.0, 400.0, 400.0));
        let tall = cover_box(
            Extent {
                width: 400.0,
                height: 600.5,
            },
            300,
            300,
        );
        assert_eq!(tall, (0.0, 100.25, 400.0, 400.0));
    }

    fn jpeg_fixture(w: u32, h: u32) -> Vec<u8> {
        let rgb: image::RgbImage =
            ImageBuffer::from_fn(w, h, |x, y| Rgb([(x % 251) as u8, (y % 241) as u8, 90]));
        turbojpeg::compress_image(&rgb, 90, turbojpeg::Subsamp::Sub2x2)
            .expect("encode fixture")
            .to_vec()
    }

    #[test]
    fn proxy_round_trip_keeps_the_original_geometry() {
        let (src_w, src_h) = (1001u32, 601u32);
        let jpeg = jpeg_fixture(src_w, src_h);
        assert_eq!(
            proxy_dimensions(&jpeg, src_w as usize, src_h as usize, 400),
            Some((501, 301))
        );
        let (img, extent) = decode_jpeg_for_proxy(&jpeg, 400).expect("proxy decode");
        assert_eq!(
            extent,
            Extent {
                width: 500.5,
                height: 300.5
            }
        );

        let stored =
            encode_proxy(img.as_rgb8().expect("rgb"), src_w, src_h, 400, 90).expect("proxy encode");
        let info = read_proxy(&stored).expect("proxy header");
        assert_eq!((info.orig_width, info.orig_height), (src_w, src_h));
        assert_eq!((info.width, info.height), (501, 301));

        // A half-scale decode of the copy holds a quarter-scale picture of the original.
        let (half, half_extent) =
            decode_image_exact(&stored[PROXY_HEADER_LEN..], 200, 120).expect("decode proxy");
        assert_eq!((half.width(), half.height()), (251, 151));
        let picture = info.picture_extent(half_extent);
        assert!((picture.width - src_w as f64 / 4.0).abs() < 1e-9);
        assert!((picture.height - src_h as f64 / 4.0).abs() < 1e-9);
    }

    #[test]
    fn proxy_serves_only_outputs_inside_the_fill_limit() {
        assert!(proxy_can_serve(501, 301, 200, 120, 85));
        assert!(proxy_can_serve(501, 301, 425, 255, 85));
        assert!(!proxy_can_serve(501, 301, 426, 255, 85));
        assert!(proxy_can_serve(501, 301, 501, 301, 100));
        assert!(!proxy_can_serve(501, 301, 1, 1, 0));
    }

    #[test]
    fn stale_or_mismatched_proxy_data_is_rejected() {
        assert!(read_proxy(b"SWP1\x00\x00\x00\x00").is_none());
        assert!(read_proxy(&[0xFF, 0xD8, 0xFF, 0xE0]).is_none());

        let jpeg = jpeg_fixture(64, 48);
        let (img, _) = decode_jpeg_for_proxy(&jpeg, 16).expect("small proxy decode");
        let mut stored =
            encode_proxy(img.as_rgb8().expect("rgb"), 64, 48, 16, 0).expect("quality 0 is clamped");
        assert!(read_proxy(&stored).is_some());

        // Claim 1/8 while the JPEG is the 1/4 buffer. The sizes disagree.
        stored[12] = 1;
        stored[13] = 0;
        stored[14] = 8;
        stored[15] = 0;
        assert!(read_proxy(&stored).is_none());
    }

    #[test]
    fn non_jpeg_and_small_sources_have_no_proxy() {
        let png = {
            let rgb: image::RgbImage = ImageBuffer::from_fn(64, 48, |_, _| Rgb([1, 2, 3]));
            let mut buf = Cursor::new(Vec::new());
            DynamicImage::ImageRgb8(rgb)
                .write_to(&mut buf, ImageFormat::Png)
                .expect("png");
            buf.into_inner()
        };
        assert!(proxy_dimensions(&png, 64, 48, 16).is_none());
        let jpeg = jpeg_fixture(32, 24);
        assert!(proxy_dimensions(&jpeg, 32, 24, 10_000).is_none());
        assert!(decode_jpeg_for_proxy(&jpeg, 10_000).is_err());
    }
}
