<!-- SPDX-License-Identifier: GPL-2.0-only -->

# Reveille website

The landing page is a dependency-free static site. Open `index.html` directly or serve this
directory with any static file server.

The product preview is a captured application screenshot. Its server figures are a past snapshot,
not a live server list.

No version is written into the page. Release buttons link to `releases/latest`, which is correct
whatever the current release is, and `script.js` asks the GitHub API for the latest release to fill
in the version, the installer size and a direct asset link. When that request fails — offline, or
rate-limited — the page keeps the version-free wording rather than naming a release it cannot
confirm.

## Deployment

`.github/workflows/pages.yml` publishes this directory after website changes reach `main`. It can
also be run manually from the Actions tab. Configure **Settings → Pages → Build and deployment →
Source** to **GitHub Actions** once for the repository; the workflow then owns subsequent
deployments.
