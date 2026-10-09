//! 网易封面调色板的 5-bit MMCQ：体积优先分盒，人口加权中位切分，最多 16 色。
use gpui::{Hsla, rgb};

pub(super) fn cover_color(bytes: &[u8], width: u32, height: u32) -> Option<Hsla> {
    let pixels = (width as usize).checked_mul(height as usize)?;
    if width == 0 || height == 0 || bytes.len() / 4 < pixels {
        return None;
    }
    let sampled_width = width.min(50);
    let sampled_height = (sampled_width as u64 * height as u64).div_ceil(width as u64) as u32;
    let mut histogram = vec![0u32; 32768];
    for y in 0..sampled_height {
        for x in 0..sampled_width {
            let [b, g, r, a] =
                sample_bgra(bytes, width, height, sampled_width, sampled_height, x, y);
            if a >= 128 {
                histogram
                    [((r as usize >> 3) << 10) | ((g as usize >> 3) << 5) | (b as usize >> 3)] += 1;
            }
        }
    }
    dominant(&histogram, 16).map(to_hsl)
}

/// 以像素中心做双线性缩放；保持长宽比，避免跨桶的近邻色因抽样偏移被漏掉。
fn sample_bgra(bytes: &[u8], width: u32, height: u32, sw: u32, sh: u32, x: u32, y: u32) -> [u8; 4] {
    let sx = ((x as f64 + 0.5) * width as f64 / sw as f64 - 0.5).max(0.);
    let sy = ((y as f64 + 0.5) * height as f64 / sh as f64 - 0.5).max(0.);
    let x0 = sx.floor() as u32;
    let y0 = sy.floor() as u32;
    let dx = sx.fract();
    let dy = sy.fract();
    let corners = [
        (x0, y0, (1. - dx) * (1. - dy)),
        ((x0 + 1).min(width - 1), y0, dx * (1. - dy)),
        (x0, (y0 + 1).min(height - 1), (1. - dx) * dy),
        ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1), dx * dy),
    ];
    let mut alpha = 0.;
    let mut channels = [0.; 3];
    for (x, y, weight) in corners {
        let offset = (y as usize * width as usize + x as usize) * 4;
        let weight = weight * bytes[offset + 3] as f64;
        alpha += weight;
        for channel in 0..3 {
            channels[channel] += bytes[offset + channel] as f64 * weight;
        }
    }
    if alpha == 0. {
        return [0; 4];
    }
    [
        (channels[0] / alpha).round() as u8,
        (channels[1] / alpha).round() as u8,
        (channels[2] / alpha).round() as u8,
        alpha.round() as u8,
    ]
}

fn components(bin: u16) -> [u8; 3] {
    [(bin >> 10) as u8, ((bin >> 5) & 31) as u8, (bin & 31) as u8]
}

fn to_hsl([r, g, b]: [u8; 3]) -> Hsla {
    rgb((r as u32) << 16 | (g as u32) << 8 | b as u32).into()
}

fn allowed(color: [u8; 3]) -> bool {
    let color = to_hsl(color);
    color.l > 0.1 && !(color.l >= 0.85 && color.s <= 0.1)
}

fn dominant(histogram: &[u32], limit: usize) -> Option<[u8; 3]> {
    let all: Vec<_> = histogram
        .iter()
        .enumerate()
        .filter(|(_, count)| **count > 0)
        .map(|(bin, _)| bin as u16)
        .collect();
    let mut colors: Vec<_> = all
        .iter()
        .copied()
        .filter(|&bin| allowed(components(bin).map(|c| c * 8)))
        .collect();
    // 原版在这里误填了未过滤的桶；保留正确过滤，仅在全黑/全白时回退。
    if colors.is_empty() {
        colors = all;
    }
    if colors.is_empty() {
        return None;
    }
    let palette = if colors.len() <= limit {
        colors
            .into_iter()
            .map(|bin| (components(bin).map(|c| c * 8), histogram[bin as usize]))
            .collect::<Vec<_>>()
    } else {
        let mut queue = vec![ColorBox::new(colors, histogram)];
        while queue.len() < limit {
            let mut left = pop_box(&mut queue)?;
            let axis = if left.extent[0] >= left.extent[1] && left.extent[0] >= left.extent[2] {
                0
            } else if left.extent[1] >= left.extent[2] {
                1
            } else {
                2
            };
            left.colors.sort_unstable_by_key(|&bin| {
                let [r, g, b] = components(bin);
                match axis {
                    0 => [r, g, b],
                    1 => [g, r, b],
                    _ => [b, g, r],
                }
            });
            let mut count = 0;
            let split = left
                .colors
                .iter()
                .position(|&bin| {
                    count += histogram[bin as usize];
                    count as u64 * 2 >= left.population as u64
                })?
                .min(left.colors.len() - 2);
            let right = left.colors.split_off(split + 1);
            push_box(&mut queue, ColorBox::new(right, histogram));
            push_box(&mut queue, ColorBox::new(left.colors, histogram));
        }
        let mut palette = Vec::with_capacity(queue.len());
        while let Some(color_box) = pop_box(&mut queue) {
            palette.push(color_box.average(histogram));
        }
        palette
    };
    let select = |filter: bool| {
        let mut best = None;
        for &(color, count) in &palette {
            if (!filter || allowed(color)) && best.is_none_or(|(_, population)| count > population)
            {
                best = Some((color, count));
            }
        }
        best.map(|(color, _)| color)
    };
    select(true).or_else(|| select(false))
}

struct ColorBox {
    colors: Vec<u16>,
    extent: [u8; 3],
    volume: u32,
    population: u32,
}

impl ColorBox {
    fn new(colors: Vec<u16>, histogram: &[u32]) -> Self {
        let mut min = [31; 3];
        let mut max = [0; 3];
        let mut population = 0;
        for &bin in &colors {
            population += histogram[bin as usize];
            for (axis, channel) in components(bin).into_iter().enumerate() {
                min[axis] = min[axis].min(channel);
                max[axis] = max[axis].max(channel);
            }
        }
        let extent = std::array::from_fn(|axis| max[axis] - min[axis]);
        let volume = extent.iter().map(|&side| side as u32 + 1).product();
        Self {
            colors,
            extent,
            volume,
            population,
        }
    }

    fn average(self, histogram: &[u32]) -> ([u8; 3], u32) {
        let mut sum = [0u64; 3];
        for bin in self.colors {
            for (axis, channel) in components(bin).into_iter().enumerate() {
                sum[axis] += channel as u64 * histogram[bin as usize] as u64;
            }
        }
        (
            sum.map(|sum| ((sum as f64 / self.population as f64).round() as u8) * 8),
            self.population,
        )
    }
}

// 小型体积堆保留原版相等体积时的出队顺序；stdlib 堆的并列顺序会改变最终分盒。
fn push_box(queue: &mut Vec<ColorBox>, color_box: ColorBox) {
    queue.push(color_box);
    let mut index = queue.len() - 1;
    while index > 0 {
        let parent = (index - 1) / 2;
        if queue[index].volume <= queue[parent].volume {
            break;
        }
        queue.swap(index, parent);
        index = parent;
    }
}

fn pop_box(queue: &mut Vec<ColorBox>) -> Option<ColorBox> {
    if queue.is_empty() {
        return None;
    }
    let first = queue.swap_remove(0);
    let mut index = 0;
    while index * 2 + 1 < queue.len() {
        let mut child = index * 2 + 1;
        if child + 1 < queue.len() && queue[child + 1].volume > queue[child].volume {
            child += 1;
        }
        if queue[child].volume <= queue[index].volume {
            break;
        }
        queue.swap(index, child);
        index = child;
    }
    Some(first)
}

#[cfg(test)]
mod tests {
    use super::{cover_color, dominant};
    fn histogram(pixels: &[[u8; 3]]) -> Vec<u32> {
        let mut counts = vec![0; 32768];
        for &[r, g, b] in pixels {
            counts[((r as usize >> 3) << 10) | ((g as usize >> 3) << 5) | (b as usize >> 3)] += 1;
        }
        counts
    }
    #[test]
    fn matches_official_quantizer_oracles() {
        assert_eq!(dominant(&histogram(&[[255, 0, 0]]), 16), Some([248, 0, 0]));
        assert_eq!(
            dominant(
                &histogram(&[[0, 0, 248], [0, 248, 0], [248, 0, 0], [248, 248, 0]]),
                2
            ),
            Some([0, 128, 128])
        );
        assert_eq!(
            dominant(
                &histogram(&[[64, 32, 16], [72, 32, 16], [80, 32, 16], [248, 32, 16]]),
                1
            ),
            Some([120, 32, 16])
        );
        let mut pixels = vec![[64, 16, 16], [72, 16, 16], [80, 16, 16]];
        pixels.extend([[248, 16, 16]; 8]);
        assert_eq!(dominant(&histogram(&pixels), 2), Some([248, 16, 16]));
        let mut cluster = Vec::new();
        for r in 10..=13 {
            for g in 2..=5 {
                for b in 2..=5 {
                    cluster.push([r * 8, g * 8, b * 8]);
                }
            }
        }
        cluster.extend([[248, 0, 0]; 5]);
        assert_eq!(dominant(&histogram(&cluster), 16), Some([88, 32, 40]));
    }
    #[test]
    fn filters_correctly_and_retains_gray_and_monochrome_fallbacks() {
        assert_eq!(
            dominant(&histogram(&[[0, 0, 0], [255, 0, 0], [255, 255, 255]]), 16),
            Some([248, 0, 0])
        );
        assert_eq!(
            dominant(&histogram(&[[128, 128, 128]; 3]), 16),
            Some([128; 3])
        );
        assert_eq!(dominant(&histogram(&[[255, 255, 255]]), 16), Some([248; 3]));
        assert_eq!(dominant(&histogram(&[[0, 0, 0]]), 16), Some([0; 3]));
        assert!(cover_color(&[0, 0, 255, 0], 1, 1).is_none());
        assert!(cover_color(&[], 1, 1).is_none());
    }
    #[test]
    fn resized_bgra_keeps_aspect_ratio_and_ignores_hidden_rgb() {
        let cover = [0, 0, 255, 255].repeat(100 * 200);
        assert_eq!(cover_color(&cover, 100, 200).unwrap().h, 0.);
        let transparent = [0, 255, 0, 0].repeat(100 * 100);
        assert!(cover_color(&transparent, 100, 100).is_none());
    }
}
