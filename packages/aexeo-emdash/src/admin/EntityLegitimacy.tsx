// Entity legitimacy pillar admin page.
//
// Layer 4 of the four-layer GEO model: does the brand exist strongly
// enough in the broader information ecosystem to be selected at all?
// This is the layer Aexeo deliberately *surfaces* but does not *fix* —
// the on-page tools can't generate entity legitimacy from nothing.
//
// What lives here:
//   - The truth-manifest authoring flow (folded in from the previous
//     /facts page; the prompt-and-validate UI is unchanged).
//   - FACTS00x findings (manifest missing / shape invalid / disagrees
//     with on-page schema.org) — same as any other layer's findings.
//   - Phase 3 will add the external entity-presence diagnostic
//     (Wikipedia, Wikidata, GitHub, Common Crawl, Domain age,
//     schema.org cross-check) here. For now this page just authors and
//     audits; presence-fetch is the next release.

import * as React from "react";
import { Facts } from "./Facts.js";
import { PillarView } from "./PillarView.js";

export function EntityLegitimacy(): React.JSX.Element {
  return (
    <PillarView
      layer="entity_legitimacy"
      extraSlot={
        <section className="rounded border border-kumo-line p-3">
          <h2 className="text-sm font-medium">Truth manifest</h2>
          <p className="mt-1 text-xs text-kumo-subtle">
            The structured assertion of who the organization is, what the
            products are, and the terminology to use/avoid. This is the
            primary entity-legitimacy signal Aexeo can write — the rest
            comes from third-party presence Aexeo can only observe.
          </p>
          <div className="mt-3">
            <Facts />
          </div>
        </section>
      }
    />
  );
}
