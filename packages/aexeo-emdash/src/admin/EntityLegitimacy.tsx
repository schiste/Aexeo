// Entity legitimacy pillar admin page.
//
// Layer 4 of the four-layer GEO model: does the brand exist strongly
// enough in the broader information ecosystem to be selected at all?
// This is the layer Aexeo deliberately *surfaces* but does not *fix* —
// the on-page tools can't generate entity legitimacy from nothing.
//
// What lives here, in editorial order:
//   1. Truth manifest authoring (folded in from the previous /facts
//      page; the prompt-and-validate UI is unchanged).
//   2. Public web presence — Phase 3's external diagnostic. Five
//      free APIs query the configured entity and surface what's
//      out there: Wikipedia, Wikidata, GitHub, RDAP (domain age),
//      Common Crawl. No scoring; the editor verifies via the
//      "Open" links per source.
//   3. FACTS00x findings (manifest missing / shape invalid /
//      disagrees with on-page schema.org) — rendered by PillarView.

import * as React from "react";
import { EntityPresence } from "./EntityPresence.js";
import { Facts } from "./Facts.js";
import { PillarView } from "./PillarView.js";

export function EntityLegitimacy(): React.JSX.Element {
  return (
    <PillarView
      layer="entity_legitimacy"
      extraSlot={
        <div className="space-y-3">
          <section className="rounded border border-kumo-line p-3">
            <h2 className="text-sm font-medium">Truth manifest</h2>
            <p className="mt-1 text-xs text-kumo-subtle">
              The structured assertion of who the organization is, what
              the products are, and the terminology to use/avoid. This
              is the primary entity-legitimacy signal Aexeo can write —
              the rest comes from third-party presence Aexeo can only
              observe.
            </p>
            <div className="mt-3">
              <Facts />
            </div>
          </section>
          <section className="rounded border border-kumo-line p-3">
            <EntityPresence />
          </section>
        </div>
      }
    />
  );
}
