#ifndef CAR_PARSER_FFI_H
#define CAR_PARSER_FFI_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// 已打开的 `.car` 档案（不透明句柄）。
typedef struct CarArchiveHandle CarArchiveHandle;

/// 由本库分配的字节缓冲区，使用 `car_buffer_free` 释放。
typedef struct {
    uint8_t *data;
    size_t len;
} CarBuffer;

/// 非预乘 RGBA8 像素，每行 `width * 4` 字节、无填充。
typedef struct {
    uint32_t width;
    uint32_t height;
    CarBuffer pixels;
} CarRgbaImage;

/// 从文件路径打开（内存映射）。失败返回 NULL，并在 `out_error` 非空时写入错误描述
/// （使用 `car_string_free` 释放）。
CarArchiveHandle *car_archive_open_path(const char *path, char **out_error);

/// 从内存字节打开（会复制一份数据）。
CarArchiveHandle *car_archive_open_bytes(const uint8_t *bytes, size_t len, char **out_error);

/// 关闭并释放句柄，可传 NULL。
void car_archive_close(CarArchiveHandle *handle);

/// 文档信息 JSON（对象）。失败返回 NULL。
char *car_archive_info_json(const CarArchiveHandle *handle, char **out_error);

/// 条目清单 JSON（数组，元素的 `index` 用于后续取像素/原始数据）。失败返回 NULL。
char *car_archive_entries_json(const CarArchiveHandle *handle, char **out_error);

/// 解码条目为 RGBA8（内部引用已解析并裁剪）。成功时 `out_image->pixels` 需用
/// `car_buffer_free` 释放。
bool car_archive_entry_rgba(const CarArchiveHandle *handle,
                            uint32_t index,
                            CarRgbaImage *out_image,
                            char **out_error);

/// 取回条目的原始字节（JPEG/WEBP/HEIF/PDF/SVG/Data 等）。失败时 `out_buffer` 置空。
bool car_archive_entry_original(const CarArchiveHandle *handle,
                                uint32_t index,
                                CarBuffer *out_buffer,
                                char **out_error);

void car_string_free(char *text);
void car_buffer_free(CarBuffer buffer);

#ifdef __cplusplus
}
#endif

#endif
