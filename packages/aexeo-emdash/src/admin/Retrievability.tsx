// Retrievability pillar admin page.
//
// Layer 1 of the four-layer GEO model: can the engine find the page at
// all? Robots, sitemap, internal links, machine-readable surfaces.
// Findings whose rule's primary layer is `retrievability` land here.

import * as React from "react";
import { PillarView } from "./PillarView.js";

export function Retrievability(): React.JSX.Element {
  return <PillarView layer="retrievability" />;
}
