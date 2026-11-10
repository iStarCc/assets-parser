//! Apple LZFSE stream decoding shared by `deepmap2` and `car`.
//!
//! The pure-Rust decoder is tried first. When it rejects a stream, the system
//! `libcompression` decoder is used on Apple platforms.

use std::borrow::Cow;

use thiserror::Error;

const MAX_SYSTEM_DECODE_BYTES: usize = 1 << 30;

#[derive(Debug, Error)]
pub enum AppleCompressionError {
    #[error("Apple compression stream decompression failed: {0}")]
    DecodeFailed(String),
}

pub fn is_apple_compression_stream(data: &[u8]) -> bool {
    matches!(
        data.get(..4),
        Some(b"bvx1") | Some(b"bvx2") | Some(b"bvxn") | Some(b"bvx-") | Some(b"bvx$")
    )
}

pub fn maybe_decode_lzfse_with_fallback<'a>(
    data: &'a [u8],
) -> Result<Cow<'a, [u8]>, AppleCompressionError> {
    if !is_apple_compression_stream(data) {
        return Ok(Cow::Borrowed(data));
    }

    decode_lzfse_with_fallback(data).map(Cow::Owned)
}

pub fn decode_lzfse_with_fallback(data: &[u8]) -> Result<Vec<u8>, AppleCompressionError> {
    let cap = data.len().saturating_mul(16).max(4096);
    let mut out = Vec::with_capacity(cap);
    let mut decoder = lzfse_rust::LzfseRingDecoder::default();
    if let Ok(n) = decoder.decode_bytes(data, &mut out)
        && (n > 0 || data.is_empty())
    {
        out.truncate(n as usize);
        return Ok(out);
    }

    decode_lzfse_with_system_library(data)
}

#[cfg(target_vendor = "apple")]
fn decode_lzfse_with_system_library(data: &[u8]) -> Result<Vec<u8>, AppleCompressionError> {
    use std::ffi::c_void;
    use std::os::raw::c_int;
    use std::ptr;

    const COMPRESSION_LZFSE: c_int = 0x801;

    #[link(name = "compression")]
    unsafe extern "C" {
        fn compression_decode_buffer(
            dst_buffer: *mut u8,
            dst_size: usize,
            src_buffer: *const u8,
            src_size: usize,
            scratch_buffer: *mut c_void,
            algorithm: c_int,
        ) -> usize;
    }

    // compression_decode_buffer 在输出超出缓冲区时会静默截断，写满即扩容重试
    let mut capacity = data.len().saturating_mul(16).max(4096);
    loop {
        let mut out = vec![0u8; capacity];
        // SAFETY: 缓冲区指针和长度均来自有效的 Vec / 切片，scratch 传空由系统自行分配。
        let written = unsafe {
            compression_decode_buffer(
                out.as_mut_ptr(),
                capacity,
                data.as_ptr(),
                data.len(),
                ptr::null_mut(),
                COMPRESSION_LZFSE,
            )
        };

        if written == 0 {
            return Err(AppleCompressionError::DecodeFailed(
                "system LZFSE decoder rejected the stream".to_string(),
            ));
        }
        if written < capacity {
            out.truncate(written);
            return Ok(out);
        }
        if capacity >= MAX_SYSTEM_DECODE_BYTES {
            return Err(AppleCompressionError::DecodeFailed(
                "decoded LZFSE stream exceeds the size limit".to_string(),
            ));
        }
        capacity = capacity.saturating_mul(2).min(MAX_SYSTEM_DECODE_BYTES);
    }
}

#[cfg(not(target_vendor = "apple"))]
fn decode_lzfse_with_system_library(_data: &[u8]) -> Result<Vec<u8>, AppleCompressionError> {
    Err(AppleCompressionError::DecodeFailed(
        "system LZFSE decoder is only available on Apple platforms".to_string(),
    ))
}
