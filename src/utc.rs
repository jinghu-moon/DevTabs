//! Gregorian dates for UTC day buckets. Shared by file info and activity charts.
pub(crate) fn date(days: u64) -> (i64, u32, u32) {
    let z = days as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month as u32, day as u32)
}

pub(crate) fn label(days: u64) -> String {
    let (year, month, day) = date(days);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn epoch_leap_day_and_year_boundary_are_exact() {
        assert_eq!(label(0), "1970-01-01");
        assert_eq!(label(19_782), "2024-02-29");
        assert_eq!(label(20_454), "2026-01-01");
        assert_eq!(label(20_769), "2026-11-12");
    }
}
