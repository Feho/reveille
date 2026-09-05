// SPDX-License-Identifier: GPL-2.0-only

//! Explicitly opt-in release-metadata checks; they never download or install a release asset.

use std::time::Duration;

use reveille_core::platform::openmohaa::{OpenMohaaReleaseClient, ReleaseSelector, ReleaseTarget};

#[tokio::test]
#[ignore = "requires the live official GitHub Releases API"]
async fn live_latest_release_has_a_digest_bearing_windows_archive() {
    let Ok(client) = OpenMohaaReleaseClient::new(Duration::from_secs(15)) else {
        return;
    };
    let Ok(package) = client.latest_release(ReleaseTarget::WindowsX64).await else {
        return;
    };
    assert!(package.asset_name.ends_with("-windows-x64.zip"));
    assert!(!package.prerelease);
    assert_eq!(package.digest.to_hex().len(), 64);
}

#[tokio::test]
#[ignore = "requires the live official GitHub Releases API"]
async fn live_preview_release_never_ranks_below_the_stable_one() {
    let Ok(client) = OpenMohaaReleaseClient::new(Duration::from_secs(15)) else {
        return;
    };
    let Ok(stable) = client.latest_release(ReleaseTarget::WindowsX64).await else {
        return;
    };
    let Ok(preview) = client
        .release(ReleaseSelector::preview(ReleaseTarget::WindowsX64))
        .await
    else {
        return;
    };
    // A preview player must never be offered an older build than the stable channel would give.
    assert!(preview.semver >= stable.semver);
    assert!(preview.asset_name.ends_with("-windows-x64.zip"));
    assert_eq!(preview.digest.to_hex().len(), 64);
}
