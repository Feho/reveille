<!-- SPDX-License-Identifier: GPL-2.0-only -->

# Reveille website

The landing page is a dependency-free static site. Open `index.html` directly or serve this
directory with any static file server.

The product preview is a captured application screenshot. Its server figures are a past snapshot,
not a live server list. Release buttons point to the latest published GitHub release.

## Deployment

`.github/workflows/pages.yml` publishes this directory after website changes reach `main`. It can
also be run manually from the Actions tab. Configure **Settings → Pages → Build and deployment →
Source** to **GitHub Actions** once for the repository; the workflow then owns subsequent
deployments.
