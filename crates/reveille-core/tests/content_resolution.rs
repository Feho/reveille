// SPDX-License-Identifier: GPL-2.0-only

//! Hermetic TFC rotation acceptance test. No test in this file opens a socket.

use reveille_core::content::{
    CatalogueCandidate, FileSize, ResolutionOutcome, WantedMap, resolve_candidates,
};
use reveille_core::mapindex::MapKey;
use serde::Deserialize;
use serde_json::Number;

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
    exact_total_mb: Number,
    with_choices_total_mb: Number,
}

#[derive(Deserialize)]
struct Case {
    wanted: String,
    hits: usize,
    outcome: String,
    catalogue_map: Option<String>,
    filename: Option<String>,
    size_mb: Option<Number>,
}

#[derive(Deserialize)]
struct ServerFixture {
    sv_maplist: String,
}

#[test]
fn frozen_tfc_cases_resolve_to_four_exact_two_choices_and_one_dead_end() {
    let fixture: Fixture = serde_json::from_str(include_str!("fixtures/mohdb_resolution.json"))
        .expect("valid frozen resolution fixture");
    let server: ServerFixture = serde_json::from_str(include_str!("fixtures/server_tfc.json"))
        .expect("valid frozen server fixture");
    let rotation = server.sv_maplist.split_whitespace().collect::<Vec<_>>();
    let mut exact = 0;
    let mut choice_required = 0;
    let mut no_source = 0;
    let mut exact_bytes = 0_u64;
    let mut choice_bytes = 0_u64;

    for (index, case) in fixture.cases.iter().enumerate() {
        assert!(rotation.contains(&case.wanted.as_str()));
        let wanted = WantedMap::new(case.wanted.clone()).expect("valid wanted map");
        let mut candidates = case
            .catalogue_map
            .as_ref()
            .zip(case.filename.as_ref())
            .zip(case.size_mb.as_ref())
            .map(|((map_name, filename), size_mb)| {
                vec![candidate(
                    index as u64,
                    map_name,
                    filename,
                    megabyte_number_to_bytes(size_mb),
                )]
            })
            .unwrap_or_default();
        while candidates.len() < case.hits {
            candidates.push(candidate(
                10_000 + candidates.len() as u64,
                "obj/unrelated_catalogue_hit",
                &format!("unrelated-{}.pk3", candidates.len()),
                100_000,
            ));
        }

        let resolution = resolve_candidates(wanted, candidates);
        assert_eq!(resolution.hits, case.hits);
        match (&*case.outcome, resolution.outcome) {
            ("exact", ResolutionOutcome::Exact { name_match, .. }) => {
                exact += 1;
                exact_bytes += name_match.file_size.get();
            }
            ("choice_required", ResolutionOutcome::ChoiceRequired { choices }) => {
                choice_required += 1;
                choice_bytes += choices[0].file_size.get();
            }
            ("no_source", ResolutionOutcome::NoSource) => no_source += 1,
            (expected, actual) => panic!("expected {expected}, received {actual:?}"),
        }
    }

    assert_eq!((exact, choice_required, no_source), (4, 2, 1));
    assert_eq!(
        exact_bytes,
        megabyte_number_to_bytes(&fixture.exact_total_mb)
    );
    assert_eq!(
        exact_bytes + choice_bytes,
        megabyte_number_to_bytes(&fixture.with_choices_total_mb)
    );
}

fn candidate(id: u64, map_name: &str, filename: &str, file_size: u64) -> CatalogueCandidate {
    CatalogueCandidate {
        id,
        map_name: map_name.to_owned(),
        map_key: MapKey::new(map_name).expect("valid catalogue key"),
        filename: filename.to_owned(),
        file_size: FileSize::new(file_size),
        map_file_tested: true,
        downloads: 1,
        download_url: format!("https://example.invalid/{filename}"),
    }
}

fn megabyte_number_to_bytes(value: &Number) -> u64 {
    let text = value.to_string();
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let whole = whole.parse::<u64>().expect("fixture MB whole number");
    let fraction = format!("{fraction:0<6}");
    let fraction = fraction[..6]
        .parse::<u64>()
        .expect("fixture MB fractional number");
    whole * 1_000_000 + fraction
}
