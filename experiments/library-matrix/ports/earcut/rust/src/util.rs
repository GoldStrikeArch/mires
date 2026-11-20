//! Relative area deviation of a triangulation.

use crate::earcut::signed_area;

pub fn deviation(
    data: &[f64],
    hole_indices: Option<&[u32]>,
    dim: usize,
    triangles: &[u32],
) -> f64 {
    let holes = match hole_indices {
        Some(h) if !h.is_empty() => Some(h),
        _ => None,
    };
    let outer_len = match holes {
        Some(h) => (h[0] as usize) * dim,
        None => data.len(),
    };

    let mut polygon_area = signed_area(data, 0, outer_len, dim).abs();
    if let Some(h) = holes {
        let len = h.len();
        let mut i = 0;
        while i < len {
            let start = (h[i] as usize) * dim;
            let end = if i + 1 < len {
                (h[i + 1] as usize) * dim
            } else {
                data.len()
            };
            polygon_area -= signed_area(data, start, end, dim).abs();
            i += 1;
        }
    }

    let mut triangles_area = 0.0;
    let tlen = triangles.len();
    let mut i = 0;
    while i < tlen {
        let a = (triangles[i] as usize) * dim;
        let b = (triangles[i + 1] as usize) * dim;
        let c = (triangles[i + 2] as usize) * dim;
        triangles_area += ((data[a] - data[c]) * (data[b + 1] - data[a + 1])
            - (data[a] - data[b]) * (data[c + 1] - data[a + 1]))
            .abs();
        i += 3;
    }

    if triangles_area == 0.0 {
        let mut max_val = 0.0;
        let mut k = 0;
        let dlen = data.len();
        while k < dlen {
            let ax = data[k].abs();
            let ay = data[k + 1].abs();
            if ax > max_val {
                max_val = ax;
            }
            if ay > max_val {
                max_val = ay;
            }
            k += dim;
        }
        return if polygon_area.abs() <= ((dlen as f64) * max_val * max_val) * f64::EPSILON {
            0.0
        } else {
            1.0
        };
    }

    ((triangles_area - polygon_area) / polygon_area).abs()
}
