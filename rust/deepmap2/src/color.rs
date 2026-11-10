use crate::model::PixelFormat;
use crate::predictor::PREDICTOR_GROUP_SIZE;

#[inline]
fn clamp_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// 截断除以 2（向零截断）
#[inline]
fn trunc_div2(v: i32) -> i32 {
    v / 2
}

/// YCoCg → RGB 转换
///
/// `scale`: chroma_scale（来自 header.version，非零则左移 1 位）
pub fn ycocg_to_rgb(y: i32, co: i32, cg: i32, scale: u8) -> (u8, u8, u8) {
    let co_scaled = co << scale;
    let cg_scaled = cg << scale;
    let co_half = trunc_div2(co_scaled);
    let cg_half = trunc_div2(cg_scaled);
    let temp = y - cg_half;
    let red = clamp_u8(temp + co_scaled - co_half);
    let green = clamp_u8(temp + cg_scaled);
    let blue = clamp_u8(temp - co_half);
    (red, green, blue)
}

/// 将一行解码后的 i16 样本（长度 = width * PREDICTOR_GROUP_SIZE）转换为 RGBA 字节
///
/// `alpha_row`: 若 pixel_format.has_alpha()，提供该行的 alpha 字节切片（长度=width）
/// `scale`: chroma_scale 值
pub fn row_to_rgba(
    pixel_format: PixelFormat,
    decoded_row: &[i16],
    alpha_row: Option<&[u8]>,
    scale: u8,
) -> Vec<u8> {
    let width = decoded_row.len() / PREDICTOR_GROUP_SIZE;
    let mut rgba = vec![0u8; width * 4];

    for px in 0..width {
        let sample_base = px * PREDICTOR_GROUP_SIZE;
        let rgba_base = px * 4;
        let luminance = decoded_row[sample_base];

        match pixel_format {
            PixelFormat::G8 => {
                let gray = luminance as u8;
                rgba[rgba_base..rgba_base + 4].copy_from_slice(&[gray, gray, gray, 0xFF]);
            }
            PixelFormat::GA88 => {
                let gray = luminance as u8;
                let alpha = alpha_row.map_or(0xFF, |a| a[px]);
                rgba[rgba_base..rgba_base + 4].copy_from_slice(&[gray, gray, gray, alpha]);
            }
            PixelFormat::Rgb888 => {
                let (red, green, blue) = ycocg_to_rgb(
                    luminance as i32,
                    decoded_row[sample_base + 1] as i32,
                    decoded_row[sample_base + 2] as i32,
                    scale,
                );
                rgba[rgba_base..rgba_base + 4].copy_from_slice(&[blue, green, red, 0xFF]);
            }
            PixelFormat::Rgba8888 => {
                let (red, green, blue) = ycocg_to_rgb(
                    luminance as i32,
                    decoded_row[sample_base + 1] as i32,
                    decoded_row[sample_base + 2] as i32,
                    scale,
                );
                let alpha = alpha_row.map_or(0xFF, |a| a[px]);
                rgba[rgba_base..rgba_base + 4].copy_from_slice(&[blue, green, red, alpha]);
            }
            PixelFormat::Unknown { .. } => {}
        }
    }
    rgba
}

/// 将 pixel_format 编码的原始字节直接转换为 RGBA（用于 None / Lossless 路径）
pub fn output_bytes_to_rgba(
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
    bytes: &[u8],
) -> Vec<u8> {
    let pixel_count = width * height;
    let mut rgba = vec![0u8; pixel_count * 4];

    match pixel_format {
        PixelFormat::G8 => {
            for (i, &gray) in bytes[..pixel_count].iter().enumerate() {
                rgba[i * 4..i * 4 + 4].copy_from_slice(&[gray, gray, gray, 0xFF]);
            }
        }
        PixelFormat::GA88 => {
            for i in 0..pixel_count {
                let gray = bytes[i * 2];
                let alpha = bytes[i * 2 + 1];
                rgba[i * 4..i * 4 + 4].copy_from_slice(&[gray, gray, gray, alpha]);
            }
        }
        PixelFormat::Rgb888 => {
            for i in 0..pixel_count {
                let base = i * 3;
                let blue = bytes[base];
                let green = bytes[base + 1];
                let red = bytes[base + 2];
                rgba[i * 4..i * 4 + 4].copy_from_slice(&[red, green, blue, 0xFF]);
            }
        }
        PixelFormat::Rgba8888 => {
            for i in 0..pixel_count {
                let base = i * 4;
                let blue = bytes[base];
                let green = bytes[base + 1];
                let red = bytes[base + 2];
                let alpha = bytes[base + 3];
                rgba[i * 4..i * 4 + 4].copy_from_slice(&[red, green, blue, alpha]);
            }
        }
        PixelFormat::Unknown { .. } => {}
    }
    rgba
}

/// 调色板条目（u32 LE BGRA 编码）转 RGBA 字节
pub fn palette_entry_to_rgba(entry: u32, alpha_override: Option<u8>) -> [u8; 4] {
    let blue = (entry & 0xFF) as u8;
    let green = ((entry >> 8) & 0xFF) as u8;
    let red = ((entry >> 16) & 0xFF) as u8;
    let alpha = alpha_override.unwrap_or(((entry >> 24) & 0xFF) as u8);
    [red, green, blue, alpha]
}

