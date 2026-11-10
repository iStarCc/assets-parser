use std::borrow::Cow;

use crate::car::{CSIItem, CarError, CarResult};
#[cfg(feature = "image")]
use crate::decode::{DecodeOptions, DecodeStage, OrientationPolicy, RenditionData};
#[cfg(feature = "image")]
use crate::metadata::ExifOrientation;
use crate::model::Encoding;
use crate::model::rendition::{CompressionType, Rendition};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaPayload {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[cfg(feature = "image")]
fn resolve_bytes_per_row(item: &CSIItem, bpp: u32) -> CarResult<u32> {
    let min_row = item.header.width * bpp;
    if let Some(bytes_per_row) = item.bytes_per_row_tlv() {
        if bytes_per_row < min_row {
            return Err(CarError::DecodeFailed(
                "bytes_per_row smaller than minimum row size".to_string(),
            ));
        }
        return Ok(bytes_per_row);
    }
    Ok(min_row)
}

#[cfg(feature = "image")]
fn strip_stride(data: Vec<u8>, width: u32, height: u32, bpp: u32, stride: u32) -> Vec<u8> {
    let row_bytes = (width * bpp) as usize;
    let stride = stride as usize;
    if row_bytes == stride {
        return data;
    }
    let mut out = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height as usize {
        let start = row * stride;
        let end = start + row_bytes;
        if end <= data.len() {
            out.extend_from_slice(&data[start..end]);
        }
    }
    out
}

#[cfg(feature = "image")]
fn is_deepmap2(item: &CSIItem) -> bool {
    matches!(
        &item.header.rendition,
        Some(Rendition::ThemeCBCK(cbck)) if matches!(cbck.compression_type, CompressionType::Deepmap2)
    )
}

pub(crate) fn is_hevc_compressed(item: &CSIItem) -> bool {
    matches!(
        &item.header.rendition,
        Some(Rendition::ThemeCBCK(cbck)) if matches!(cbck.compression_type, CompressionType::HEVC)
    )
}

fn hevc_source_bytes(item: &CSIItem) -> CarResult<Option<Cow<'_, [u8]>>> {
    let Some(Rendition::ThemeCBCK(cbck)) = &item.header.rendition else {
        return Ok(None);
    };
    if !matches!(cbck.compression_type, CompressionType::HEVC) {
        return Ok(None);
    }
    if cbck.chunks.len() != 1 {
        return Err(CarError::DecodeFailed(format!(
            "HEVC raw export expects a single chunk, got {}",
            cbck.chunks.len()
        )));
    }

    heic_container_from_hevc_chunk(cbck.chunks[0].raw_data.as_slice())
        .map(|data| Some(Cow::Borrowed(data)))
}

fn heic_container_from_hevc_chunk(raw: &[u8]) -> CarResult<&[u8]> {
    const WRAPPER_LEN: usize = 8;
    const MIN_FTYP_LEN: usize = 16;

    if raw.len() < WRAPPER_LEN + MIN_FTYP_LEN {
        return Err(CarError::DecodeFailed(
            "HEVC chunk too short to contain HEIC payload".to_string(),
        ));
    }

    let declared_len = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
    let body_len = raw.len() - WRAPPER_LEN;
    if declared_len != body_len {
        return Err(CarError::DecodeFailed(format!(
            "HEVC chunk length mismatch: declared {}, got {}",
            declared_len, body_len
        )));
    }

    let body = &raw[WRAPPER_LEN..];
    let ftyp_len = u32::from_be_bytes(body[0..4].try_into().unwrap()) as usize;
    if ftyp_len < MIN_FTYP_LEN || ftyp_len > body.len() {
        return Err(CarError::DecodeFailed(format!(
            "HEVC chunk has invalid ftyp box length: {ftyp_len}"
        )));
    }
    if &body[4..8] != b"ftyp" {
        return Err(CarError::DecodeFailed(
            "HEVC chunk does not start with HEIC ftyp box".to_string(),
        ));
    }
    if !ftyp_has_heif_brand(&body[..ftyp_len]) {
        return Err(CarError::DecodeFailed(
            "HEVC chunk ftyp box does not advertise HEIC/HEIF brand".to_string(),
        ));
    }

    Ok(body)
}

fn ftyp_has_heif_brand(ftyp_box: &[u8]) -> bool {
    is_heif_brand(&ftyp_box[8..12]) || ftyp_box[16..].chunks_exact(4).any(is_heif_brand)
}

fn is_heif_brand(brand: &[u8]) -> bool {
    matches!(
        brand,
        b"heic"
            | b"heix"
            | b"hevc"
            | b"hevx"
            | b"heim"
            | b"heis"
            | b"mif1"
            | b"msf1"
            | b"MiHE"
            | b"MiHB"
    )
}

#[cfg(feature = "image")]
fn unpremultiply_rgba8_in_place(pixels: &mut [u8]) {
    for px in pixels.chunks_exact_mut(4) {
        let alpha = px[3];
        if alpha == 0 || alpha == u8::MAX {
            continue;
        }

        for channel in &mut px[..3] {
            let value = (((*channel as u32) * u8::MAX as u32) + (alpha as u32 / 2)) / alpha as u32;
            *channel = value.min(u8::MAX as u32) as u8;
        }
    }
}

#[cfg(feature = "image")]
fn unpremultiply_rgba16_in_place(pixels: &mut [u16]) {
    for px in pixels.chunks_exact_mut(4) {
        let alpha = px[3];
        if alpha == 0 || alpha == u16::MAX {
            continue;
        }

        for channel in &mut px[..3] {
            let value = (((*channel as u64) * u16::MAX as u64) + (alpha as u64 / 2)) / alpha as u64;
            *channel = value.min(u16::MAX as u64) as u16;
        }
    }
}

/// Crop `img` to the sub-region described by `rect`.
///
/// `RenditionTypeReference.(x, y, width, height)` stores a CGRect-style frame whose
/// origin is measured from the source image's bottom-left corner. `image` uses a
/// top-left origin, so `y` must be flipped before cropping.
#[cfg(feature = "image")]
fn apply_reference_frame(
    img: image::DynamicImage,
    rect: &super::ReferenceRect,
) -> CarResult<image::DynamicImage> {
    let img_width = img.width();
    let img_height = img.height();
    let right = rect.x.checked_add(rect.width);
    let top = rect.y.checked_add(rect.height);
    match (right, top) {
        (Some(right), Some(top)) if right <= img_width && top <= img_height => {
            let crop_y = img_height - top;
            Ok(img.crop_imm(rect.x, crop_y, rect.width, rect.height))
        }
        _ => Err(CarError::DecodeFailed(format!(
            "reference crop rect (x={}, y={}, w={}, h={}) exceeds source image {}x{}",
            rect.x, rect.y, rect.width, rect.height, img_width, img_height
        ))),
    }
}

#[cfg(feature = "image")]

#[cfg(feature = "image")]
fn apply_exif_orientation(
    item: &CSIItem,
    options: &DecodeOptions,
    img: image::DynamicImage,
) -> image::DynamicImage {
    if !matches!(options.orientation_policy, OrientationPolicy::ApplyExif) {
        return img;
    }

    match item.exif_orientation() {
        Some(ExifOrientation::Mirrored) => img.fliph(),
        Some(ExifOrientation::Rotated180) => img.rotate180(),
        Some(ExifOrientation::Rotated180Mirrored) => img.rotate180().fliph(),
        Some(ExifOrientation::Rotated90) => img.rotate90(),
        Some(ExifOrientation::Rotated90Mirrored) => img.rotate90().fliph(),
        Some(ExifOrientation::Rotated270) => img.rotate270(),
        Some(ExifOrientation::Rotated270Mirrored) => img.rotate270().fliph(),
        Some(ExifOrientation::None | ExifOrientation::Normal | ExifOrientation::Unknown(_))
        | None => img,
    }
}

#[cfg(feature = "image")]
impl CSIItem {
    pub fn to_image(&self) -> CarResult<image::DynamicImage> {
        self.to_image_with_options(&DecodeOptions::default())
    }

    pub fn to_image_with_options(&self, options: &DecodeOptions) -> CarResult<image::DynamicImage> {
        self.to_image_with_crops_and_options(&[], options)
    }

    /// Decode the source, crop in innermost-first order, then apply its EXIF orientation.
    ///
    /// Crop coordinates refer to the unoriented source. The image conversion budget
    /// applies before cropping, so a small crop cannot bypass the source budget.
    pub fn to_image_with_crops_and_options(
        &self,
        crops: &[super::ReferenceRect],
        options: &DecodeOptions,
    ) -> CarResult<image::DynamicImage> {
        let source_options = DecodeOptions {
            orientation_policy: OrientationPolicy::Ignore,
            ..options.clone()
        };
        let img = self.to_image_unoriented(&source_options)?;
        options.check_output_bytes(img.as_bytes().len(), DecodeStage::ImageConversion)?;
        let mut img = img;
        for rect in crops {
            img = apply_reference_frame(img, rect)?;
        }
        Ok(apply_exif_orientation(self, options, img))
    }

    fn to_image_unoriented(&self, options: &DecodeOptions) -> CarResult<image::DynamicImage> {
        use image::{DynamicImage, ImageBuffer};

        let data = match self.decode_data_with_options(options)? {
            Some(RenditionData::Image { data }) => data,
            _ => {
                return Err(CarError::UnsupportedEncoding(self.header.encoding));
            }
        };

        let w = self.header.width;
        let h = self.header.height;

        // Deepmap2 special case: always RGBA8888
        if is_deepmap2(self) {
            let row_bytes = (w as usize).saturating_mul(4);
            let header_len = row_bytes.saturating_mul(h as usize);
            let mut pixels = if data.len() == header_len {
                data
            } else {
                // Deepmap2 already returns a packed RGBA buffer. Some assets still carry a
                // BytesPerRow TLV for the original source pixel format, which is not applicable
                // after deepmap2 expansion. Only use stride stripping as a recovery path when the
                // decoded byte count does not already match the target RGBA image size.
                let stride = resolve_bytes_per_row(self, 4).unwrap_or(w * 4);
                strip_stride(data, w, h, 4, stride)
            };
            if row_bytes == 0 || pixels.len() % row_bytes != 0 {
                return Err(CarError::DecodeFailed(format!(
                    "RGBA buffer size mismatch: expected a multiple of {row_bytes} bytes, got {}",
                    pixels.len()
                )));
            }
            unpremultiply_rgba8_in_place(&mut pixels);
            let inferred_h = (pixels.len() / row_bytes) as u32;
            let buf = ImageBuffer::from_raw(w, inferred_h, pixels)
                .ok_or_else(|| CarError::DecodeFailed("RGBA buffer size mismatch".into()))?;
            return Ok(DynamicImage::ImageRgba8(buf));
        }

        match self.header.encoding {
            Encoding::ARGB => {
                let stride = resolve_bytes_per_row(self, 4)?;
                let mut pixels = strip_stride(data, w, h, 4, stride);
                // BGRA(预乘 alpha) → RGBA(直 alpha)
                for chunk in pixels.chunks_exact_mut(4) {
                    chunk.swap(0, 2);
                }
                unpremultiply_rgba8_in_place(&mut pixels);
                let buf = ImageBuffer::from_raw(w, h, pixels)
                    .ok_or_else(|| CarError::DecodeFailed("ARGB buffer size mismatch".into()))?;
                Ok(DynamicImage::ImageRgba8(buf))
            }
            Encoding::GRAY => {
                let stride = resolve_bytes_per_row(self, 1)?;
                let pixels = strip_stride(data, w, h, 1, stride);
                let buf = ImageBuffer::from_raw(w, h, pixels)
                    .ok_or_else(|| CarError::DecodeFailed("GRAY buffer size mismatch".into()))?;
                Ok(DynamicImage::ImageLuma8(buf))
            }
            Encoding::GA8 => {
                let stride = resolve_bytes_per_row(self, 2)?;
                let pixels = strip_stride(data, w, h, 2, stride);
                let buf = ImageBuffer::from_raw(w, h, pixels)
                    .ok_or_else(|| CarError::DecodeFailed("GA8 buffer size mismatch".into()))?;
                Ok(DynamicImage::ImageLumaA8(buf))
            }
            Encoding::ARGB16 => {
                let stride = resolve_bytes_per_row(self, 8)?;
                let stride = stride as usize;
                let row_bytes = (w * 8) as usize; // 4 channels × 2 bytes each (BGRA16)
                let mut pixels: Vec<u16> = Vec::with_capacity((w * h) as usize * 4);
                for row in 0..h as usize {
                    let row_start = row * stride;
                    if row_start + row_bytes > data.len() {
                        break;
                    }
                    // BGRA16 → RGBA16: write channels in R G B A order
                    for px in data[row_start..row_start + row_bytes].chunks_exact(8) {
                        let b = u16::from_le_bytes([px[0], px[1]]);
                        let g = u16::from_le_bytes([px[2], px[3]]);
                        let r = u16::from_le_bytes([px[4], px[5]]);
                        let a = u16::from_le_bytes([px[6], px[7]]);
                        pixels.push(r);
                        pixels.push(g);
                        pixels.push(b);
                        pixels.push(a);
                    }
                }
                unpremultiply_rgba16_in_place(&mut pixels);
                let buf = ImageBuffer::from_raw(w, h, pixels)
                    .ok_or_else(|| CarError::DecodeFailed("ARGB16 buffer size mismatch".into()))?;
                Ok(DynamicImage::ImageRgba16(buf))
            }
            Encoding::GA16 => {
                let stride = resolve_bytes_per_row(self, 4)?;
                let stride = stride as usize;
                let row_bytes = (w * 4) as usize; // 2 channels × 2 bytes each
                let mut pixels: Vec<u16> = Vec::with_capacity((w * h) as usize * 2);
                for row in 0..h as usize {
                    let row_start = row * stride;
                    if row_start + row_bytes > data.len() {
                        break;
                    }
                    for px in data[row_start..row_start + row_bytes].chunks_exact(4) {
                        pixels.push(u16::from_le_bytes([px[0], px[1]]));
                        pixels.push(u16::from_le_bytes([px[2], px[3]]));
                    }
                }
                let buf = ImageBuffer::from_raw(w, h, pixels)
                    .ok_or_else(|| CarError::DecodeFailed("GA16 buffer size mismatch".into()))?;
                Ok(DynamicImage::ImageLumaA16(buf))
            }
            Encoding::RGB5 => {
                let stride = resolve_bytes_per_row(self, 2)?;
                let stripped = strip_stride(data, w, h, 2, stride);
                // XRGB1555: bit layout [x r4 r3 r2 r1 r0 g4 g3 | g2 g1 g0 b4 b3 b2 b1 b0]
                let mut pixels = Vec::with_capacity((w * h * 3) as usize);
                for pair in stripped.chunks_exact(2) {
                    let val = u16::from_le_bytes([pair[0], pair[1]]);
                    let r5 = ((val >> 10) & 0x1F) as u8;
                    let g5 = ((val >> 5) & 0x1F) as u8;
                    let b5 = (val & 0x1F) as u8;
                    pixels.push((r5 << 3) | (r5 >> 2));
                    pixels.push((g5 << 3) | (g5 >> 2));
                    pixels.push((b5 << 3) | (b5 >> 2));
                }
                let buf = ImageBuffer::from_raw(w, h, pixels)
                    .ok_or_else(|| CarError::DecodeFailed("RGB5 buffer size mismatch".into()))?;
                Ok(DynamicImage::ImageRgb8(buf))
            }
            Encoding::JPEG | Encoding::WEBP => Ok(image::load_from_memory(&data)?),
            _ => Err(CarError::UnsupportedEncoding(self.header.encoding)),
        }
    }

    pub fn rgba_bytes(&self) -> CarResult<RgbaPayload> {
        self.rgba_bytes_with_crops(&[])
    }

    pub fn rgba_bytes_with_crops(&self, crops: &[super::ReferenceRect]) -> CarResult<RgbaPayload> {
        self.rgba_bytes_with_crops_and_options(crops, &DecodeOptions::default())
    }

    pub fn rgba_bytes_with_crops_and_options(
        &self,
        crops: &[super::ReferenceRect],
        options: &DecodeOptions,
    ) -> CarResult<RgbaPayload> {
        let img = self.to_image_with_crops_and_options(crops, options)?;
        let rgba_len = (img.width() as usize)
            .checked_mul(img.height() as usize)
            .and_then(|len| len.checked_mul(4))
            .ok_or_else(|| CarError::DecodeFailed("RGBA output size overflow".into()))?;
        options.check_output_bytes(rgba_len, DecodeStage::ImageConversion)?;
        let img = img.to_rgba8();
        Ok(RgbaPayload {
            width: img.width(),
            height: img.height(),
            rgba: img.into_raw(),
        })
    }

}

impl CSIItem {
    pub fn source_bytes(&self) -> CarResult<Cow<'_, [u8]>> {
        use crate::decode::RenditionDataRef;

        if let Some(bytes) = hevc_source_bytes(self)? {
            return Ok(bytes);
        }

        let preserves_source = matches!(
            self.header.encoding,
            Encoding::Data
                | Encoding::HEIF
                | Encoding::PDF
                | Encoding::SVG
                | Encoding::JPEG
                | Encoding::WEBP
        );
        if !preserves_source {
            return Err(CarError::UnsupportedEncoding(self.header.encoding));
        }

        match &self.header.rendition {
            Some(Rendition::RawData(rd)) => {
                return deepmap2::apple_compression::maybe_decode_lzfse_with_fallback(
                    rd.raw_data.as_slice(),
                )
                .map_err(CarError::from);
            }
            Some(Rendition::ThemeCBCK(cbck))
                if matches!(cbck.compression_type, CompressionType::Uncompressed)
                    && matches!(
                        self.header.encoding,
                        Encoding::HEIF
                            | Encoding::PDF
                            | Encoding::SVG
                            | Encoding::JPEG
                            | Encoding::WEBP
                    ) =>
            {
                let mut bytes = Vec::new();
                for chunk in &cbck.chunks {
                    let data = deepmap2::apple_compression::maybe_decode_lzfse_with_fallback(
                        chunk.raw_data.as_slice(),
                    )?;
                    bytes.extend_from_slice(data.as_ref());
                }
                return Ok(Cow::Owned(bytes));
            }
            _ => {}
        }

        match self.decode_data_ref()? {
            Some(RenditionDataRef::RawData { data }) => Ok(data),
            Some(RenditionDataRef::Image { data }) => Ok(data),
            _ => Err(CarError::UnsupportedEncoding(self.header.encoding)),
        }
    }

    pub fn source_bytes_owned(&self) -> CarResult<Vec<u8>> {
        self.source_bytes().map(Cow::into_owned)
    }

}
