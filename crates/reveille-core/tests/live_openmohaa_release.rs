// SPDX-License-Identifier: GPL-2.0-only

//! Explicitly opt-in release-metadata check; it never downloads or installs a release asset.

use std::time::Duration;

use reveille_core::platform::openmohaa::{OpenMohaaReleaseClient, ReleaseTarget};

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
    assert_eq!(package.digest.to_hex().len(), 64);
}

#[tokio::test]
#[ignore = "requires the live official GitHub Releases API"]
async fn live_latest_release_has_a_digest_bearing_archive_for_this_host() {
    let Ok(target) = ReleaseTarget::for_host() else {
        return;
    };
    let Ok(client) = OpenMohaaReleaseClient::new(Duration::from_secs(15)) else {
        return;
    };
    let Ok(package) = client.latest_release(target).await else {
        return;
    };
    assert_eq!(package.digest.to_hex().len(), 64);
    if matches!(target, ReleaseTarget::MacosArm64 | ReleaseTarget::MacosX64) {
        assert!(
            package
                .asset_name
                .ends_with("-macos-multiarch-arm64-x86_64.zip")
        );
    }
}
