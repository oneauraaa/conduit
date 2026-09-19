"use strict";

// Supplying a pkg config selects its enhanced SEA pipeline, which preserves
// Node's inspector support required by Playwright. The sidecar and the two
// Playwright metadata files are already embedded by build-runtime.cjs.
module.exports = { pkg: {} };
