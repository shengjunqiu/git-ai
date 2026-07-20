# Chart.js vendor record

- Upstream package: `chart.js`
- Version: `4.4.7`
- Source artifact: official npm package `chart.js@4.4.7`
- Vendored file: `dist/chart.umd.js`
- SHA256: `2812cb8825fdc57469eb2f7bb055e9429244e599920511ee477e828499b632cb`
- License: MIT; the unmodified upstream text is stored in `LICENSE.md`.

The Dashboard loads this file only from
`/static/assets/vendor/chart.js/chart.umd.js`, so charts do not require network
access to a CDN.

## Upgrade

1. Review the target Chart.js release notes and security advisories.
2. Download the exact package with
   `npm pack chart.js@<version> --registry=https://registry.npmjs.org`.
3. Extract `dist/chart.umd.js` and `LICENSE.md` into this directory.
4. Update the version and SHA256 above, then verify the hash with
   `shasum -a 256 chart.umd.js`.
5. Run `node --check ../../../dashboard.js` and the enterprise server tests.
6. Start the server with external network access disabled and verify all four
   Dashboard charts render without external requests.
