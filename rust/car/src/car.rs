use std::{collections::HashMap, collections::HashSet, io::Read, path::Path};

use deku::prelude::*;
use deku::reader::Reader;
use thiserror::Error;

use crate::model::*;
use bom::{
    BOM, BOMEror,
    raw::{BOMBytes, BOMStr},
};

pub type CarResult<T> = Result<T, CarError>;

type RenditionKey = Box<[u16]>;

mod tree_key {
    pub(crate) const APPEARANCE_KEYS: &[u8] = "APPEARANCEKEYS".as_bytes();
    pub(crate) const EXTENDED_METADATA: &[u8] = "EXTENDED_METADATA".as_bytes();
    pub(crate) const CAR_HEADER: &[u8] = "CARHEADER".as_bytes();
    pub(crate) const KEY_FORMAT: &[u8] = "KEYFORMAT".as_bytes();
    pub(crate) const FACET_KEYS: &[u8] = "FACETKEYS".as_bytes();
    pub(crate) const RENDITIONS: &[u8] = "RENDITIONS".as_bytes();
}

#[derive(Debug)]
pub struct CSIItem {
    pub(crate) attrs: Vec<rendition::Attribute>,
    pub(crate) header: CSIHeader,
    pub(crate) key_values: RenditionKey,
}

/// Crop rectangle extracted from a `RenditionTypeReference` TLV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Result of resolving an `InternalReference` rendition.
#[derive(Debug)]
pub struct ResolvedInternalReference<'a> {
    /// The final non-reference source rendition item.
    pub source: &'a CSIItem,
    /// The raw layout value from the first `RenditionTypeReference.layout` field.
    pub effective_layout: u16,
    /// Crop rectangles accumulated from each resolved reference layer (innermost first).
    pub crops: Vec<ReferenceRect>,
}

pub struct Car {
    pub(crate) header: Header,
    pub(crate) appearance_keys: HashMap<String, BOMBytes>,
    pub(crate) extended_metadata: ExtendedMetadata,
    pub(crate) key_fmt: rendition::KeyFmt,
    facet_keys: HashMap<String, rendition::KeyToken>,
    items_by_identifier: HashMap<u16, Vec<CSIItem>>,
    exact_item_by_key: HashMap<RenditionKey, (u16, usize)>,
    attr_index_map: HashMap<u16, usize>,
}

impl Car {
    pub fn new<P: AsRef<Path>>(file: P) -> CarResult<Self> {
        Self::from_bom(BOM::new_with_file(file)?)
    }

    pub fn from_reader<R>(mut reader: R) -> CarResult<Self>
    where
        R: Read,
    {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> CarResult<Self> {
        Self::from_bom(BOM::from_bytes(bytes)?)
    }

    fn from_bom(mut bom: BOM) -> CarResult<Self> {
        let key_fmt: rendition::KeyFmt = bom.read_block_with_name(tree_key::KEY_FORMAT)?;
        let attr_id_index = key_fmt
            .attribute_types
            .iter()
            .position(|attr_type| attr_type == &rendition::AttributeType::Identifier)
            .ok_or(CarError::KeyFormatNotFoundIDAttr)?;

        // Catalogs without appearance variants may omit this tree.
        let appearance_keys: HashMap<_, _> =
            match bom.read_tree_to_map::<BOMStr, BOMBytes>(tree_key::APPEARANCE_KEYS) {
                Ok(keys) => keys,
                Err(BOMEror::NotFoundVar(name)) if name.as_bytes() == tree_key::APPEARANCE_KEYS => {
                    HashMap::new()
                }
                Err(error) => return Err(error.into()),
            }
            .into_iter()
            .map(|item| (item.0.content, item.1))
            .collect();
        let extended_metadata: ExtendedMetadata =
            bom.read_block_with_name(tree_key::EXTENDED_METADATA)?;
        let header: Header = bom.read_block_with_name(tree_key::CAR_HEADER)?;

        let facet_keys: HashMap<_, _> = bom
            .read_tree_to_map::<BOMStr, rendition::KeyToken>(tree_key::FACET_KEYS)?
            .into_iter()
            .map(|item| (item.0.content, item.1))
            .collect();

        let mut items_by_identifier: HashMap<u16, Vec<CSIItem>> = HashMap::new();
        let mut exact_item_by_key: HashMap<RenditionKey, (u16, usize)> = HashMap::new();
        bom.parse_tree(tree_key::RENDITIONS, |k, v| {
            let mut k_reader = Reader::new(k);
            let mut attrs: Vec<rendition::Attribute> =
                Vec::with_capacity(key_fmt.attribute_types.len());
            for attr_type in &key_fmt.attribute_types {
                let attr_val = u16::from_reader_with_ctx(&mut k_reader, deku::ctx::Endian::Little)?;
                attrs.push(rendition::Attribute {
                    name: *attr_type,
                    val: attr_val,
                });
            }

            let header = CSIHeader::from_bom_block(v)?;

            let id = attrs.get(attr_id_index).unwrap().val;
            let key_values = attrs.iter().map(|attr| attr.val).collect::<Vec<_>>();
            let item = CSIItem {
                attrs,
                header,
                key_values: key_values.clone().into_boxed_slice(),
            };
            let items = items_by_identifier.entry(id).or_default();
            let item_index = items.len();
            exact_item_by_key
                .entry(key_values.into_boxed_slice())
                .or_insert((id, item_index));
            items.push(item);

            Ok(())
        })?;

        let attr_index_map: HashMap<u16, usize> = key_fmt
            .attribute_types
            .iter()
            .enumerate()
            .map(|(idx, &attr_type)| (attribute_type_tag(attr_type), idx))
            .collect();

        Ok(Self {
            header,
            appearance_keys,
            extended_metadata,
            key_fmt,
            facet_keys,
            items_by_identifier,
            exact_item_by_key,
            attr_index_map,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn appearance_keys(&self) -> &HashMap<String, BOMBytes> {
        &self.appearance_keys
    }

    pub fn extended_metadata(&self) -> &ExtendedMetadata {
        &self.extended_metadata
    }

    pub fn key_format(&self) -> &rendition::KeyFmt {
        &self.key_fmt
    }

    pub fn item_with_key_values(&self, key_values: &[u16]) -> Option<&CSIItem> {
        let &(identifier, item_index) = self.exact_item_by_key.get(key_values)?;
        self.item_by_identifier(identifier, item_index)
    }

    pub fn resolved_source_bytes(&self, item: &CSIItem) -> CarResult<Vec<u8>> {
        let source = if matches!(item.layout(), rendition::LayoutType::InternalReference) {
            self.try_resolve_internal_reference(item)?.source
        } else {
            item
        };

        source.source_bytes_owned()
    }

    pub fn facets(&self) -> Vec<&rendition::KeyToken> {
        self.named_facets()
            .into_iter()
            .map(|(_, facet)| facet)
            .collect()
    }

    pub fn named_facets(&self) -> Vec<(&str, &rendition::KeyToken)> {
        let mut facets: Vec<_> = self
            .facet_keys
            .iter()
            .map(|(k, v)| (k.as_str(), v))
            .collect();
        facets.sort_unstable_by_key(|(name, _)| *name);
        facets
    }

    pub fn facet(&self, name: &str) -> Option<&rendition::KeyToken> {
        self.facet_keys.get(name)
    }

    pub fn facet_with_name(&self, name: &str) -> Option<&rendition::KeyToken> {
        self.facet(name)
    }

    pub fn items_for_facet(&self, facet: &rendition::KeyToken) -> Option<&[CSIItem]> {
        facet_identifier(facet)
            .and_then(|identifier| self.items_by_identifier.get(&identifier))
            .map(Vec::as_slice)
    }

    pub fn rendtions_with_facet(&self, facet: &rendition::KeyToken) -> Option<&[CSIItem]> {
        self.items_for_facet(facet)
    }

    /// Correctly-spelled alias for [`Car::rendtions_with_facet`].
    ///
    /// The historical misspelled method remains available for compatibility.
    pub fn renditions_with_facet(&self, facet: &rendition::KeyToken) -> Option<&[CSIItem]> {
        self.items_for_facet(facet)
    }

    /// Alias for [`Car::items_for_facet`] using the public rendition terminology.
    pub fn renditions_for_facet(&self, facet: &rendition::KeyToken) -> Option<&[CSIItem]> {
        self.items_for_facet(facet)
    }

    pub fn items_for_name(&self, name: &str) -> Option<&[CSIItem]> {
        self.facet(name)
            .and_then(|facet| self.items_for_facet(facet))
    }

    pub fn rendtions_with_name(&self, name: &str) -> Option<&[CSIItem]> {
        self.items_for_name(name)
    }

    /// Correctly-spelled alias for [`Car::rendtions_with_name`].
    ///
    /// The historical misspelled method remains available for compatibility.
    pub fn renditions_with_name(&self, name: &str) -> Option<&[CSIItem]> {
        self.items_for_name(name)
    }

    /// Alias for [`Car::items_for_name`] using the public rendition terminology.
    pub fn renditions_for_name(&self, name: &str) -> Option<&[CSIItem]> {
        self.items_for_name(name)
    }

    pub fn face_item_with_name(&self, name: &str) -> Option<FacetItem<'_>> {
        self.facet(name).and_then(|facet| {
            self.items_for_facet(facet)
                .map(|resources| FacetItem { facet, resources })
        })
    }

    /// Correctly-spelled alias for [`Car::face_item_with_name`].
    ///
    /// The historical misspelled method remains available for compatibility.
    pub fn facet_item_with_name(&self, name: &str) -> Option<FacetItem<'_>> {
        self.face_item_with_name(name)
    }

    /// Resolve an `InternalReference` rendition to its final source rendition.
    ///
    /// Searches ALL entries in the RENDITIONS tree using exact key matching, not just
    /// facet-accessible items.  Recursion depth is capped at 16; cycles return `None`.
    ///
    /// Returns `None` if `item` is not an `InternalReference`, if the reference cannot
    /// be parsed, or if the source cannot be located.
    pub fn resolve_internal_reference<'a>(
        &'a self,
        item: &'a CSIItem,
    ) -> Option<ResolvedInternalReference<'a>> {
        self.try_resolve_internal_reference(item).ok()
    }

    /// Resolve an `InternalReference` rendition and report typed failure reasons.
    ///
    /// The compatibility method [`Car::resolve_internal_reference`] maps all errors
    /// to `None`.
    pub fn try_resolve_internal_reference<'a>(
        &'a self,
        item: &'a CSIItem,
    ) -> Result<ResolvedInternalReference<'a>, ReferenceResolveError> {
        if !matches!(
            item.header.metadata.layout_type,
            rendition::LayoutType::InternalReference
        ) {
            return Err(ReferenceResolveError::NotInternalReference);
        }

        let reference = reference_tlv(item).ok_or(ReferenceResolveError::MissingReferenceTlv)?;

        let effective_layout = reference.layout;

        let mut crops = Vec::new();
        let mut visited: HashSet<RenditionKey> = HashSet::new();

        let source = self.try_resolve_ref_inner(reference, &mut crops, &mut visited, 0)?;

        Ok(ResolvedInternalReference {
            source,
            effective_layout,
            crops,
        })
    }

    fn try_resolve_ref_inner<'a>(
        &'a self,
        reference: &rendition::RenditionTypeReference,
        crops: &mut Vec<ReferenceRect>,
        visited: &mut HashSet<RenditionKey>,
        depth: usize,
    ) -> Result<&'a CSIItem, ReferenceResolveError> {
        if depth >= 16 {
            return Err(ReferenceResolveError::DepthLimit { max_depth: 16 });
        }

        // Parse Reference.keys as (attr_id: u16, value: u16) LE pairs until (0, 0).
        let mut key_array = vec![0u16; self.key_fmt.attribute_types.len()];
        let mut chunks = reference.keys.chunks_exact(4);
        for chunk in &mut chunks {
            let attr_id = u16::from_le_bytes([chunk[0], chunk[1]]);
            let value = u16::from_le_bytes([chunk[2], chunk[3]]);
            if attr_id == 0 && value == 0 {
                break;
            }
            let idx = *self
                .attr_index_map
                .get(&attr_id)
                .ok_or(ReferenceResolveError::UnknownAttribute { attr_id })?;
            key_array[idx] = value;
        }
        if !chunks.remainder().is_empty() {
            return Err(ReferenceResolveError::MalformedKey {
                byte_len: reference.keys.len(),
            });
        }

        // Cycle detection using the complete key array.
        let key_values = key_array.into_boxed_slice();
        if !visited.insert(key_values.clone()) {
            return Err(ReferenceResolveError::Cycle {
                key_values: key_values.to_vec(),
            });
        }

        // Exact key match across ALL RENDITIONS entries (not just facet-accessible items).
        let &(identifier, item_index) = self
            .exact_item_by_key
            .get(key_values.as_ref())
            .ok_or_else(|| ReferenceResolveError::TargetNotFound {
                key_values: key_values.to_vec(),
            })?;
        let source = self
            .item_by_identifier(identifier, item_index)
            .ok_or_else(|| ReferenceResolveError::TargetNotFound {
                key_values: key_values.to_vec(),
            })?;

        // Recurse if the found source is itself an InternalReference.
        //
        // Push this layer's frame AFTER recursing so that `crops` is ordered
        // innermost-first.  `save_image_with_crops` iterates the slice front-to-back
        // and composites inner-frame first, then outer-frame — which is the correct
        // order for a chain  A → B → C:  place C into B's frame, then the result
        // into A's frame.
        if matches!(
            source.header.metadata.layout_type,
            rendition::LayoutType::InternalReference
        ) {
            let next_ref =
                reference_tlv(source).ok_or(ReferenceResolveError::MissingReferenceTlv)?;
            let result = self.try_resolve_ref_inner(next_ref, crops, visited, depth + 1);
            if result.is_ok() {
                crops.push(reference_rect(reference));
            }
            result
        } else {
            // Leaf source: push and return.
            crops.push(reference_rect(reference));
            Ok(source)
        }
    }
}

fn reference_tlv(item: &CSIItem) -> Option<&rendition::RenditionTypeReference> {
    item.header.tlv_data.iter().find_map(|tlv| {
        if let rendition::RenditionType::Reference(reference) = tlv {
            Some(reference)
        } else {
            None
        }
    })
}

fn reference_rect(reference: &rendition::RenditionTypeReference) -> ReferenceRect {
    ReferenceRect {
        x: reference.x,
        y: reference.y,
        width: reference.width,
        height: reference.height,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ReferenceResolveError {
    #[error("rendition is not an InternalReference")]
    NotInternalReference,
    #[error("InternalReference has no Reference TLV")]
    MissingReferenceTlv,
    #[error("InternalReference key byte length is not divisible into attr/value pairs: {byte_len}")]
    MalformedKey { byte_len: usize },
    #[error("InternalReference uses unknown attribute id {attr_id}")]
    UnknownAttribute { attr_id: u16 },
    #[error("InternalReference target not found for key {key_values:?}")]
    TargetNotFound { key_values: Vec<u16> },
    #[error("InternalReference cycle detected at key {key_values:?}")]
    Cycle { key_values: Vec<u16> },
    #[error("InternalReference depth limit reached: {max_depth}")]
    DepthLimit { max_depth: usize },
}

#[derive(Debug)]
pub struct FacetItem<'a> {
    facet: &'a rendition::KeyToken,
    resources: &'a [CSIItem],
}

impl<'a> FacetItem<'a> {
    pub fn facet(&self) -> &'a rendition::KeyToken {
        self.facet
    }

    pub fn resources(&self) -> &'a [CSIItem] {
        self.resources
    }
}

impl CSIItem {
    pub fn attributes(&self) -> &[rendition::Attribute] {
        &self.attrs
    }

    pub fn header(&self) -> &CSIHeader {
        &self.header
    }

    pub fn name(&self) -> &str {
        &self.header.metadata.name
    }

    pub fn width(&self) -> u32 {
        self.header.width
    }

    pub fn height(&self) -> u32 {
        self.header.height
    }

    pub fn scale(&self) -> u32 {
        self.attrs
            .iter()
            .find(|attr| attr.name == rendition::AttributeType::Scale && attr.val > 0)
            .map(|attr| u32::from(attr.val))
            .unwrap_or(self.header.scale_factor / 100)
    }

    pub fn layout(&self) -> rendition::LayoutType {
        self.header.metadata.layout_type
    }

    pub fn encoding(&self) -> Encoding {
        self.header.encoding
    }

    pub fn color_model(&self) -> ColorModel {
        self.header.color_model
    }

    pub fn rendition_kind(&self) -> rendition::RenditionKind {
        match &self.header.rendition {
            Some(rendition::Rendition::Color(_)) => rendition::RenditionKind::Color,
            Some(rendition::Rendition::RawData(_)) => rendition::RenditionKind::RawData,
            Some(rendition::Rendition::ThemeCBCK(_)) => rendition::RenditionKind::ThemeCBCK,
            Some(rendition::Rendition::MultisizeImageSet(_)) => {
                rendition::RenditionKind::MultisizeImageSet
            }
            Some(rendition::Rendition::Unknown { .. }) => rendition::RenditionKind::Unknown,
            None => rendition::RenditionKind::None,
        }
    }

    pub fn compression(&self) -> Option<rendition::CompressionType> {
        match &self.header.rendition {
            Some(rendition::Rendition::ThemeCBCK(cbck)) => Some(cbck.compression_type),
            _ => None,
        }
    }

    pub fn key_values(&self) -> &[u16] {
        &self.key_values
    }

    pub fn uti(&self) -> Option<String> {
        self.header.tlv_data.iter().find_map(|tlv| match tlv {
            rendition::RenditionType::UTI(uti) => Some(
                String::from_utf8_lossy(&uti.string)
                    .trim_end_matches('\0')
                    .to_string(),
            ),
            _ => None,
        })
    }

    pub(crate) fn bytes_per_row_tlv(&self) -> Option<u32> {
        self.header.tlv_data.iter().find_map(|tlv| match tlv {
            rendition::RenditionType::BytesPerRow { bytes_per_row, .. } => Some(*bytes_per_row),
            _ => None,
        })
    }
}

impl Car {
    fn item_by_identifier(&self, identifier: u16, item_index: usize) -> Option<&CSIItem> {
        self.items_by_identifier
            .get(&identifier)
            .and_then(|items| items.get(item_index))
    }
}

fn facet_identifier(facet: &rendition::KeyToken) -> Option<u16> {
    facet
        .attrs
        .iter()
        .find(|attr| attr.name == rendition::AttributeType::Identifier)
        .map(|attr| attr.val)
}

fn attribute_type_tag(attr_type: rendition::AttributeType) -> u16 {
    match attr_type {
        rendition::AttributeType::ThemeLook => 0,
        rendition::AttributeType::Element => 1,
        rendition::AttributeType::Part => 2,
        rendition::AttributeType::Size => 3,
        rendition::AttributeType::Direction => 4,
        rendition::AttributeType::Placeholder => 5,
        rendition::AttributeType::Value => 6,
        rendition::AttributeType::ThemeAppearance => 7,
        rendition::AttributeType::Dimension1 => 8,
        rendition::AttributeType::Dimension2 => 9,
        rendition::AttributeType::State => 10,
        rendition::AttributeType::Layer => 11,
        rendition::AttributeType::Scale => 12,
        rendition::AttributeType::Localization => 13,
        rendition::AttributeType::PresentationState => 14,
        rendition::AttributeType::Idiom => 15,
        rendition::AttributeType::Subtype => 16,
        rendition::AttributeType::Identifier => 17,
        rendition::AttributeType::PreviousValue => 18,
        rendition::AttributeType::PreviousState => 19,
        rendition::AttributeType::HorizontalSizeClass => 20,
        rendition::AttributeType::VerticalSizeClass => 21,
        rendition::AttributeType::MemoryLevelClass => 22,
        rendition::AttributeType::GraphicsFeatureSetClass => 23,
        rendition::AttributeType::DisplayGamut => 24,
        rendition::AttributeType::DeploymentTarget => 25,
        rendition::AttributeType::GlyphWeight => 26,
        rendition::AttributeType::GlyphSize => 27,
        rendition::AttributeType::Unknown { tag } => tag,
    }
}

#[derive(Error, Debug)]
pub enum CarError {
    #[error("Bom read failed {0}")]
    BOM(#[from] BOMEror),
    #[error("KeyFormat not found id attr")]
    KeyFormatNotFoundIDAttr,
    #[error("Rendition not found id attr")]
    RenditionNotFoundIDAttr,
    #[error("Unsupported compression type: {0:?}")]
    UnsupportedCompression(rendition::CompressionType),
    #[error("Decode failed: {0}")]
    DecodeFailed(String),
    #[error("Decode budget exceeded: {0}")]
    DecodeBudgetExceeded(crate::decode::DecodeBudgetError),
    #[error("Reference resolve failed: {0}")]
    ReferenceResolve(#[from] ReferenceResolveError),
    #[error("Deepmap2 error: {0}")]
    Deepmap2(#[from] deepmap2::Deepmap2Error),
    #[error("Unsupported encoding: {0:?}")]
    UnsupportedEncoding(Encoding),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[cfg(feature = "image")]
    #[error("Image error: {0}")]
    Image(#[from] image::ImageError),
}

impl From<deepmap2::apple_compression::AppleCompressionError> for CarError {
    fn from(error: deepmap2::apple_compression::AppleCompressionError) -> Self {
        match error {
            deepmap2::apple_compression::AppleCompressionError::DecodeFailed(message) => {
                Self::DecodeFailed(message)
            }
        }
    }
}
