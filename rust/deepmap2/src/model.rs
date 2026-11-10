use deku::prelude::*;

/// deepmap2 解码类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, DekuRead)]
#[deku(endian = "little", id_type = "u8", ctx = "endian: deku::ctx::Endian")]
pub enum DecodeType {
    /// Type 1: 原始未压缩数据
    #[deku(id = 1)]
    None,
    /// Type 2: LZFSE 多流 + zigzag + 预测器 + YCoCg
    #[deku(id = 2)]
    Default,
    /// Type 3: LZFSE 压缩（无预测器变换）
    #[deku(id = 3)]
    Lossless,
    /// Type 4: 调色板索引
    #[deku(id = 4)]
    Palette,
    /// 未知类型
    #[deku(id_pat = "_")]
    Unknown { tag: u8 },
}

/// 像素格式
#[derive(Debug, Clone, Copy, PartialEq, Eq, DekuRead)]
#[deku(endian = "little", id_type = "u8", ctx = "endian: deku::ctx::Endian")]
pub enum PixelFormat {
    /// 1 字节/像素: 灰度
    #[deku(id = 1)]
    G8,
    /// 2 字节/像素: 灰度 + alpha
    #[deku(id = 2)]
    GA88,
    /// 3 字节/像素: RGB
    #[deku(id = 3)]
    Rgb888,
    /// 4 字节/像素: RGBA
    #[deku(id = 4)]
    Rgba8888,
    /// 未知格式
    #[deku(id_pat = "_")]
    Unknown { tag: u8 },
}

impl PixelFormat {
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::G8 => 1,
            Self::GA88 => 2,
            Self::Rgb888 => 3,
            Self::Rgba8888 => 4,
            Self::Unknown { tag } => tag as usize,
        }
    }

    pub fn has_alpha(self) -> bool {
        matches!(self, Self::GA88 | Self::Rgba8888)
    }

    pub fn is_color(self) -> bool {
        matches!(self, Self::Rgb888 | Self::Rgba8888)
    }

    /// 每像素分量数（YCoCg 流分量数: 彩色=3, 灰度=1）
    pub fn split_stream_components(self) -> usize {
        if self.is_color() { 3 } else { 1 }
    }
}

/// 预测器类型（按行编码在解压数据中，不是全局头字段）
#[derive(Debug, Clone, Copy, PartialEq, Eq, DekuRead)]
#[deku(endian = "little", id_type = "u8", ctx = "endian: deku::ctx::Endian")]
pub enum Predictor {
    #[deku(id = 0)]
    None,
    #[deku(id = 1)]
    Paeth,
    #[deku(id = 2)]
    Left,
    #[deku(id = 3)]
    Up,
    #[deku(id = 4)]
    Mean,
    #[deku(id_pat = "_")]
    Unknown { tag: u8 },
}

/// deepmap2 文件头（小端序，deku 解析）
///
/// 二进制布局（12 字节基础，Palette 类型有额外字段）:
/// ```text
/// offset  size  field
/// 0       4     magic = b"dmp2"
/// 4       1     decode_type
/// 5       1     version (aux_flag_0: chroma scale 开关)
/// 6       1     predictor_type (aux_flag_1: 实际为每行数据中的预测器，此字段含义待定)
/// 7       1     pixel_format
/// 8       2     width (LE)
/// 10      2     height (LE)
/// --- Palette 专用 ---
/// 12      2     palette_size
/// 14      2     palette_type (3 or 4)
/// 16      palette_size*4  palette (BGRA u32 LE entries)
/// ```
#[derive(Debug, Clone, DekuRead)]
#[deku(magic = b"dmp2", endian = "little")]
pub struct Deepmap2Header {
    pub decode_type: DecodeType,
    /// chroma scale 开关（非零=开启 chroma scaling）
    pub version: u8,
    /// 保留/未知辅助标志
    pub predictor_type: u8,
    pub pixel_format: PixelFormat,
    pub width: u16,
    pub height: u16,
    // Palette 专用字段
    #[deku(cond = "matches!(decode_type, DecodeType::Palette)")]
    pub palette_size: Option<u16>,
    #[deku(cond = "matches!(decode_type, DecodeType::Palette)")]
    pub palette_type: Option<u16>,
    #[deku(count = "palette_size.unwrap_or(0) as usize")]
    pub palette: Vec<u32>,
}

impl Deepmap2Header {
    /// 返回该头部消耗的字节数（payload 的起始偏移量）
    pub fn header_size(&self) -> usize {
        if matches!(self.decode_type, DecodeType::Palette) {
            16 + self.palette_size.unwrap_or(0) as usize * 4
        } else {
            12
        }
    }

    /// chroma scale 值（用于 YCoCg 转换中的色度缩放）
    pub fn chroma_scale(&self) -> u8 {
        if self.version != 0 { 1 } else { 0 }
    }

    pub fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }
}
