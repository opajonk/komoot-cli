use anyhow::Result;

use crate::api::{KomootApi, TOUR_PLANNED, TOUR_RECORDED, TourEntry};
use crate::filter::{Filters, tour_matches_filters};

fn tour_type_label(tour_type: &str) -> &str {
    match tour_type {
        TOUR_PLANNED => "planned",
        TOUR_RECORDED => "recorded",
        _ => tour_type,
    }
}

fn escape_markdown_cell(value: &str) -> String {
    value.replace('\n', " ").replace('|', "\\|")
}

fn render_markdown_table(tours: &[TourEntry]) -> String {
    use std::fmt::Write as _;

    let mut output =
        String::from("| ID | Name | Type | Status | Date |\n| --- | --- | --- | --- | --- |\n");
    for tour in tours {
        writeln!(
            &mut output,
            "| {} | {} | {} | {} | {} |",
            escape_markdown_cell(&tour.id),
            escape_markdown_cell(&tour.name),
            escape_markdown_cell(tour_type_label(&tour.tour_type)),
            escape_markdown_cell(&tour.status),
            escape_markdown_cell(&tour.date)
        )
        .expect("writing to String should not fail");
    }
    output
}

pub fn list_with_client(client: &dyn KomootApi, filters: &Filters) -> Result<String> {
    let tours = client.fetch_all_tours()?;
    let filtered_tours: Vec<_> = tours
        .into_iter()
        .filter(|tour| tour_matches_filters(tour, filters))
        .collect();
    Ok(render_markdown_table(&filtered_tours))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_helpers::{MockKomootClient, make_tour};
    use crate::api::{TOUR_PLANNED, TOUR_RECORDED};

    #[test]
    fn list_renders_markdown_table() {
        let tours = vec![
            make_tour(
                "1",
                "Sunday Ride",
                TOUR_RECORDED,
                "public",
                "2024-01-01T00:00:00Z",
            ),
            make_tour(
                "2",
                "Plan | Adventure",
                TOUR_PLANNED,
                "friends",
                "2024-02-01T00:00:00Z",
            ),
        ];
        let client = MockKomootClient {
            username: "user".to_string(),
            tours,
            gpx_by_id: Default::default(),
        };

        let table = list_with_client(&client, &Filters::default()).expect("list");
        assert_eq!(
            table,
            "| ID | Name | Type | Status | Date |\n\
| --- | --- | --- | --- | --- |\n\
| 1 | Sunday Ride | recorded | public | 2024-01-01T00:00:00Z |\n\
| 2 | Plan \\| Adventure | planned | friends | 2024-02-01T00:00:00Z |\n"
        );
    }

    #[test]
    fn list_applies_filters_like_export() {
        let tours = vec![
            make_tour(
                "10",
                "Recorded Public",
                TOUR_RECORDED,
                "public",
                "2024-03-01T00:00:00Z",
            ),
            make_tour(
                "11",
                "Recorded Private",
                TOUR_RECORDED,
                "private",
                "2024-03-02T00:00:00Z",
            ),
            make_tour(
                "12",
                "Planned Public",
                TOUR_PLANNED,
                "public",
                "2024-03-03T00:00:00Z",
            ),
        ];
        let client = MockKomootClient {
            username: "user".to_string(),
            tours,
            gpx_by_id: Default::default(),
        };
        let filters = crate::filter::build_filters(
            None,
            None,
            &["public".to_string()],
            &["recorded".to_string()],
        )
        .expect("filters");

        let table = list_with_client(&client, &filters).expect("list");
        assert_eq!(
            table,
            "| ID | Name | Type | Status | Date |\n\
| --- | --- | --- | --- | --- |\n\
| 10 | Recorded Public | recorded | public | 2024-03-01T00:00:00Z |\n"
        );
    }
}
