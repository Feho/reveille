// SPDX-License-Identifier: GPL-2.0-only

use std::collections::BTreeMap;
use std::path::Path;

use reveille_core::bsp::Checksum;
use reveille_core::install::{IdentificationMethod, Product, identify};
use reveille_core::mapindex::{MapIndex, Provider};
use reveille_core::preflight::{MapStatus, Verdict, check};
use serde::Deserialize;

#[derive(Deserialize)]
struct ScanFixture {
    pk3_count: usize,
    maps_indexed: usize,
    maps_multi_provider: usize,
    keys_without_slash: usize,
    pk3dir_count: usize,
    known_checksums: BTreeMap<String, i32>,
    checksum_source_pak: String,
}

#[derive(Deserialize)]
struct ServerFixture {
    sv_maplist: String,
}

#[test]
fn real_asset_corpus_matches_frozen_expectations_when_available() {
    let root = Path::new("/home/feho/MOHAA");
    if !root.join("main").is_dir() {
        return;
    }

    let expected: ScanFixture = serde_json::from_str(include_str!("fixtures/install_scan.json"))
        .expect("valid frozen scan fixture");
    let installation = identify(root).expect("identify real asset corpus");
    assert_eq!(installation.products, vec![Product::AlliedAssault]);
    assert_eq!(
        installation.identification,
        IdentificationMethod::DataDirectoriesOnly
    );

    let index = MapIndex::scan(root.join("main")).expect("scan real asset corpus");
    let stats = index.stats();
    assert_eq!(stats.archives, expected.pk3_count);
    assert_eq!(stats.package_directories, expected.pk3dir_count);
    // This is a corpus measurement, not loose-file coverage: the fixture contains no loose BSP.
    // The synthetic `mirrors_package_and_loose_file_precedence` test exercises that path.
    assert_eq!(stats.loose_bsp_files, 0);
    assert_eq!(stats.skipped_entries, 0);
    assert_eq!(stats.maps, expected.maps_indexed);
    assert_eq!(stats.multi_provider_maps, expected.maps_multi_provider);
    assert_eq!(
        index
            .maps()
            .filter(|map| !map.name.as_str().contains('/'))
            .count(),
        expected.keys_without_slash
    );

    for (name, checksum) in expected.known_checksums {
        let map = index.get(&name).expect("known map is indexed");
        let provider = map.effective_provider().expect("known map has a provider");
        assert_eq!(provider.checksum(), Checksum::new(checksum));
        assert!(matches!(
            provider,
            Provider::Pk3 { archive, .. }
                if archive.file_name().is_some_and(|file| file == expected.checksum_source_pak.as_str())
        ));
    }
}

#[test]
fn frozen_tfc_rotation_has_seven_present_and_seven_absent_when_corpus_is_available() {
    let root = Path::new("/home/feho/MOHAA");
    if !root.join("main").is_dir() {
        return;
    }

    let server: ServerFixture = serde_json::from_str(include_str!("fixtures/server_tfc.json"))
        .expect("valid frozen server fixture");
    let index = MapIndex::scan(root.join("main")).expect("scan real asset corpus");
    let rotation = server.sv_maplist.split_whitespace().collect::<Vec<_>>();
    let report = check(&index, &rotation, None);

    assert_eq!(
        report.verdict,
        Verdict::ProblemsFound {
            absent: 7,
            checksum_mismatches: 0,
        }
    );
    assert_eq!(
        report
            .maps
            .iter()
            .filter(|map| matches!(map.status, MapStatus::Present { .. }))
            .count(),
        7
    );
}
