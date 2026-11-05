# Aexeo : plan de migration vers les gates, presets et adapters

État : livraison de l’architecture et des contrôles disponibles hors ligne. Les modèles Jev/JEF et leur validation réelle sont reportés à une étape ultérieure, conformément au choix du projet.

## Objectif

Faire évoluer Aexeo vers un moteur agnostique composé de gates configurables, de presets portables et d'adapters, tout en conservant par défaut les usages SEO/GEO, CLI, crawl et EmDash existants. Les vérifications de sources et de faits viennent en dernier.

## Suivi

- [x] Mettre à niveau Aethyme et ouvrir une session broker.
- [x] Copier ce plan dans le worktree de session et maintenir son avancement au fil des livraisons.
- [x] Phase 1 — caractériser les contrats et usages existants ; ajouter les tests de compatibilité nécessaires ; isoler les points de couplage et nettoyer sans modifier le comportement.
- [x] Phase 2 — définir les contrats d'évaluation, capacités, scopes, gates et résultats ; construire le moteur et le preset SEO/GEO existant ; adapter les parcours site, crawl, CLI et EmDash/WASM ; vérifier la parité.
- [x] Phase 3 — rendre les presets TOML composables et portables : héritage, surcharges, validation, sélection par contexte, inspection et versionnement ; fournir des presets initiaux.
- [x] Phase 4 — implémenter les gates de contenu déterministes et l'adapter Markdown/stdin ; vérifier un même preset et une même gate sur Markdown et EmDash.
- [x] Phase 5 — livrer le workflow agent : skill, découverte des presets, sorties structurées, correctifs sûrs et benchmarks de latence.
- [x] Phase 6 — fournir les extensions et les contrôles de sources et de faits disponibles hors ligne, en explicitant leurs limites ; les modèles externes restent facultatifs et différés.
- [x] Livrer par le workflow broker : PR #20 fusionnée dans `main` au commit `92413c5`, session 1 clôturée ; tests locaux et CI de la PR validés. La CI du commit de fusion est suivie séparément.

## Travail restant identifié

- [x] Exposer l’évaluation de presets sur un crawl dans la CLI, avec statut partiel, diagnostics et tests de bout en bout.
- [x] Ajouter des extensions fournies par un hôte Rust et tester leur présence ou indisponibilité ; les adapters CLI et WASM signalent les extensions non chargées comme non vérifiées.
- [x] Ajouter en dernier un contrôle littéral de passages et comparaison de faits structurés dans les sources fournies, avec indisponibilité explicite.
- [ ] Différé, hors prérequis de cette livraison : évaluer Jev/JEF ou une alternative sur un jeu indépendant et valider la procédure d’appréciation des sources avec des réviseurs qualifiés. Aucun résultat simulé ne prouve leur précision.
- [x] Relancer les contrôles locaux et distants avant intégration : suite complète locale, 21 tests d’adaptateur et CI de la PR réussis. Le broker a vérifié les conflits ; aucune gate Aethyme n’était configurée sur la base.
- [ ] Suivre le résultat de la CI sur le commit de fusion `92413c5` dans `main`.

## Progression détaillée

- Validation récente : `CARGO_NET_OFFLINE=true sh scripts/check-repo.sh` réussi sur `4004aee` ; builds WASM plugin et worker réussis ; vérifications Node/WASM des faits structurés et des revues périmées réussies. La CI complète a réussi sur `fa43b6e` (Rust, installation release, performance, couverture, EmDash, worker et adaptateur Jev). Chaque nouveau commit doit être vérifié à son tour.
- Phase 6 : procédure de revue des sources documentée (portée, contexte, provenance, arbitrage et révision) ; son efficacité doit encore être validée auprès de réviseurs qualifiés.
- Phase 6 : le rapport de mesure indique les effectifs et résultats par classe, y compris les classes absentes ; ces statistiques descriptives ne prouvent pas la fiabilité.
- Phase 6 : séparation reproductible par composantes reliées (affirmation ou article) : 237 passages de développement et 101 de test, sans identifiants communs ; pas de stratification et pas de classe « uncertain ».
- Phase 6 : importateur SciFact testé sur le jeu de développement officiel : 338 passages annotés, 188 affirmations distinctes, 182 documents ; 112 affirmations sans passage annoté écartées. Corpus de domaine scientifique, sans classe « uncertain » ; fichiers locaux sous `.aexeo-reports/scifact/`, non suivis par Git. Le pipeline lot + métriques a été vérifié sur ces 338 cas avec des réponses simulées ; aucune mesure du modèle effectuée.
- Phase 6 : adaptateur Jev optionnel avec seuil explicite, fiabilité fournie séparément et attribuée à un évaluateur avec justification, mesures hors ligne sur labels indépendants, contrôle de version/protocole/seuil et tests simulés et transport HTTP vérifié sur serveur local (21 tests hors ligne pour l’adaptateur et les outils de mesure) ; aucun appel réel ni mesure de calibration effectués.
- Phase 6 : extensions Rust explicites et gates de présence littérale, de comparaison de valeurs JSON et d’intégration de jugements externes liés aux snapshots fournis ; aucun jugement sémantique, contrôle de fiabilité ou vérification factuelle générale n’est encore implémenté.
- Phase 2 : adapter Rust pour appliquer un preset à `RuntimeAudit` en conservant statut, raison de troncature et diagnostics de crawl ; la sortie CLI historique du crawl reste inchangée ; `content --crawl-url` expose aussi ce parcours.
- Phase 2 : premier moteur de contenu indépendant des sites ; trois types de gate et résultats `passed` / `failed` / `not_checked`. Le preset `seo-website` appelle l’audit site existant comme une gate globale ; les groupes natifs sont aussi disponibles individuellement avec `site_group` ; le preset complet reste une gate globale pour conserver les diagnostics de déploiement.
- Phase 3 : presets TOML nommés, héritage, surcharges, désactivation, détection des cycles et inspection CLI ; presets article et glossaire minimaux, variantes éditoriales à seuils consultatifs et fichiers TOML copiables intégrés aux builds.
- Phase 4 : CLI texte/stdin et bridge EmDash/WASM raccordés au même moteur ; parsing Markdown et gate de titres partagés avec EmDash ; preset optionnel raccordé aux sauvegardes et refreshs EmDash configurés ; sélection par collection disponible ; mode sandbox pris en charge via configuration du sidecar et sélection par collection.
- Phase 5 : documentation, skill et découverte CLI des presets ; benchmark local reproductible et premier correctif explicite de fin de fichier ; autres correctifs sûrs à étudier.

- Phase 1 : suite Rust complète, typage et 74 tests du plugin EmDash validés ; tests ajoutés pour la compatibilité des options de règles, sévérités et suppressions du pont EmDash.
- Phase 1 : dispatch statique regroupé dans un registre unique avec vérification des groupes et conservation de leur ordre et des libellés de timing.
- Phase 1 : la PR #11 modifie aussi le dispatch pour ajouter les règles image ; resynchroniser et réconcilier après son intégration.
- Phase 1 : inventaire des contrats et frontières CLI, crawl, WASM et EmDash documenté dans `docs/content-architecture.md`, avec tests de parité et de compatibilité.

- CI : correction des `unwrap` refusés par le Clippy strict et de la dépendance verrouillée du plugin signalée par l’audit de production ; validation finale du nouveau commit en cours. La surcharge npm ne se propage pas aux consommateurs du paquet.

- Validation : `sh scripts/check-repo.sh` complet réussi (format, Clippy strict, tests, isolation, audit, deny, udeps, documentation et configuration) ; build plugin et worker WASM, typechecks et suite EmDash complète (85 tests) réussis. `cargo audit` conserve un avertissement Tokio autorisé. Le broker ne configure aucune gate sur la base ; les contrôles CI et la revue restent nécessaires.

- Phase 5 : benchmark release local de 100 itérations, document synthétique 29 011 octets : évaluation p50 25 µs / p95 191 µs ; parsing Markdown + évaluation p50 9 651 µs / p95 67 894 µs (machine chargée ; sans démarrage CLI).

## Invariants

- Les usages existants et leurs paramètres par défaut continuent de fonctionner sans configuration supplémentaire.
- Les identifiants de règles, contrats de résultats, formats de sortie et comportements d'intégration restent compatibles ; toute migration nécessaire est explicite.
- Une gate indique ses entrées requises et son scope ; un contrôle non effectué ne doit jamais apparaître comme réussi.
- Les presets sont explicables et résolus de manière déterministe ; les checks de base fonctionnent localement, sans réseau ni modèle.
- Les performances du parcours unitaire sont mesurées ; les chemins natif et WASM sont vérifiés indépendamment.
