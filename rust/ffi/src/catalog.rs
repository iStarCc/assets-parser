//! 资源目录模型：把 `car::Car` 展开为 Swift 侧可直接消费的条目清单。

use std::path::Path;

use car::rendition::{CompressionType, LayoutType, Rendition, RenditionKind};
use car::{CSIItem, Car, CarError, ColorSpace, Encoding, ReferenceRect};
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    Image,
    Color,
    Document,
    RawData,
    Other,
}

/// 条目内容在宿主侧的获取方式。
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Preview {
    /// 解码为 RGBA8 像素（ARGB、GA、Deepmap2 等）。
    Rgba,
    /// 原始字节本身即可被系统解码（JPEG、WEBP、HEIF）。
    Encoded,
    /// 颜色条目，使用 `color` 字段。
    Color,
    /// 无法预览，仅可能导出原始字节。
    None,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorInfo {
    pub space: &'static str,
    pub components: Vec<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryInfo {
    pub index: u32,
    pub name: String,
    pub rendition_name: String,
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub idiom: &'static str,
    pub appearance: String,
    pub display_gamut: u16,
    pub layout: String,
    pub encoding: String,
    pub kind: EntryKind,
    pub preview: Preview,
    /// 可导出的原始字节扩展名；为空表示该条目没有可导出的原始数据。
    pub original_extension: Option<String>,
    pub color: Option<ColorInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveInfo {
    pub storage_version: u32,
    #[serde(rename = "coreUIVersion")]
    pub core_ui_version: u32,
    pub schema_version: u32,
    pub main_version_string: String,
    pub version_string: String,
    pub authoring_tool: String,
    pub deployment_platform: String,
    pub deployment_platform_version: String,
    pub rendition_count: u32,
    pub entry_count: u32,
}

struct EntryRecord {
    key_values: Vec<u16>,
    info: EntryInfo,
}

pub struct Catalog {
    car: Car,
    entries: Vec<EntryRecord>,
}

struct ResolvedVisual<'a> {
    logical: &'a CSIItem,
    payload: &'a CSIItem,
    crops: Vec<ReferenceRect>,
}

struct Classification {
    kind: EntryKind,
    preview: Preview,
    original_extension: Option<String>,
}

pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Catalog {
    pub fn open_path(path: &Path) -> Result<Self, CarError> {
        Ok(Self::build(Car::new(path)?))
    }

    pub fn open_bytes(bytes: Vec<u8>) -> Result<Self, CarError> {
        Ok(Self::build(Car::from_bytes(bytes)?))
    }

    fn build(car: Car) -> Self {
        let mut entries = Vec::new();

        for (facet_name, facet) in car.named_facets() {
            let Some(items) = car.items_for_facet(facet) else {
                continue;
            };

            for item in items {
                // 打包图集只是内部引用的像素来源，多尺寸图标集仅含元数据
                if matches!(item.layout(), LayoutType::PackedImage)
                    || matches!(item.rendition_kind(), RenditionKind::MultisizeImageSet)
                {
                    continue;
                }

                let Some(resolved) = resolve_visual(&car, item) else {
                    entries.push(unresolved_entry(&car, entries.len(), facet_name, item));
                    continue;
                };

                entries.push(EntryRecord {
                    key_values: item.key_values().to_vec(),
                    info: entry_info(&car, entries.len(), facet_name, &resolved),
                });
            }
        }

        Self { car, entries }
    }

    pub fn archive_info(&self) -> ArchiveInfo {
        let document = self.car.document_info();
        ArchiveInfo {
            storage_version: document.asset_storage_version,
            core_ui_version: document.core_ui_version,
            schema_version: document.schema_version,
            main_version_string: document.main_version_string,
            version_string: document.version_string,
            authoring_tool: document.authoring_tool,
            deployment_platform: document.deployment_platform,
            deployment_platform_version: document.deployment_platform_version,
            rendition_count: document.rendition_count,
            entry_count: self.entries.len() as u32,
        }
    }

    pub fn entry_infos(&self) -> Vec<&EntryInfo> {
        self.entries.iter().map(|entry| &entry.info).collect()
    }

    pub fn rgba(&self, index: u32) -> Result<RgbaImage, CatalogError> {
        let resolved = self.resolve_entry(index)?;
        let payload = car::image::rgba_bytes_with_crops(resolved.payload, &resolved.crops)?;
        Ok(RgbaImage {
            width: payload.width,
            height: payload.height,
            rgba: payload.rgba,
        })
    }

    pub fn original_bytes(&self, index: u32) -> Result<Vec<u8>, CatalogError> {
        let resolved = self.resolve_entry(index)?;
        Ok(self.car.resolved_source_bytes(resolved.logical)?)
    }

    fn resolve_entry(&self, index: u32) -> Result<ResolvedVisual<'_>, CatalogError> {
        let record = self
            .entries
            .get(index as usize)
            .ok_or(CatalogError::EntryNotFound(index))?;
        let logical = self
            .car
            .item_with_key_values(&record.key_values)
            .ok_or(CatalogError::EntryNotFound(index))?;
        resolve_visual(&self.car, logical).ok_or_else(|| CatalogError::UnresolvedReference(
            logical.name().to_string(),
        ))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("entry {0} not found")]
    EntryNotFound(u32),
    #[error("internal reference of `{0}` cannot be resolved")]
    UnresolvedReference(String),
    #[error(transparent)]
    Car(#[from] CarError),
}

fn resolve_visual<'a>(car: &'a Car, logical: &'a CSIItem) -> Option<ResolvedVisual<'a>> {
    match logical.layout() {
        LayoutType::InternalReference => {
            let resolved = car.try_resolve_internal_reference(logical).ok()?;
            Some(ResolvedVisual {
                logical,
                payload: resolved.source,
                crops: resolved.crops,
            })
        }
        _ => Some(ResolvedVisual {
            logical,
            payload: logical,
            crops: Vec::new(),
        }),
    }
}

fn unresolved_entry(car: &Car, index: usize, facet_name: &str, item: &CSIItem) -> EntryRecord {
    let resolved = ResolvedVisual {
        logical: item,
        payload: item,
        crops: Vec::new(),
    };
    let mut info = entry_info(car, index, facet_name, &resolved);
    info.kind = EntryKind::Other;
    info.preview = Preview::None;
    info.original_extension = None;
    info.color = None;
    EntryRecord {
        key_values: item.key_values().to_vec(),
        info,
    }
}

fn entry_info(car: &Car, index: usize, facet_name: &str, resolved: &ResolvedVisual<'_>) -> EntryInfo {
    let logical = resolved.logical;
    let attributes = logical.variant_attributes();
    let classification = classify(resolved);

    EntryInfo {
        index: index as u32,
        name: facet_name.to_string(),
        rendition_name: logical.name().to_string(),
        width: logical.width(),
        height: logical.height(),
        scale: logical.scale(),
        idiom: idiom_name(attributes.idiom),
        appearance: attributes
            .appearance
            .map(|value| appearance_name(car, value))
            .unwrap_or_default(),
        display_gamut: attributes.display_gamut.unwrap_or(0),
        layout: format!("{:?}", logical.layout()),
        encoding: format!("{:?}", resolved.payload.encoding()),
        kind: classification.kind,
        preview: classification.preview,
        original_extension: classification.original_extension,
        color: color_info(resolved.payload),
    }
}

fn classify(resolved: &ResolvedVisual<'_>) -> Classification {
    let payload = resolved.payload;

    let none = |kind| Classification {
        kind,
        preview: Preview::None,
        original_extension: None,
    };
    let encoded = |kind, extension: &str| Classification {
        kind,
        preview: Preview::Encoded,
        original_extension: Some(extension.to_string()),
    };
    let original_only = |kind, extension: String| Classification {
        kind,
        preview: Preview::None,
        original_extension: Some(extension),
    };

    match payload.rendition_kind() {
        RenditionKind::Color => {
            return Classification {
                kind: EntryKind::Color,
                preview: Preview::Color,
                original_extension: None,
            };
        }
        RenditionKind::MultisizeImageSet | RenditionKind::None | RenditionKind::Unknown => {
            return none(EntryKind::Other);
        }
        RenditionKind::RawData | RenditionKind::ThemeCBCK => {}
    }

    if matches!(payload.compression(), Some(CompressionType::HEVC)) {
        return encoded(EntryKind::Image, "heic");
    }

    match payload.encoding() {
        Encoding::JPEG => encoded(EntryKind::Image, "jpg"),
        Encoding::WEBP => encoded(EntryKind::Image, "webp"),
        Encoding::HEIF => encoded(EntryKind::Image, "heic"),
        Encoding::SVG => original_only(EntryKind::Image, "svg".to_string()),
        Encoding::PDF => original_only(EntryKind::Document, "pdf".to_string()),
        Encoding::ARGB
        | Encoding::ARGB16
        | Encoding::GRAY
        | Encoding::GA8
        | Encoding::GA16
        | Encoding::RGB5 => Classification {
            kind: EntryKind::Image,
            preview: Preview::Rgba,
            original_extension: None,
        },
        Encoding::Data => original_only(
            EntryKind::RawData,
            Path::new(resolved.logical.name())
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| "bin".to_string()),
        ),
        Encoding::None | Encoding::Unknown { .. } => none(EntryKind::Other),
    }
}

fn idiom_name(idiom: Option<u16>) -> &'static str {
    match idiom {
        Some(0) => "universal",
        Some(1) => "phone",
        Some(2) => "pad",
        Some(3) => "tv",
        Some(4) => "car",
        Some(5) => "watch",
        Some(6) => "marketing",
        Some(7) => "mac",
        Some(8) => "vision",
        _ => "",
    }
}

/// 外观键（如 `UIAppearanceDark`）按其数值反查名称；未登记时返回空串。
fn appearance_name(car: &Car, value: u16) -> String {
    car.appearance_keys()
        .iter()
        .find(|(_, bytes)| {
            let raw = &bytes.content;
            raw.len() >= 2 && u16::from_le_bytes([raw[0], raw[1]]) == value
        })
        .map(|(name, _)| name.clone())
        .unwrap_or_default()
}

fn color_info(item: &CSIItem) -> Option<ColorInfo> {
    let Rendition::Color(color) = item.header().rendition.as_ref()? else {
        return None;
    };
    Some(ColorInfo {
        space: color_space_name(&color.color_space),
        components: color.components.clone(),
    })
}

fn color_space_name(color_space: &ColorSpace) -> &'static str {
    match color_space {
        ColorSpace::SRGB => "srgb",
        ColorSpace::GrayGamma2_2 => "gray-gamma-2.2",
        ColorSpace::DisplayP3 => "display-p3",
        ColorSpace::ExtendedRangeSRGB => "extended-srgb",
        ColorSpace::ExtendedLinearSRGB => "extended-linear-srgb",
        ColorSpace::ExtendedGray => "extended-gray",
        ColorSpace::SystemSRGB => "system-srgb",
        ColorSpace::Unknown { .. } => "unknown",
    }
}
