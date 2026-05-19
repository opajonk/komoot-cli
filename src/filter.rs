use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use std::collections::HashSet;

use crate::api::{TOUR_PLANNED, TOUR_RECORDED, TourEntry};

#[derive(Debug, Default)]
pub struct Filters {
    pub from_date: Option<NaiveDate>,
    pub to_date: Option<NaiveDate>,
    pub statuses: Option<HashSet<String>>,
    /// Values are "planned" or "recorded"
    pub types: Option<HashSet<String>>,
}

pub fn tour_matches_filters(tour: &TourEntry, filters: &Filters) -> bool {
    if let Some(ref types) = filters.types {
        let label = match tour.tour_type.as_str() {
            TOUR_PLANNED => "planned",
            TOUR_RECORDED => "recorded",
            _ => "",
        };
        if !types.contains(label) {
            return false;
        }
    }
    if let Some(ref statuses) = filters.statuses
        && !statuses.contains(&tour.status)
    {
        return false;
    }
    if filters.from_date.is_some() || filters.to_date.is_some() {
        // Parse tour date directly from RFC 3339, falling back to 2000-01-01 for
        // unparseable dates so that date-range filters still work predictably.
        let tour_date = DateTime::parse_from_rfc3339(&tour.date)
            .map(|dt| dt.with_timezone(&Utc).date_naive())
            .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2000, 1, 1).unwrap());
        if let Some(from) = filters.from_date
            && tour_date < from
        {
            return false;
        }
        if let Some(to) = filters.to_date
            && tour_date > to
        {
            return false;
        }
    }
    true
}

/// Validates and builds a [`Filters`] value from raw CLI inputs.
///
/// All parameters correspond directly to CLI flags:
/// - `from_date` / `to_date`: optional `YYYY-MM-DD` strings
/// - `statuses`: slice of status values (each must be `public`, `friends`, or `private`)
/// - `tour_types`: slice of type values (each must be `planned` or `recorded`)
pub fn build_filters(
    from_date: Option<&str>,
    to_date: Option<&str>,
    statuses: &[String],
    tour_types: &[String],
) -> Result<Filters> {
    let from_date = from_date
        .map(|s| {
            NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .with_context(|| format!("invalid --from-date '{s}': expected YYYY-MM-DD"))
        })
        .transpose()?;
    let to_date = to_date
        .map(|s| {
            NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .with_context(|| format!("invalid --to-date '{s}': expected YYYY-MM-DD"))
        })
        .transpose()?;
    for s in statuses {
        if !["public", "friends", "private"].contains(&s.as_str()) {
            anyhow::bail!("invalid --status value '{s}': must be one of public, friends, private");
        }
    }
    for t in tour_types {
        if !["planned", "recorded"].contains(&t.as_str()) {
            anyhow::bail!("invalid --type value '{t}': must be one of planned, recorded");
        }
    }
    Ok(Filters {
        from_date,
        to_date,
        statuses: if statuses.is_empty() {
            None
        } else {
            Some(statuses.iter().cloned().collect())
        },
        types: if tour_types.is_empty() {
            None
        } else {
            Some(tour_types.iter().cloned().collect())
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_filters_rejects_invalid_date() {
        assert!(build_filters(Some("not-a-date"), None, &[], &[]).is_err());
    }

    #[test]
    fn build_filters_rejects_invalid_status() {
        assert!(build_filters(None, None, &["unknown".to_string()], &[]).is_err());
    }

    #[test]
    fn build_filters_rejects_invalid_type() {
        assert!(build_filters(None, None, &[], &["cycling".to_string()]).is_err());
    }

    #[test]
    fn build_filters_accepts_valid_inputs() {
        let statuses = vec!["public".to_string(), "friends".to_string()];
        let types = vec!["recorded".to_string()];
        let filters = build_filters(Some("2024-01-01"), Some("2024-12-31"), &statuses, &types)
            .expect("valid filters");
        assert_eq!(
            filters.from_date,
            Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        );
        assert_eq!(
            filters.to_date,
            Some(NaiveDate::from_ymd_opt(2024, 12, 31).unwrap())
        );
        assert_eq!(
            filters.statuses,
            Some(["public".to_string(), "friends".to_string()].into())
        );
        assert_eq!(filters.types, Some(["recorded".to_string()].into()));
    }
}
