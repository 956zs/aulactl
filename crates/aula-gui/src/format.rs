//! Value formatting shared by the pages: travel in mm.

/// `raw` trigger units as mm with two decimals ("1.00"). `unit_mm` is `TravelLimits::unit_mm`.
pub fn mm(raw: f64, unit_mm: f64) -> String {
    let v = raw * unit_mm;
    if !v.is_finite() || unit_mm <= 0.0 {
        return "-".into();
    }
    // Avoid "-0.00".
    format!("{:.2}", if v.abs() < 0.005 { 0.0 } else { v })
}

/// The raw value nearest to `mm`, clamped to `min..=max`. None when the input is not a number.
pub fn raw_from_mm(mm: f64, unit_mm: f64, min: u16, max: u16) -> Option<u16> {
    if !mm.is_finite() || !unit_mm.is_finite() || unit_mm <= 0.0 || min > max {
        return None;
    }
    let raw = (mm / unit_mm).round().clamp(f64::from(min), f64::from(max));
    Some(raw as u16)
}

/// Parses user text like "1.5", "1,50" or "1.5 mm" into mm.
pub fn parse_mm(text: &str) -> Option<f64> {
    let t = text.trim().trim_end_matches("mm").trim().replace(',', ".");
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mm_two_decimals() {
        assert_eq!(mm(100.0, 0.01), "1.00");
        assert_eq!(mm(8.0, 0.01), "0.08");
        assert_eq!(mm(340.0, 0.01), "3.40");
        assert_eq!(mm(126.0, 0.001), "0.13");
    }

    #[test]
    fn mm_edge_cases() {
        assert_eq!(mm(0.0, 0.01), "0.00");
        assert_eq!(mm(-0.1, 0.01), "0.00");
        assert_eq!(mm(f64::NAN, 0.01), "-");
        assert_eq!(mm(1.0, 0.0), "-");
        assert_eq!(mm(1.0, -1.0), "-");
    }

    #[test]
    fn raw_from_mm_rounds_and_clamps() {
        assert_eq!(raw_from_mm(1.0, 0.01, 8, 340), Some(100));
        assert_eq!(raw_from_mm(1.004, 0.01, 8, 340), Some(100));
        assert_eq!(raw_from_mm(1.006, 0.01, 8, 340), Some(101));
        assert_eq!(raw_from_mm(0.0, 0.01, 8, 340), Some(8));
        assert_eq!(raw_from_mm(9.0, 0.01, 8, 340), Some(340));
        assert_eq!(raw_from_mm(f64::INFINITY, 0.01, 8, 340), None);
        assert_eq!(raw_from_mm(1.0, 0.0, 8, 340), None);
        assert_eq!(raw_from_mm(1.0, 0.01, 9, 8), None);
    }

    #[test]
    fn parse_user_text() {
        assert_eq!(parse_mm("1.5"), Some(1.5));
        assert_eq!(parse_mm(" 1,50 mm"), Some(1.5));
        assert_eq!(parse_mm("2mm"), Some(2.0));
        assert_eq!(parse_mm(""), None);
        assert_eq!(parse_mm("mm"), None);
        assert_eq!(parse_mm("abc"), None);
        assert_eq!(parse_mm("-1"), None);
        assert_eq!(parse_mm("NaN"), None);
    }

}
