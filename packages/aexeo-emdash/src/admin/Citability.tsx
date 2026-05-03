// Citability pillar admin page.
//
// Layer 2 of the four-layer GEO model: once retrieved, does the page
// look worth citing? Structure, schema, evidence density, scannability.
// Most rules in the engine are citability-primary.

import * as React from "react";
import { PillarView } from "./PillarView.js";

export function Citability(): React.JSX.Element {
  return <PillarView layer="citability" />;
}
