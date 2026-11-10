use std::{borrow::Cow, io::Read};

use flate2::read::MultiGzDecoder;
use lzfse_rust::LzfseRingDecoder;
use thiserror::Error;

use crate::car::{CarError, CarResult};
use crate::model::rendition::{
    CompressionType, MultisizeImageSetEntry, Rendition, RenditionThemeCBCK,
};
use crate::model::{ColorSpace, Encoding};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrientationPolicy {
    Ignore,
    ApplyExif,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeOptions {
    pub max_output_bytes: Option<usize>,
    pub orientation_policy: OrientationPolicy,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            max_output_bytes: None,
            orientation_policy: OrientationPolicy::Ignore,
        }
    }
}

impl DecodeOptions {
    pub fn with_max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = Some(max_output_bytes);
        self
    }

    pub fn apply_exif_orientation(mut self, apply: bool) -> Self {
        self.orientation_policy = if apply {
            OrientationPolicy::ApplyExif
        } else {
            OrientationPolicy::Ignore
        };
        self
    }

    pub(crate) fn check_output_bytes(&self, requested: usize, stage: DecodeStage) -> CarResult<()> {
        if let Some(limit) = self.max_output_bytes
            && requested > limit
        {
            return Err(CarError::DecodeBudgetExceeded(
                DecodeBudgetError::OutputBytesExceeded {
                    limit,
                    requested,
                    stage,
                },
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeStage {
    BorrowedPayload,
    Uncompressed,
    Lzfse,
    Zip,
    Lzvn,
    Deepmap2,
    ImageConversion,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecodeBudgetError {
    #[error(
        "decoded output would exceed budget at {stage:?}: requested {requested}, limit {limit}"
    )]
    OutputBytesExceeded {
        limit: usize,
        requested: usize,
        stage: DecodeStage,
    },
}

#[derive(Debug)]
pub enum RenditionData {
    Color {
        color_space: ColorSpace,
        components: Vec<f64>,
    },
    Image {
        data: Vec<u8>,
    },
    RawData {
        data: Vec<u8>,
    },
    MultisizeImageSet {
        entries: Vec<MultisizeImageSetEntry>,
    },
}

#[derive(Debug)]
pub enum RenditionDataRef<'a> {
    Color {
        color_space: Cow<'a, ColorSpace>,
        components: Cow<'a, [f64]>,
    },
    Image {
        data: Cow<'a, [u8]>,
    },
    RawData {
        data: Cow<'a, [u8]>,
    },
    MultisizeImageSet {
        entries: Cow<'a, [MultisizeImageSetEntry]>,
    },
}

impl<'a> RenditionDataRef<'a> {
    pub fn into_owned(self) -> RenditionData {
        match self {
            Self::Color {
                color_space,
                components,
            } => RenditionData::Color {
                color_space: color_space.into_owned(),
                components: components.into_owned(),
            },
            Self::Image { data } => RenditionData::Image {
                data: data.into_owned(),
            },
            Self::RawData { data } => RenditionData::RawData {
                data: data.into_owned(),
            },
            Self::MultisizeImageSet { entries } => RenditionData::MultisizeImageSet {
                entries: entries.into_owned(),
            },
        }
    }

    fn from_owned(data: RenditionData) -> Self {
        match data {
            RenditionData::Color {
                color_space,
                components,
            } => Self::Color {
                color_space: Cow::Owned(color_space),
                components: Cow::Owned(components),
            },
            RenditionData::Image { data } => Self::Image {
                data: Cow::Owned(data),
            },
            RenditionData::RawData { data } => Self::RawData {
                data: Cow::Owned(data),
            },
            RenditionData::MultisizeImageSet { entries } => Self::MultisizeImageSet {
                entries: Cow::Owned(entries),
            },
        }
    }
}

fn check_borrowed_payload_bytes<T>(options: &DecodeOptions, payload: &[T]) -> CarResult<()> {
    options.check_output_bytes(std::mem::size_of_val(payload), DecodeStage::BorrowedPayload)
}

impl crate::car::CSIItem {
    pub fn decode_data_ref(&self) -> CarResult<Option<RenditionDataRef<'_>>> {
        self.decode_data_ref_with_options(&DecodeOptions::default())
    }

    pub fn decode_data_ref_with_options(
        &self,
        options: &DecodeOptions,
    ) -> CarResult<Option<RenditionDataRef<'_>>> {
        let rendition = match &self.header.rendition {
            None => return Ok(None),
            Some(r) => r,
        };

        match rendition {
            Rendition::Color(rc) => {
                check_borrowed_payload_bytes(options, rc.components.as_slice())?;
                Ok(Some(RenditionDataRef::Color {
                    color_space: Cow::Borrowed(&rc.color_space),
                    components: Cow::Borrowed(&rc.components),
                }))
            }
            Rendition::RawData(rd) => {
                check_borrowed_payload_bytes(options, rd.raw_data.as_slice())?;
                Ok(Some(RenditionDataRef::RawData {
                    data: Cow::Borrowed(rd.raw_data.as_slice()),
                }))
            }
            Rendition::MultisizeImageSet(mis) => {
                check_borrowed_payload_bytes(options, mis.entries.as_slice())?;
                Ok(Some(RenditionDataRef::MultisizeImageSet {
                    entries: Cow::Borrowed(mis.entries.as_slice()),
                }))
            }
            Rendition::ThemeCBCK(cbck) => decode_theme_cbck(self, cbck, options)
                .map(|data| data.map(RenditionDataRef::from_owned)),
            Rendition::Unknown { tag, .. } => Err(CarError::DecodeFailed(format!(
                "Unknown rendition type: {:?}",
                tag
            ))),
        }
    }

    pub fn decode_data_owned(&self) -> CarResult<Option<RenditionData>> {
        self.decode_data_ref_with_options(&DecodeOptions::default())
            .map(|data| data.map(RenditionDataRef::into_owned))
    }

    pub fn decode_data_owned_with_options(
        &self,
        options: &DecodeOptions,
    ) -> CarResult<Option<RenditionData>> {
        self.decode_data_ref_with_options(options)
            .map(|data| data.map(RenditionDataRef::into_owned))
    }

    pub fn decode_data(&self) -> CarResult<Option<RenditionData>> {
        self.decode_data_owned()
    }

    pub fn decode_data_with_options(
        &self,
        options: &DecodeOptions,
    ) -> CarResult<Option<RenditionData>> {
        self.decode_data_owned_with_options(options)
    }
}

fn deepmap2_pixel_format_override(encoding: Encoding) -> Option<deepmap2::PixelFormat> {
    match encoding {
        Encoding::ARGB => Some(deepmap2::PixelFormat::Rgba8888),
        Encoding::ARGB16 => Some(deepmap2::PixelFormat::Rgba8888),
        Encoding::GA8 => Some(deepmap2::PixelFormat::GA88),
        _ => None,
    }
}

/// 像素编码的每像素字节数（仅对具有固定 pixel stride 的 raster encoding 返回 Some）
fn encoding_bytes_per_pixel(encoding: Encoding) -> Option<usize> {
    match encoding {
        Encoding::ARGB => Some(4),
        Encoding::GRAY => Some(1),
        Encoding::GA8 => Some(2),
        Encoding::ARGB16 => Some(8),
        Encoding::GA16 => Some(4),
        Encoding::RGB5 => Some(2),
        _ => None,
    }
}

fn chunk_output_height(
    chunk: &crate::model::rendition::ThemeChunk,
    total_chunks: usize,
    full_height: u16,
) -> CarResult<Option<u16>> {
    match chunk.part_header.as_ref().map(|header| header.height) {
        Some(height) if total_chunks == 1 && height == 0 => Ok(Some(full_height)),
        None if total_chunks == 1 => Ok(Some(full_height)),
        Some(0) | None => Ok(None),
        Some(height) => u16::try_from(height).map(Some).map_err(|_| {
            CarError::DecodeFailed(format!("Theme part height exceeds u16: {height}"))
        }),
    }
}

fn dmp2_region(chunk: &[u8], dmp2_pos: usize) -> &[u8] {
    let search_start = dmp2_pos.saturating_sub(0x40);
    if let Some(prev_kcbc_rel) = deepmap2::tile::find_bytes(&chunk[search_start..dmp2_pos], b"KCBC")
    {
        let kcbc_pos = search_start + prev_kcbc_rel;
        if kcbc_pos + 20 <= chunk.len() {
            let block_length =
                u32::from_le_bytes(chunk[kcbc_pos + 0x10..kcbc_pos + 0x14].try_into().unwrap())
                    as usize;
            let block_end = kcbc_pos.saturating_add(block_length);
            if block_length >= 0x20 && block_end > dmp2_pos && block_end <= chunk.len() {
                return &chunk[dmp2_pos..block_end];
            }
        }
    }

    if let Some(next_rel) = deepmap2::tile::find_bytes(&chunk[dmp2_pos + 4..], b"KCBC") {
        let next_kcbc = dmp2_pos + 4 + next_rel;
        return &chunk[dmp2_pos..next_kcbc];
    }

    &chunk[dmp2_pos..]
}

fn decode_theme_cbck(
    item: &crate::car::CSIItem,
    cbck: &RenditionThemeCBCK,
    options: &DecodeOptions,
) -> CarResult<Option<RenditionData>> {
    match &cbck.compression_type {
        CompressionType::Uncompressed => {
            let total: usize = cbck.chunks.iter().map(|c| c.raw_data.len()).sum();
            options.check_output_bytes(total, DecodeStage::Uncompressed)?;
            let mut data = Vec::with_capacity(total);
            for chunk in &cbck.chunks {
                data.extend_from_slice(chunk.raw_data.as_slice());
            }
            Ok(Some(RenditionData::Image { data }))
        }
        CompressionType::Lzfse => {
            let decode_chunk = |raw: &[u8]| -> CarResult<Vec<u8>> {
                let cap = raw.len().saturating_mul(16).max(4096);
                let mut output = Vec::with_capacity(cap);
                let mut decoder = LzfseRingDecoder::default();
                let n = decoder.decode_bytes(raw, &mut output).map_err(|_| {
                    CarError::DecodeFailed("LZFSE decompression failed".to_string())
                })?;
                if n == 0 && !raw.is_empty() {
                    return Err(CarError::DecodeFailed(
                        "LZFSE produced zero bytes for non-empty input".to_string(),
                    ));
                }
                output.truncate(n as usize);
                Ok(output)
            };

            // 顺序解压：避免嵌套并行与外层 extract par_iter 争抢线程池。
            let mut all_data = Vec::new();
            let mut total = 0usize;
            for chunk in &cbck.chunks {
                let part = decode_chunk(chunk.raw_data.as_slice())?;
                total = total.checked_add(part.len()).ok_or_else(|| {
                    CarError::DecodeFailed("LZFSE output length overflow".to_string())
                })?;
                options.check_output_bytes(total, DecodeStage::Lzfse)?;
                all_data.reserve(part.len());
                all_data.extend_from_slice(&part);
            }

            Ok(Some(RenditionData::Image { data: all_data }))
        }
        CompressionType::Zip => {
            let mut all_data = Vec::new();
            let mut total = 0usize;
            for chunk in &cbck.chunks {
                let mut decoder = MultiGzDecoder::new(chunk.raw_data.as_slice());
                let mut part = Vec::new();
                decoder.read_to_end(&mut part).map_err(|err| {
                    CarError::DecodeFailed(format!("Zip/gzip decompression failed: {err}"))
                })?;
                total = total.checked_add(part.len()).ok_or_else(|| {
                    CarError::DecodeFailed("Zip/gzip output length overflow".to_string())
                })?;
                options.check_output_bytes(total, DecodeStage::Zip)?;
                all_data.reserve(part.len());
                all_data.extend_from_slice(&part);
            }

            Ok(Some(RenditionData::Image { data: all_data }))
        }
        CompressionType::Lzvn => {
            let width = item.header.width as usize;
            let height = item.header.height as usize;
            // 与 Uncompressed/Lzfse 分支的契约一致：返回原始编码字节，
            // 按实际 encoding 的 bpp（以及可能存在的 BytesPerRow stride）计算
            // decoded_len —— lzvn::decode_raw 要求精确长度。
            let bpp = encoding_bytes_per_pixel(item.header.encoding).ok_or_else(|| {
                CarError::DecodeFailed(format!(
                    "LZVN decompression unsupported for encoding {:?}",
                    item.header.encoding
                ))
            })?;
            let min_row = width * bpp;
            // 优先使用 BytesPerRow TLV（可能大于 width*bpp，表示带行填充），
            // 否则回退到紧凑布局。to_image() 端会基于同一 TLV 剥离 stride。
            let bytes_per_row = match item.bytes_per_row_tlv() {
                Some(bpr) => {
                    let bpr = bpr as usize;
                    if bpr < min_row {
                        return Err(CarError::DecodeFailed(format!(
                            "bytes_per_row ({}) smaller than minimum row size ({})",
                            bpr, min_row
                        )));
                    }
                    bpr
                }
                None => min_row,
            };
            let expected_total = bytes_per_row * height;
            options.check_output_bytes(expected_total, DecodeStage::Lzvn)?;
            let mut all_data = Vec::with_capacity(expected_total);
            let is_single = cbck.chunks.len() == 1;
            for chunk in &cbck.chunks {
                // 单 chunk 对应整图，多 chunk 必须依据 part_header.height 计算本段行数。
                let expected_chunk = if is_single {
                    expected_total
                } else {
                    let chunk_height = chunk
                        .part_header
                        .as_ref()
                        .map(|h| h.height as usize)
                        .filter(|h| *h > 0)
                        .ok_or_else(|| {
                            CarError::DecodeFailed(
                                "LZVN multi-chunk missing part_header height".to_string(),
                            )
                        })?;
                    chunk_height * bytes_per_row
                };
                let decompressed =
                    deepmap2::codec::decompress_lzvn(chunk.raw_data.as_slice(), expected_chunk)?;
                all_data.extend_from_slice(&decompressed);
            }
            if all_data.len() != expected_total {
                return Err(CarError::DecodeFailed(format!(
                    "LZVN output size mismatch: expected {}, got {}",
                    expected_total,
                    all_data.len()
                )));
            }
            Ok(Some(RenditionData::Image { data: all_data }))
        }
        CompressionType::Deepmap2 => {
            let width = u16::try_from(item.header.width)
                .map_err(|_| CarError::DecodeFailed("CSI width exceeds u16".to_string()))?;
            let height = u16::try_from(item.header.height)
                .map_err(|_| CarError::DecodeFailed("CSI height exceeds u16".to_string()))?;
            let pixel_format_override = deepmap2_pixel_format_override(item.header.encoding);
            let mut all_data = Vec::new();
            options.check_output_bytes(
                (width as usize)
                    .saturating_mul(height as usize)
                    .saturating_mul(4),
                DecodeStage::Deepmap2,
            )?;
            let mut expected_width: Option<u16> = None;
            let mut decoded_height = 0u32;
            for chunk in &cbck.chunks {
                let output_height = chunk_output_height(chunk, cbck.chunks.len(), height)?;
                let chunk = chunk.raw_data.as_slice();
                let kcbc_pos = deepmap2::tile::find_bytes(chunk, b"KCBC");
                let dmp2_pos = deepmap2::tile::find_bytes(chunk, b"dmp2");
                let img = match (kcbc_pos, dmp2_pos) {
                    (Some(k), Some(d)) if k <= d => deepmap2::decode_kcbc_with_options(
                        &chunk[k..],
                        Some(width),
                        None,
                        pixel_format_override,
                    )?,
                    (Some(_), Some(d)) => deepmap2::decode_with_options(
                        dmp2_region(chunk, d),
                        Some(width),
                        output_height,
                        pixel_format_override,
                    )?,
                    (Some(k), None) => deepmap2::decode_kcbc_with_options(
                        &chunk[k..],
                        Some(width),
                        None,
                        pixel_format_override,
                    )?,
                    (None, Some(d)) => deepmap2::decode_with_options(
                        dmp2_region(chunk, d),
                        Some(width),
                        output_height,
                        pixel_format_override,
                    )?,
                    (None, None) => {
                        return Err(CarError::DecodeFailed(
                            "No deepmap2 magic found in chunk".to_string(),
                        ));
                    }
                };
                match expected_width {
                    None => expected_width = Some(img.width),
                    Some(w) if w != img.width => {
                        return Err(CarError::DecodeFailed(format!(
                            "Deepmap2 chunk width mismatch: expected {}, got {}",
                            w, img.width
                        )));
                    }
                    _ => {}
                }
                decoded_height = decoded_height.saturating_add(u32::from(img.height));
                all_data.extend_from_slice(&img.rgba);
                options.check_output_bytes(all_data.len(), DecodeStage::Deepmap2)?;
            }
            if cbck.chunks.len() > 1 && decoded_height != u32::from(height) {
                return Err(CarError::DecodeFailed(format!(
                    "Deepmap2 chunk height mismatch: expected {}, got {}",
                    height, decoded_height
                )));
            }
            Ok(Some(RenditionData::Image { data: all_data }))
        }
        _ => Err(CarError::UnsupportedCompression(cbck.compression_type)),
    }
}
