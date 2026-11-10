//! `car-parser` 的 C ABI 封装，供 Swift 通过 XCFramework 调用。
//!
//! 约定：
//! - 返回 `char *` / `CarBuffer` 的对象由调用方用 `car_string_free` / `car_buffer_free` 释放。
//! - 失败时函数返回 NULL / false，并在 `out_error` 非空时写入错误描述。
//! - 所有入口都捕获 panic，损坏的文件只会返回错误而不会终止进程。

mod catalog;

use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::ptr;

use catalog::Catalog;

pub struct CarArchiveHandle {
    catalog: Catalog,
}

#[repr(C)]
pub struct CarBuffer {
    pub data: *mut u8,
    pub len: usize,
}

#[repr(C)]
pub struct CarRgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: CarBuffer,
}

impl CarBuffer {
    const EMPTY: CarBuffer = CarBuffer {
        data: ptr::null_mut(),
        len: 0,
    };

    fn from_vec(bytes: Vec<u8>) -> Self {
        let boxed = bytes.into_boxed_slice();
        let len = boxed.len();
        Self {
            data: Box::into_raw(boxed).cast::<u8>(),
            len,
        }
    }
}

/// 在 panic 边界内执行，并把错误统一转换为字符串。
fn guarded<T>(body: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(result) => result,
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            Err(format!("car-parser panicked: {message}"))
        }
    }
}

fn into_c_string(text: String) -> *mut c_char {
    // 文本中不会出现内部 NUL；出现时退化为空串而不是 panic
    CString::new(text).unwrap_or_default().into_raw()
}

unsafe fn report_error(out_error: *mut *mut c_char, message: String) {
    if !out_error.is_null() {
        // SAFETY: 调用方保证 out_error 指向有效的 `char *` 存储。
        unsafe { *out_error = into_c_string(message) };
    }
}

unsafe fn finish<T>(result: Result<T, String>, out_error: *mut *mut c_char) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(message) => {
            // SAFETY: 透传调用方对 out_error 的约定。
            unsafe { report_error(out_error, message) };
            None
        }
    }
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

/// 从文件路径打开 `.car`（内存映射，不复制文件内容）。
///
/// # Safety
/// `path` 必须是以 NUL 结尾的有效 UTF-8 字符串；`out_error` 可为 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn car_archive_open_path(
    path: *const c_char,
    out_error: *mut *mut c_char,
) -> *mut CarArchiveHandle {
    let result = guarded(|| {
        if path.is_null() {
            return Err("path is null".to_string());
        }
        // SAFETY: 已检查非空，且调用方保证以 NUL 结尾。
        let path = unsafe { CStr::from_ptr(path) }
            .to_str()
            .map_err(|error| error.to_string())?;
        let catalog = Catalog::open_path(Path::new(path)).map_err(|error| error.to_string())?;
        Ok(Box::into_raw(Box::new(CarArchiveHandle { catalog })))
    });
    // SAFETY: 透传调用方对 out_error 的约定。
    unsafe { finish(result, out_error) }.unwrap_or(ptr::null_mut())
}

/// 从内存字节打开 `.car`（会复制一份数据）。
///
/// # Safety
/// `bytes` 必须指向至少 `len` 字节的有效内存；`out_error` 可为 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn car_archive_open_bytes(
    bytes: *const u8,
    len: usize,
    out_error: *mut *mut c_char,
) -> *mut CarArchiveHandle {
    let result = guarded(|| {
        if bytes.is_null() {
            return Err("bytes is null".to_string());
        }
        // SAFETY: 已检查非空，调用方保证长度有效。
        let data = unsafe { std::slice::from_raw_parts(bytes, len) }.to_vec();
        let catalog = Catalog::open_bytes(data).map_err(|error| error.to_string())?;
        Ok(Box::into_raw(Box::new(CarArchiveHandle { catalog })))
    });
    // SAFETY: 透传调用方对 out_error 的约定。
    unsafe { finish(result, out_error) }.unwrap_or(ptr::null_mut())
}

/// 关闭并释放句柄。
///
/// # Safety
/// `handle` 必须来自 `car_archive_open_*` 且只能关闭一次，可为 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn car_archive_close(handle: *mut CarArchiveHandle) {
    if !handle.is_null() {
        // SAFETY: 调用方保证句柄有效且未被释放。
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// 返回文档信息 JSON。
///
/// # Safety
/// `handle` 必须有效；`out_error` 可为 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn car_archive_info_json(
    handle: *const CarArchiveHandle,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    let result = guarded(|| {
        // SAFETY: 调用方保证句柄有效。
        let handle = unsafe { handle.as_ref() }.ok_or("handle is null")?;
        to_json(&handle.catalog.archive_info())
    });
    // SAFETY: 透传调用方对 out_error 的约定。
    unsafe { finish(result, out_error) }
        .map(into_c_string)
        .unwrap_or(ptr::null_mut())
}

/// 返回全部条目的 JSON 数组。
///
/// # Safety
/// `handle` 必须有效；`out_error` 可为 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn car_archive_entries_json(
    handle: *const CarArchiveHandle,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    let result = guarded(|| {
        // SAFETY: 调用方保证句柄有效。
        let handle = unsafe { handle.as_ref() }.ok_or("handle is null")?;
        to_json(&handle.catalog.entry_infos())
    });
    // SAFETY: 透传调用方对 out_error 的约定。
    unsafe { finish(result, out_error) }
        .map(into_c_string)
        .unwrap_or(ptr::null_mut())
}

/// 解码条目为非预乘 RGBA8 像素（内部引用已解析并裁剪）。
///
/// # Safety
/// `handle`、`out_image` 必须有效；`out_error` 可为 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn car_archive_entry_rgba(
    handle: *const CarArchiveHandle,
    index: u32,
    out_image: *mut CarRgbaImage,
    out_error: *mut *mut c_char,
) -> bool {
    let result = guarded(|| {
        // SAFETY: 调用方保证句柄有效。
        let handle = unsafe { handle.as_ref() }.ok_or("handle is null")?;
        if out_image.is_null() {
            return Err("out_image is null".to_string());
        }
        let image = handle
            .catalog
            .rgba(index)
            .map_err(|error| error.to_string())?;
        // SAFETY: 已检查 out_image 非空。
        unsafe {
            *out_image = CarRgbaImage {
                width: image.width,
                height: image.height,
                pixels: CarBuffer::from_vec(image.rgba),
            };
        }
        Ok(())
    });
    // SAFETY: 透传调用方对 out_error 的约定。
    unsafe { finish(result, out_error) }.is_some()
}

/// 取回条目的原始字节（JPEG/WEBP/HEIF/PDF/SVG/Data 等）。
///
/// # Safety
/// `handle`、`out_buffer` 必须有效；`out_error` 可为 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn car_archive_entry_original(
    handle: *const CarArchiveHandle,
    index: u32,
    out_buffer: *mut CarBuffer,
    out_error: *mut *mut c_char,
) -> bool {
    let result = guarded(|| {
        // SAFETY: 调用方保证句柄有效。
        let handle = unsafe { handle.as_ref() }.ok_or("handle is null")?;
        if out_buffer.is_null() {
            return Err("out_buffer is null".to_string());
        }
        let bytes = handle
            .catalog
            .original_bytes(index)
            .map_err(|error| error.to_string())?;
        // SAFETY: 已检查 out_buffer 非空。
        unsafe { *out_buffer = CarBuffer::from_vec(bytes) };
        Ok(())
    });
    if result.is_err() && !out_buffer.is_null() {
        // SAFETY: 失败时保证输出缓冲区处于确定的空状态。
        unsafe { *out_buffer = CarBuffer::EMPTY };
    }
    // SAFETY: 透传调用方对 out_error 的约定。
    unsafe { finish(result, out_error) }.is_some()
}

/// 释放 `car_*` 返回的字符串。
///
/// # Safety
/// `text` 必须来自本库且只能释放一次，可为 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn car_string_free(text: *mut c_char) {
    if !text.is_null() {
        // SAFETY: 该指针由 CString::into_raw 产生。
        drop(unsafe { CString::from_raw(text) });
    }
}

/// 释放 `CarBuffer`。
///
/// # Safety
/// `buffer` 必须来自本库且只能释放一次。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn car_buffer_free(buffer: CarBuffer) {
    if !buffer.data.is_null() {
        // SAFETY: 该缓冲区由 Box<[u8]>::into_raw 产生，长度与创建时一致。
        drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(buffer.data, buffer.len)) });
    }
}
