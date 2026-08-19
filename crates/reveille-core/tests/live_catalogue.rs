// SPDX-License-Identifier: GPL-2.0-only

//! Explicitly opt-in catalogue smoke check; default and CI runs never open a socket.

use std::time::Duration;

use reveille_core::content::MohDbClient;

#[tokio::test]
#[ignore = "requires the live third-party moh-db catalogue"]
async fn live_catalogue_smoke_check_skips_when_unavailable() {
    let Ok(client) = MohDbClient::new(Duration::from_secs(15)) else {
        return;
    };
    let Ok(page) = client.lookup("obj/obj_howitzer").await else {
        return;
    };
    assert!(page.total_elements >= 1);
    assert!(
        page.content
            .iter()
            .any(|candidate| candidate.map_key.as_str() == "obj/obj_howitzer")
    );
}
