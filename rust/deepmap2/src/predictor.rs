/// 预测器组大小（对应 YCoCg 三分量: Y、Co、Cg）
pub const PREDICTOR_GROUP_SIZE: usize = 3;

/// 截断除以 2（向零截断，与 Python trunc_div2 一致）
#[inline]
fn trunc_div2(v: i32) -> i32 {
    v / 2
}

/// 将 i32 截断为 i16 范围（wrapping）
#[inline]
fn wrap_i16(v: i32) -> i16 {
    v as i16
}

/// 对行数据应用反预测器，返回重建后的行数据。
///
/// `predictor_raw` 为每行存储的预测器字节（Deepmap2Predictor 枚举值）:
/// 0=None, 1=Paeth, 2=Left, 3=Up, 4=Mean
///
/// `row` 为当前行的 i16 样本切片（长度 = width * split_stream_components）
/// `prev_row` 为上一行的重建结果（第一行为 None）
pub fn apply_predictor(predictor_raw: u8, row: &[i16], prev_row: Option<&[i16]>) -> Vec<i16> {
    apply_predictor_with_stride(predictor_raw, row, prev_row, PREDICTOR_GROUP_SIZE)
}

pub(crate) fn apply_predictor_with_stride(
    predictor_raw: u8,
    row: &[i16],
    prev_row: Option<&[i16]>,
    stride: usize,
) -> Vec<i16> {
    let count = row.len();
    match predictor_raw {
        0 => unpredict_none(row),
        1 => unpredict_paeth(row, prev_row, count, stride),
        2 => unpredict_left(row, count, stride),
        3 => unpredict_up(row, prev_row, count),
        4 => unpredict_mean(row, prev_row, count, stride),
        _ => row.to_vec(),
    }
}

/// None 预测器: 恒等变换
fn unpredict_none(data: &[i16]) -> Vec<i16> {
    data.to_vec()
}

/// Left 预测器: 以 PREDICTOR_GROUP_SIZE 为步长累加
///
/// 前 PREDICTOR_GROUP_SIZE 个样本直接输出，之后每个样本加上
/// 相同组偏移的前一个输出值（即同分量的左邻像素）。
fn unpredict_left(data: &[i16], count: usize, stride: usize) -> Vec<i16> {
    let mut output = vec![0i16; count];
    let head = stride.min(count);
    output[..head].copy_from_slice(&data[..head]);
    for i in stride..count {
        output[i] = wrap_i16(data[i] as i32 + output[i - stride] as i32);
    }
    output
}

/// Up 预测器: 每个样本加上上一行相同位置的值
fn unpredict_up(data: &[i16], prev_row: Option<&[i16]>, count: usize) -> Vec<i16> {
    let mut output = vec![0i16; count];
    for i in 0..count {
        let up = prev_row.map_or(0, |p| p[i] as i32);
        output[i] = wrap_i16(data[i] as i32 + up);
    }
    output
}

/// Mean 预测器: 预测值 = truncate((left + up + 1) / 2)
fn unpredict_mean(data: &[i16], prev_row: Option<&[i16]>, count: usize, stride: usize) -> Vec<i16> {
    let mut output = vec![0i16; count];
    // 前 stride 个样本用 Up 预测
    for i in 0..stride.min(count) {
        let up = prev_row.map_or(0, |p| p[i] as i32);
        output[i] = wrap_i16(data[i] as i32 + up);
    }
    for i in stride..count {
        let left = output[i - stride] as i32;
        let up = prev_row.map_or(0, |p| p[i] as i32);
        let predictor = trunc_div2(left + up + 1);
        output[i] = wrap_i16(data[i] as i32 + predictor);
    }
    output
}

/// vImage 风格的 Paeth 预测器（仅在 left / up 之间二选一）
fn paeth_predictor(left: i32, up: i32, up_left: i32) -> i32 {
    let dist_left = (up - up_left).abs();
    let dist_up = (left - up_left).abs();
    if dist_left <= dist_up { left } else { up }
}

/// Paeth 预测器（组粒度，每组 PREDICTOR_GROUP_SIZE 个分量）
fn unpredict_paeth(
    data: &[i16],
    prev_row: Option<&[i16]>,
    count: usize,
    stride: usize,
) -> Vec<i16> {
    let mut output = vec![0i16; count];
    // 前 stride 个样本用 Up 预测
    for i in 0..stride.min(count) {
        let up = prev_row.map_or(0, |p| p[i] as i32);
        output[i] = wrap_i16(data[i] as i32 + up);
    }
    let mut i = stride;
    while i < count {
        let group_size = stride.min(count - i);
        // 判断本组用 left 还是 up（以组内首个分量为判断基准）
        let left0 = output[i - stride] as i32;
        let up0 = prev_row.map_or(0, |p| p[i] as i32);
        let up_left0 = prev_row.map_or(0, |p| p[i - stride] as i32);
        let predicted_first = paeth_predictor(left0, up0, up_left0);
        let use_left = predicted_first == left0;

        for offset in 0..group_size {
            let left = output[i + offset - stride] as i32;
            let up = prev_row.map_or(0, |p| p[i + offset] as i32);
            let base = if use_left { left } else { up };
            output[i + offset] = wrap_i16(data[i + offset] as i32 + base);
        }
        i += PREDICTOR_GROUP_SIZE;
    }
    output
}
