//! Performance benchmarks for image processing operations
//!
//! Run with: cargo bench
//!
//! This benchmarks:
//! - JPG to JPG resizing at various quality levels (60, 80, 100)
//! - JPG to WebP conversion at various quality levels (60, 80, 100)
//! - Cache read/write operations

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use image::{DynamicImage, GenericImageView, ImageReader};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use tempfile::TempDir;

// We need to inline the relevant functions since bench can't easily import from lib
// (This is a common pattern for Rust benchmarks)

/// Decode image bytes into a DynamicImage
fn decode_image(data: &[u8]) -> DynamicImage {
    ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .expect("Failed to guess format")
        .decode()
        .expect("Failed to decode image")
}

/// Resize an image to thumbnail size (contain mode) - original image crate
fn resize_to_thumbnail(img: &DynamicImage, width: u32, height: u32) -> DynamicImage {
    img.resize(width, height, image::imageops::FilterType::Lanczos3)
}

/// Fast SIMD-accelerated resize using fast_image_resize crate
fn resize_fast(
    img: &DynamicImage,
    width: u32,
    height: u32,
    filter: fast_image_resize::FilterType,
) -> DynamicImage {
    use fast_image_resize::{
        images::Image as FirImage, PixelType, ResizeAlg, ResizeOptions, Resizer,
    };
    use image::{ImageBuffer, Rgba};

    let (orig_width, orig_height) = img.dimensions();
    let rgba = img.to_rgba8();
    let src_data = rgba.as_raw();

    let src_image =
        FirImage::from_vec_u8(orig_width, orig_height, src_data.to_vec(), PixelType::U8x4)
            .expect("Failed to create source image");

    let mut dst_image = FirImage::new(width, height, PixelType::U8x4);

    let mut resizer = Resizer::new();
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(filter));

    resizer
        .resize(&src_image, &mut dst_image, &options)
        .expect("Failed to resize image");

    let dst_data = dst_image.into_vec();
    let buffer: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_raw(width, height, dst_data)
        .expect("Failed to create output image buffer");
    DynamicImage::ImageRgba8(buffer)
}

/// Encode image as JPEG with quality
fn encode_jpeg(img: &DynamicImage, quality: u8) -> Vec<u8> {
    use image::codecs::jpeg::JpegEncoder;
    use image::{ExtendedColorType, ImageEncoder};

    let mut buffer = Cursor::new(Vec::new());
    let encoder = JpegEncoder::new_with_quality(&mut buffer, quality);
    // JPEG doesn't support alpha channel, convert to RGB
    let rgb = img.to_rgb8();
    encoder
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .expect("Failed to encode JPEG");
    buffer.into_inner()
}

/// Encode image as WebP with quality (using libwebp)
fn encode_webp(img: &DynamicImage, quality: u8) -> Vec<u8> {
    let rgba = img.to_rgba8();
    let width = rgba.width() as i32;
    let height = rgba.height() as i32;
    let stride = width.saturating_mul(4);
    let mut out: *mut u8 = std::ptr::null_mut();
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
    assert!(size > 0 && !out.is_null(), "webp encode failed");
    let bytes = unsafe { std::slice::from_raw_parts(out, size).to_vec() };
    unsafe { libwebp_sys::WebPFree(out.cast()) };
    bytes
}

/// Get path to test fixtures
fn fixtures_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Load the large test photo
fn load_large_photo() -> Vec<u8> {
    let path = fixtures_path().join("large_photo.jpg");
    fs::read(&path).unwrap_or_else(|_| panic!("Failed to read test image at {:?}", path))
}

/// Load the medium test photo  
fn load_medium_photo() -> Vec<u8> {
    let path = fixtures_path().join("medium_photo.jpg");
    fs::read(&path).unwrap_or_else(|_| panic!("Failed to read test image at {:?}", path))
}

// =============================================================================
// BENCHMARK: Full pipeline (decode -> resize -> encode)
// =============================================================================

fn bench_full_pipeline_jpg_to_jpg(c: &mut Criterion) {
    let large_photo_bytes = load_large_photo();

    let mut group = c.benchmark_group("full_pipeline_jpg_to_jpg");
    group.throughput(Throughput::Bytes(large_photo_bytes.len() as u64));

    // Thumbnail size: 150x150
    let thumbnail_size = (150, 150);

    for quality in [60, 80, 100] {
        group.bench_with_input(
            BenchmarkId::new("large_to_thumbnail", format!("q{}", quality)),
            &quality,
            |b, &quality| {
                b.iter(|| {
                    let img = decode_image(&large_photo_bytes);
                    let resized = resize_to_thumbnail(&img, thumbnail_size.0, thumbnail_size.1);
                    let encoded = encode_jpeg(&resized, quality);
                    black_box(encoded)
                });
            },
        );
    }

    group.finish();
}

fn bench_full_pipeline_jpg_to_webp(c: &mut Criterion) {
    let large_photo_bytes = load_large_photo();

    let mut group = c.benchmark_group("full_pipeline_jpg_to_webp");
    group.throughput(Throughput::Bytes(large_photo_bytes.len() as u64));

    // Thumbnail size: 150x150
    let thumbnail_size = (150, 150);

    for quality in [60, 80, 100] {
        group.bench_with_input(
            BenchmarkId::new("large_to_thumbnail", format!("q{}", quality)),
            &quality,
            |b, &quality| {
                b.iter(|| {
                    let img = decode_image(&large_photo_bytes);
                    let resized = resize_to_thumbnail(&img, thumbnail_size.0, thumbnail_size.1);
                    let encoded = encode_webp(&resized, quality);
                    black_box(encoded)
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// BENCHMARK: Decode only
// =============================================================================

fn bench_decode(c: &mut Criterion) {
    let large_photo_bytes = load_large_photo();
    let medium_photo_bytes = load_medium_photo();

    let mut group = c.benchmark_group("decode");

    group.throughput(Throughput::Bytes(large_photo_bytes.len() as u64));
    group.bench_function("large_photo", |b| {
        b.iter(|| {
            let img = decode_image(&large_photo_bytes);
            black_box(img)
        });
    });

    group.throughput(Throughput::Bytes(medium_photo_bytes.len() as u64));
    group.bench_function("medium_photo", |b| {
        b.iter(|| {
            let img = decode_image(&medium_photo_bytes);
            black_box(img)
        });
    });

    group.finish();
}

// =============================================================================
// BENCHMARK: Resize only
// =============================================================================

fn bench_resize(c: &mut Criterion) {
    let large_photo_bytes = load_large_photo();
    let img = decode_image(&large_photo_bytes);

    let mut group = c.benchmark_group("resize");

    // Different target sizes
    let sizes = [
        ("thumbnail_150", 150, 150),
        ("small_300", 300, 300),
        ("medium_600", 600, 600),
        ("large_1200", 1200, 1200),
    ];

    for (name, width, height) in sizes {
        group.bench_function(name, |b| {
            b.iter(|| {
                let resized = resize_to_thumbnail(&img, width, height);
                black_box(resized)
            });
        });
    }

    group.finish();
}

// =============================================================================
// BENCHMARK: Compare resize filter algorithms
// =============================================================================

fn bench_resize_filters(c: &mut Criterion) {
    use image::imageops::FilterType;

    let large_photo_bytes = load_large_photo();
    let img = decode_image(&large_photo_bytes);

    let mut group = c.benchmark_group("resize_filters");

    // Test different filter algorithms for 150x150 thumbnail
    let filters = [
        ("nearest", FilterType::Nearest),
        ("triangle", FilterType::Triangle),      // Bilinear
        ("catmull_rom", FilterType::CatmullRom), // Bicubic
        ("gaussian", FilterType::Gaussian),
        ("lanczos3", FilterType::Lanczos3), // Current default
    ];

    for (name, filter) in filters {
        group.bench_function(name, |b| {
            b.iter(|| {
                let resized = img.resize(150, 150, filter);
                black_box(resized)
            });
        });
    }

    group.finish();
}

// =============================================================================
// BENCHMARK: fast_image_resize vs image crate
// =============================================================================

fn bench_fast_image_resize(c: &mut Criterion) {
    use fast_image_resize::FilterType as FirFilter;
    use image::imageops::FilterType as ImgFilter;

    let large_photo_bytes = load_large_photo();
    let img = decode_image(&large_photo_bytes);

    let mut group = c.benchmark_group("fast_vs_image_crate");

    // Compare same filter (CatmullRom) between crates for thumbnail
    group.bench_function("image_crate_catmullrom_150", |b| {
        b.iter(|| {
            let resized = img.resize(150, 150, ImgFilter::CatmullRom);
            black_box(resized)
        });
    });

    group.bench_function("fast_image_resize_catmullrom_150", |b| {
        b.iter(|| {
            let resized = resize_fast(&img, 150, 150, FirFilter::CatmullRom);
            black_box(resized)
        });
    });

    // Compare Lanczos3 between crates for larger output
    group.bench_function("image_crate_lanczos3_600", |b| {
        b.iter(|| {
            let resized = img.resize(600, 600, ImgFilter::Lanczos3);
            black_box(resized)
        });
    });

    group.bench_function("fast_image_resize_lanczos3_600", |b| {
        b.iter(|| {
            let resized = resize_fast(&img, 600, 600, FirFilter::Lanczos3);
            black_box(resized)
        });
    });

    // Compare full pipeline with fast resize
    group.bench_function("full_pipeline_fast_150", |b| {
        b.iter(|| {
            let img = decode_image(&large_photo_bytes);
            let resized = resize_fast(&img, 150, 150, FirFilter::CatmullRom);
            let encoded = encode_jpeg(&resized, 80);
            black_box(encoded)
        });
    });

    group.bench_function("full_pipeline_old_150", |b| {
        b.iter(|| {
            let img = decode_image(&large_photo_bytes);
            let resized = resize_to_thumbnail(&img, 150, 150);
            let encoded = encode_jpeg(&resized, 80);
            black_box(encoded)
        });
    });

    group.finish();
}

// =============================================================================
// BENCHMARK: Encode only (JPEG)
// =============================================================================

fn bench_encode_jpeg(c: &mut Criterion) {
    let large_photo_bytes = load_large_photo();
    let img = decode_image(&large_photo_bytes);
    let thumbnail = resize_to_thumbnail(&img, 150, 150);

    let mut group = c.benchmark_group("encode_jpeg");

    for quality in [60, 80, 100] {
        group.bench_with_input(
            BenchmarkId::new("thumbnail", format!("q{}", quality)),
            &quality,
            |b, &quality| {
                b.iter(|| {
                    let encoded = encode_jpeg(&thumbnail, quality);
                    black_box(encoded)
                });
            },
        );
    }

    // Also benchmark encoding the full-size image
    for quality in [60, 80, 100] {
        group.bench_with_input(
            BenchmarkId::new("fullsize", format!("q{}", quality)),
            &quality,
            |b, &quality| {
                b.iter(|| {
                    let encoded = encode_jpeg(&img, quality);
                    black_box(encoded)
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// BENCHMARK: Encode only (WebP)
// =============================================================================

fn bench_encode_webp(c: &mut Criterion) {
    let large_photo_bytes = load_large_photo();
    let img = decode_image(&large_photo_bytes);
    let thumbnail = resize_to_thumbnail(&img, 150, 150);

    let mut group = c.benchmark_group("encode_webp");

    for quality in [60, 80, 100] {
        group.bench_with_input(
            BenchmarkId::new("thumbnail", format!("q{}", quality)),
            &quality,
            |b, &quality| {
                b.iter(|| {
                    let encoded = encode_webp(&thumbnail, quality);
                    black_box(encoded)
                });
            },
        );
    }

    // Also benchmark encoding the full-size image
    for quality in [60, 80, 100] {
        group.bench_with_input(
            BenchmarkId::new("fullsize", format!("q{}", quality)),
            &quality,
            |b, &quality| {
                b.iter(|| {
                    let encoded = encode_webp(&img, quality);
                    black_box(encoded)
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// BENCHMARK: Cache operations
// =============================================================================

fn bench_cache_operations(c: &mut Criterion) {
    use sha2::{Digest, Sha256};

    let large_photo_bytes = load_large_photo();
    let img = decode_image(&large_photo_bytes);
    let thumbnail = resize_to_thumbnail(&img, 150, 150);
    let encoded_jpg = encode_jpeg(&thumbnail, 80);
    let encoded_webp = encode_webp(&thumbnail, 80);

    let mut group = c.benchmark_group("cache");

    // Benchmark cache key generation
    group.bench_function("generate_key", |b| {
        b.iter(|| {
            let mut hasher = Sha256::new();
            hasher.update(b"https://example.com/image.jpg");
            hasher.update(150u32.to_le_bytes());
            hasher.update(150u32.to_le_bytes());
            hasher.update(b"contain");
            hasher.update([80u8]);
            hasher.update(b"jpg");
            let key = hex::encode(hasher.finalize());
            black_box(key)
        });
    });

    // Benchmark cache write + read (disk I/O)
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let cache_path = temp_dir.path().join("cache");
    fs::create_dir_all(&cache_path).expect("Failed to create cache dir");

    // Write benchmark (JPEG)
    group.throughput(Throughput::Bytes(encoded_jpg.len() as u64));
    group.bench_function("write_jpg_thumbnail", |b| {
        let mut counter = 0u64;
        b.iter(|| {
            counter += 1;
            let file_path = cache_path.join(format!("test_jpg_{}.jpg", counter));
            fs::write(&file_path, &encoded_jpg).expect("Failed to write");
            black_box(file_path)
        });
    });

    // Write a file for read benchmark
    let read_test_path = cache_path.join("read_test.jpg");
    fs::write(&read_test_path, &encoded_jpg).expect("Failed to write test file");

    // Read benchmark (JPEG)
    group.throughput(Throughput::Bytes(encoded_jpg.len() as u64));
    group.bench_function("read_jpg_thumbnail", |b| {
        b.iter(|| {
            let data = fs::read(&read_test_path).expect("Failed to read");
            black_box(data)
        });
    });

    // Write benchmark (WebP)
    group.throughput(Throughput::Bytes(encoded_webp.len() as u64));
    group.bench_function("write_webp_thumbnail", |b| {
        let mut counter = 0u64;
        b.iter(|| {
            counter += 1;
            let file_path = cache_path.join(format!("test_webp_{}.webp", counter));
            fs::write(&file_path, &encoded_webp).expect("Failed to write");
            black_box(file_path)
        });
    });

    // Write a WebP file for read benchmark
    let read_test_webp_path = cache_path.join("read_test.webp");
    fs::write(&read_test_webp_path, &encoded_webp).expect("Failed to write test file");

    // Read benchmark (WebP)
    group.throughput(Throughput::Bytes(encoded_webp.len() as u64));
    group.bench_function("read_webp_thumbnail", |b| {
        b.iter(|| {
            let data = fs::read(&read_test_webp_path).expect("Failed to read");
            black_box(data)
        });
    });

    group.finish();
}

// =============================================================================
// BENCHMARK: Different source sizes
// =============================================================================

fn bench_different_sizes(c: &mut Criterion) {
    let large_photo_bytes = load_large_photo();
    let medium_photo_bytes = load_medium_photo();

    let mut group = c.benchmark_group("size_comparison");

    // Large photo to thumbnail (JPG q80)
    group.throughput(Throughput::Bytes(large_photo_bytes.len() as u64));
    group.bench_function("large_to_thumb_jpg_q80", |b| {
        b.iter(|| {
            let img = decode_image(&large_photo_bytes);
            let resized = resize_to_thumbnail(&img, 150, 150);
            let encoded = encode_jpeg(&resized, 80);
            black_box(encoded)
        });
    });

    // Medium photo to thumbnail (JPG q80)
    group.throughput(Throughput::Bytes(medium_photo_bytes.len() as u64));
    group.bench_function("medium_to_thumb_jpg_q80", |b| {
        b.iter(|| {
            let img = decode_image(&medium_photo_bytes);
            let resized = resize_to_thumbnail(&img, 150, 150);
            let encoded = encode_jpeg(&resized, 80);
            black_box(encoded)
        });
    });

    // Large photo to thumbnail (WebP q80)
    group.throughput(Throughput::Bytes(large_photo_bytes.len() as u64));
    group.bench_function("large_to_thumb_webp_q80", |b| {
        b.iter(|| {
            let img = decode_image(&large_photo_bytes);
            let resized = resize_to_thumbnail(&img, 150, 150);
            let encoded = encode_webp(&resized, 80);
            black_box(encoded)
        });
    });

    // Medium photo to thumbnail (WebP q80)
    group.throughput(Throughput::Bytes(medium_photo_bytes.len() as u64));
    group.bench_function("medium_to_thumb_webp_q80", |b| {
        b.iter(|| {
            let img = decode_image(&medium_photo_bytes);
            let resized = resize_to_thumbnail(&img, 150, 150);
            let encoded = encode_webp(&resized, 80);
            black_box(encoded)
        });
    });

    group.finish();
}

// =============================================================================
// BENCHMARK: Format comparison (JPEG vs WebP at same quality)
// =============================================================================

fn bench_format_comparison(c: &mut Criterion) {
    let large_photo_bytes = load_large_photo();
    let img = decode_image(&large_photo_bytes);
    let thumbnail = resize_to_thumbnail(&img, 150, 150);

    let mut group = c.benchmark_group("format_comparison");

    for quality in [60, 80, 100] {
        group.bench_with_input(
            BenchmarkId::new("jpeg", format!("q{}", quality)),
            &quality,
            |b, &quality| {
                b.iter(|| {
                    let encoded = encode_jpeg(&thumbnail, quality);
                    black_box(encoded)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("webp", format!("q{}", quality)),
            &quality,
            |b, &quality| {
                b.iter(|| {
                    let encoded = encode_webp(&thumbnail, quality);
                    black_box(encoded)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_full_pipeline_jpg_to_jpg,
    bench_full_pipeline_jpg_to_webp,
    bench_decode,
    bench_resize,
    bench_resize_filters,
    bench_fast_image_resize,
    bench_encode_jpeg,
    bench_encode_webp,
    bench_cache_operations,
    bench_different_sizes,
    bench_format_comparison,
);

criterion_main!(benches);
