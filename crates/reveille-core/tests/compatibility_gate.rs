// SPDX-License-Identifier: GPL-2.0-only

//! Hermetic coverage of the four player-visible compatibility states.

use reveille_core::content::{
    CatalogueCandidate, CatalogueResolution, CatalogueResolutionPass, FileSize, ResolutionOutcome,
    WantedMap,
};
use reveille_core::join::{CompatibilityState, classify};
use reveille_core::mapindex::MapKey;
use reveille_core::preflight::{Report, Verdict};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    rotation_published: bool,
    absent: usize,
    checksum_mismatches: usize,
    resolution_outcomes: Vec<String>,
    expected: String,
}

#[derive(Deserialize)]
struct TfcResolutionFixture {
    cases: Vec<TfcResolutionCase>,
}

#[derive(Deserialize)]
struct TfcResolutionCase {
    wanted: String,
    outcome: String,
}

#[derive(Deserialize)]
struct TfcServerFixture {
    sv_maplist: String,
}

#[test]
fn frozen_cases_cover_exactly_the_four_gate_states() {
    let fixture: Fixture = serde_json::from_str(include_str!("fixtures/compatibility_states.json"))
        .expect("valid compatibility fixture");

    for case in fixture.cases {
        let report = Report {
            verdict: if case.absent == 0 && case.checksum_mismatches == 0 {
                Verdict::Compatible
            } else {
                Verdict::ProblemsFound {
                    absent: case.absent,
                    checksum_mismatches: case.checksum_mismatches,
                }
            },
            maps: Vec::new(),
        };
        let pass = CatalogueResolutionPass {
            resolutions: case
                .resolution_outcomes
                .iter()
                .enumerate()
                .map(|(index, outcome)| resolution(index, outcome))
                .collect(),
            non_results: Vec::new(),
        };
        let state = classify(
            case.rotation_published.then_some(&report),
            (!pass.resolutions.is_empty()).then_some(&pass),
        );
        let actual = match &state {
            CompatibilityState::Compatible => "compatible",
            CompatibilityState::NeedsMaps { .. } => "needs_maps",
            CompatibilityState::NoSource { .. } => "no_source",
            CompatibilityState::CantTell => "cant_tell",
        };
        assert_eq!(actual, case.expected, "{}", case.name);

        if case.name == "TFC mixed shopping list" {
            let CompatibilityState::NeedsMaps {
                count,
                shopping_list: Some(shopping_list),
            } = state
            else {
                panic!("TFC must retain its shopping list");
            };
            assert_eq!(count.get(), 7);
            assert_eq!(shopping_list.resolutions.len(), 7);
        }
    }
}

#[test]
fn frozen_tfc_rotation_is_needs_seven_with_its_mixed_shopping_list_attached() {
    let server: TfcServerFixture =
        serde_json::from_str(include_str!("fixtures/server_tfc.json")).expect("valid TFC server");
    let source: TfcResolutionFixture =
        serde_json::from_str(include_str!("fixtures/mohdb_resolution.json"))
            .expect("valid TFC resolutions");
    let rotation = server.sv_maplist.split_whitespace().collect::<Vec<_>>();
    assert_eq!(rotation.len(), 14);
    assert!(
        source
            .cases
            .iter()
            .all(|case| rotation.contains(&case.wanted.as_str()))
    );

    let report = Report {
        verdict: Verdict::ProblemsFound {
            absent: source.cases.len(),
            checksum_mismatches: 0,
        },
        maps: Vec::new(),
    };
    let pass = CatalogueResolutionPass {
        resolutions: source
            .cases
            .iter()
            .enumerate()
            .map(|(index, case)| resolution(index, &case.outcome))
            .collect(),
        non_results: Vec::new(),
    };

    let CompatibilityState::NeedsMaps {
        count,
        shopping_list: Some(attached),
    } = classify(Some(&report), Some(&pass))
    else {
        panic!("TFC must need content and retain its shopping list");
    };
    assert_eq!(count.get(), 7);
    assert_eq!(attached, pass);
}

fn resolution(index: usize, outcome: &str) -> CatalogueResolution {
    let wanted = WantedMap::new(format!("obj/frozen_{index}")).expect("valid wanted map");
    CatalogueResolution {
        wanted,
        hits: usize::from(outcome != "no_source"),
        outcome: match outcome {
            "exact" => ResolutionOutcome::Exact {
                name_match: candidate(index),
                alternatives: Vec::new(),
            },
            "choice_required" => ResolutionOutcome::ChoiceRequired {
                choices: vec![candidate(index)],
            },
            "no_source" => ResolutionOutcome::NoSource,
            unexpected => panic!("unknown fixture outcome {unexpected}"),
        },
    }
}

fn candidate(index: usize) -> CatalogueCandidate {
    let map_name = format!("obj/frozen_{index}");
    CatalogueCandidate {
        id: index as u64,
        map_key: MapKey::new(&map_name).expect("valid map key"),
        map_name,
        filename: format!("frozen_{index}.pk3"),
        file_size: FileSize::new(1_000),
        map_file_tested: true,
        downloads: 1,
        download_url: format!("https://example.invalid/frozen_{index}.pk3"),
    }
}
