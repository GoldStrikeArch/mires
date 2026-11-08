/// Counts `true` values.
pub fn count_truthy(values: &[bool]) -> usize {
    values.iter().filter(|&&value| value).count()
}

/// Rounds to a multiple using FingerprintJS's JavaScript rounding semantics.
pub fn round(value: f64, base: f64) -> f64 {
    if base.abs() >= 1.0 {
        js_round(value / base) * base
    } else {
        let counter_base = 1.0 / base;
        js_round(value * counter_base) / counter_base
    }
}

fn js_round(value: f64) -> f64 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    if value >= i64::MAX as f64 || value <= i64::MIN as f64 {
        return value;
    }
    let truncated = value as i64 as f64;
    let floor = if truncated > value {
        truncated - 1.0
    } else {
        truncated
    };
    let result = if value - floor < 0.5 {
        floor
    } else {
        floor + 1.0
    };
    if result == 0.0 && value.is_sign_negative() {
        -0.0
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_like_javascript() {
        assert_eq!(round(7.9, 1.0), 8.0);
        assert_eq!(round(29_847.23, 10.0), 29_850.0);
        assert_eq!(round(0.123_432_1, 0.0001), 0.1234);
        assert_eq!(round(-234.65, 2.0), -234.0);
        assert_eq!(round(-1.5, 1.0), -1.0);
    }
}
