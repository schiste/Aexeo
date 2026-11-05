# CLEF et revue d'Aexeo

- [x] Vérifier l'API publique Lift Wing et le contrat effectif de CLEF-Flash.
- [x] Raccorder CLEF au contrat de revue existant avec provenance distincte, conservation du texte et fiabilité des sources explicitement incertaine.
- [x] Tester l'adaptateur hors ligne et effectuer un essai exploratoire borné sur des annotations humaines ; documenter biais, échecs et limites.
- [x] Revoir les responsabilités et duplications, puis mesurer et optimiser le comptage de mots partagé entre règles ; vérifier les cas limites.
- [ ] Vérifier la CLI et la chaîne complète, terminer les contrôles du dépôt et de CI, puis livrer dans main via le broker.

Les jugements du modèle ne prouvent ni le support d'une affirmation ni la fiabilité d'une source. Les échantillons exploratoires ne sont pas une calibration de production ; garder les passages douteux en revue humaine.

Mesure locale indicative en release sur 29 011 octets, 20 règles et 100 itérations : médiane 793 µs avant, 35 µs après. Les valeurs de queue étaient bruitées pendant les compilations et ne constituent pas un budget de production. Le benchmark ne mesure pas les appels au modèle.
