# Aexeo : plan de migration vers les gates, presets et adapters

État : session broker ouverte ; phase 1 en cours.

## Objectif

Faire évoluer Aexeo vers un moteur agnostique composé de gates configurables, de presets portables et d'adapters, tout en conservant par défaut les usages SEO/GEO, CLI, crawl et EmDash existants. Les vérifications de sources et de faits viennent en dernier.

## Suivi

- [x] Mettre à niveau Aethyme et ouvrir une session broker.
- [x] Copier ce plan dans le worktree de session et maintenir son avancement au fil des livraisons.
- [ ] Phase 1 — caractériser les contrats et usages existants ; ajouter les tests de compatibilité nécessaires ; isoler les points de couplage et nettoyer sans modifier le comportement.
- [ ] Phase 2 — définir les contrats d'évaluation, capacités, scopes, gates et résultats ; construire le moteur et le preset SEO/GEO existant ; adapter les parcours site, crawl, CLI et EmDash/WASM ; vérifier la parité.
- [ ] Phase 3 — rendre les presets TOML composables et portables : héritage, surcharges, validation, sélection par contexte, inspection et versionnement ; fournir des presets initiaux.
- [ ] Phase 4 — implémenter les gates de contenu déterministes et l'adapter Markdown/stdin ; vérifier un même preset et une même gate sur Markdown et EmDash.
- [ ] Phase 5 — livrer le workflow agent : skill, découverte des presets, sorties structurées, correctifs sûrs et benchmarks de latence.
- [ ] Phase 6 — étendre selon les besoins ; envisager YAML, extensions personnalisées et évaluations sémantiques ; réserver la vérification de sources/faits pour la fin.
- [ ] Exécuter les validations pertinentes, contrôles du dépôt et vérifications natives/WASM ; documenter les changements ; livrer par le workflow broker et suivre la livraison autorisée.

## Progression détaillée

- Phase 1 : suite Rust complète, typage et 74 tests du plugin EmDash validés ; tests ajoutés pour la compatibilité des options de règles, sévérités et suppressions du pont EmDash.
- Phase 1 : inventaire des contrats CLI, crawl, WASM et EmDash et identification des coutures de migration à poursuivre.

## Invariants

- Les usages existants et leurs paramètres par défaut continuent de fonctionner sans configuration supplémentaire.
- Les identifiants de règles, contrats de résultats, formats de sortie et comportements d'intégration restent compatibles ; toute migration nécessaire est explicite.
- Une gate indique ses entrées requises et son scope ; un contrôle non effectué ne doit jamais apparaître comme réussi.
- Les presets sont explicables et résolus de manière déterministe ; les checks de base fonctionnent localement, sans réseau ni modèle.
- Les performances du parcours unitaire sont mesurées ; les chemins natif et WASM sont vérifiés indépendamment.
