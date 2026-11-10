//! Stable image-oriented helpers for [`crate::CSIItem`].
//!
//! This module keeps optional image functionality out of the core parser API.
//! Callers can decode or save image-like renditions through free functions
//! instead of depending on feature-gated inherent methods.
//!
//! Typical usage:
//!
//! ```no_run
//! # #[cfg(feature = "image")]
//! # fn demo(item: &car::CSIItem) -> Result<(), car::CarError> {
//! let image = car::image::to_image(item)?;
//! car::image::save_image(item, "/tmp/example.png")?;
//! let _ = image;
//! # Ok(())
//! # }
//! ```

use crate::car::{CSIItem, CarError, ReferenceRect};
use crate::decode::DecodeOptions;
use crate::image_conv::RgbaPayload;

pub type CarImageError = CarError;

pub fn to_image(item: &CSIItem) -> Result<image::DynamicImage, CarImageError> {
    item.to_image()
}

pub fn to_image_with_options(
    item: &CSIItem,
    options: &DecodeOptions,
) -> Result<image::DynamicImage, CarImageError> {
    item.to_image_with_options(options)
}

/// Decode, apply reference crops in order, then apply the requested EXIF orientation.
pub fn to_image_with_crops_and_options(
    item: &CSIItem,
    crops: &[ReferenceRect],
    options: &DecodeOptions,
) -> Result<image::DynamicImage, CarImageError> {
    item.to_image_with_crops_and_options(crops, options)
}

pub fn rgba_bytes(item: &CSIItem) -> Result<RgbaPayload, CarImageError> {
    item.rgba_bytes()
}

pub fn rgba_bytes_with_crops(
    item: &CSIItem,
    crops: &[ReferenceRect],
) -> Result<RgbaPayload, CarImageError> {
    item.rgba_bytes_with_crops(crops)
}

pub fn rgba_bytes_with_crops_and_options(
    item: &CSIItem,
    crops: &[ReferenceRect],
    options: &DecodeOptions,
) -> Result<RgbaPayload, CarImageError> {
    item.rgba_bytes_with_crops_and_options(crops, options)
}
