//! Characterization tests for the two `classify_page_kind` implementations.
//!
//! There are two of them, and they are not the same function:
//!
//! - `site::classify_page_kind(relative, route)` attaches a kind to every
//!   parsed `Page`. Rules branch on it — `PageKind::Legal` is what exempts a
//!   legal page from A11Y005, for instance.
//! - `generate::classify_page_kind(route)` sections the generated machine
//!   artifacts (`llms.txt` and friends) under human headings.
//!
//! They disagree on 12 of the 21 routes in `DIVERGENT_ROUTES` below. That is
//! currently load-bearing rather than accidental — the generator's taxonomy
//! is deliberately finer-grained (`Feature`, `Skill`, `Category`,
//! `Maintainer`), and collapsing it onto the site's would change published
//! artifact output. But the disagreement is invisible: nothing recorded it,
//! and the two functions can be edited independently without either one
//! noticing.
//!
//! So this test does not assert the two *agree* — it asserts they *keep
//! disagreeing in exactly the documented places*. If someone changes either
//! classifier, this fails and forces the decision to be revisited rather than
//! discovered later in a diff of generated artifacts.
//!
//! What would resolve it properly is one shared classifier with a
//! per-consumer mapping, but that is a design change, not a bug fix, and it
//! needs a decision on which taxonomy the published artifacts should use.

#[cfg(test)]
mod page_kind_classification {
    use crate::generate;
    use crate::site;

    /// Routes where the two classifiers return different kinds, with the
    /// kind each one picks. If any cell in this table changes, one of the two
    /// functions moved and the divergence is no longer what is documented.
    const DIVERGENT_ROUTES: &[(&str, &str, &str)] = &[
        // The generator splits the site's `Detail` into four finer kinds so
        // `llms.txt` can group them, but the site has no such notion.
        ("features/x", "Detail", "Feature"),
        ("skill/seo", "Detail", "Skill"),
        ("admin/panel", "Admin", "Detail"),
        // The generator splits the site's `Listing` three ways.
        ("category/tools", "Listing", "Category"),
        ("maintainer/aeptus", "Listing", "Maintainer"),
        // Search is its own kind for the rules engine; to the generator it is
        // just another utility page.
        ("search", "Search", "Utility"),
        // `404` is the sharpest disagreement. The generator compensates by
        // filtering `404` out of the artifact set before classifying, so the
        // `Other` never reaches output.
        ("404", "NotFound", "Other"),
        // `feed` is a first-class kind for the rules engine — an RSS feed has
        // no `<h1>` and should not be told to add one — but the generator has
        // no feed section and files it under `Other`.
        ("feed", "Feed", "Other"),
        // The legal cases are the ones worth arguing about. The generator uses
        // `route.contains("legal")`, so it catches a legal page filed at a
        // nested or suffixed URL. The site requires an exact match or a
        // `legal/` prefix, so `legal-notice` and `company/legal/terms` both
        // fall through to `Generic` — which means the A11Y005 `<main>`
        // exemption documented on that rule does not actually apply to them.
        // Broadening the site's match would suppress findings on real sites,
        // and narrowing the generator's would drop a legal section from
        // `llms.txt`, so neither is changed here.
        ("legal-notice", "Generic", "Legal"),
        ("company/legal/terms", "Generic", "Legal"),
        // The site's `Generic` is the generator's `Detail` (any nested route)
        // or `Other` (a flat, unmatched route).
        ("blog/post", "Generic", "Detail"),
        ("about", "Generic", "Other"),
    ];

    /// Routes both classifiers agree on. These are the cases where the two
    /// implementations are interchangeable, and they are worth pinning so a
    /// change that breaks agreement shows up as a *new* divergence rather
    /// than quietly reducing this list.
    const SHARED_ROUTES: &[(&str, &str)] = &[
        ("", "Home"),
        ("legal", "Legal"),
        ("privacy", "Legal"),
        ("docs/getting-started", "Docs"),
        ("guide/setup", "Docs"),
        ("skills", "Listing"),
        ("maintainers", "Listing"),
        ("submit", "Utility"),
    ];

    #[test]
    fn the_documented_divergences_are_accurate() {
        for (route, site_kind, generate_kind) in DIVERGENT_ROUTES {
            assert_eq!(
                site::classify_page_kind_for_test(route),
                *site_kind,
                "site classifier moved for {route:?}"
            );
            assert_eq!(
                generate::classify_page_kind_for_test(route),
                *generate_kind,
                "generator classifier moved for {route:?}"
            );
        }
    }

    #[test]
    fn the_documented_agreements_are_accurate() {
        for (route, kind) in SHARED_ROUTES {
            assert_eq!(
                site::classify_page_kind_for_test(route),
                *kind,
                "site classifier moved for {route:?}"
            );
            assert_eq!(
                generate::classify_page_kind_for_test(route),
                *kind,
                "generator classifier moved for {route:?}"
            );
        }
    }

    /// The two tables must not overlap and must not drift apart: every route
    /// in the divergence table has to actually differ, and every route in the
    /// agreement table has to actually agree. Without this, a change to one
    /// classifier could leave a stale row in the table that still passed.
    #[test]
    fn the_tables_do_not_go_stale() {
        for (route, site_kind, generate_kind) in DIVERGENT_ROUTES {
            assert_ne!(
                site_kind, generate_kind,
                "{route:?} is listed as divergent but both say {site_kind}"
            );
        }
        for (route, kind) in SHARED_ROUTES {
            let site_kind = site::classify_page_kind_for_test(route);
            assert_eq!(
                site_kind, *kind,
                "{route:?} is listed as agreeing but the site says {site_kind}"
            );
        }

        // No route may appear in both tables.
        for (route, _, _) in DIVERGENT_ROUTES {
            assert!(
                !SHARED_ROUTES.iter().any(|(shared, _)| shared == route),
                "{route:?} is in both tables"
            );
        }
    }

    /// The specific consequence of the legal divergence, asserted directly so
    /// the reason this table exists is not lost. A11Y005 skips legal pages
    /// because they are often thin templates with no `<main>`, and the skip is
    /// keyed on `PageKind::Legal`. A legal page at a nested URL is not
    /// recognised as legal, so it gets the finding anyway.
    #[test]
    fn a_nested_legal_route_does_not_receive_the_legal_exemption() {
        for route in ["legal-notice", "company/legal/terms"] {
            assert_eq!(
                site::classify_page_kind_for_test(route),
                "Generic",
                "{route:?} is the case the site's classifier does not catch"
            );
            assert_eq!(
                generate::classify_page_kind_for_test(route),
                "Legal",
                "the generator does treat {route:?} as legal"
            );
        }
    }
}
