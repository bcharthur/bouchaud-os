# Timer local : transitions et limites de preuve

Reference b95043b5 ; le candidat 291d6b4d n'est pas reapplique.
Le controleur centralise les quatre sites WRMSR existants. Il ne change ni
le quantum, ni les bornes de sommeil, ni la marge d'une echeance deja due.
Il ne pretend pas expliquer a lui seul le surcout P2 : la plupart des
programmations observees ont des cibles differentes et restent obligatoires.

| Appel | Contexte et protection | Transition / obligation materielle |
|---|---|---|
| init_local_scheduler_timer | CPU proprietaire au demarrage | LVT TSC-deadline puis quantum initial |
| arm_local_scheduler_timer | IRQ locale ou restauration, IF masque dans la fonction | Ancien etat invalide ; nouveau quantum absolu, ou borne interactive plus proche ; WRMSR obligatoire |
| avance_pour_sommeil_interactif | tache locale se gare, IF masque dans la fonction | Nouvelle borne plus proche que la cible et le quantum : WRMSR ; aucune prolongation du quantum |
| note_entree_vecteur_local | IRQ, IF=0, apres EOI | Cible passee : etat Expire ; cible future : conservee. Ne decide PAS de la cause materielle |
| rearme_reste_du_quantum | IRQ ou retour HLT masque | Quantum existant ou prochaine interactive ; cible identique elidee seulement si encore future et connue armee |
| arme_reveil_precis_avant_hlt | scheduler, IF=0, handshake idle publie | Borne du CPU/propre avant quantum ; EN_ARMEMENT protege la bascule de politique |
| retire_reveil_precis_apres_hlt | retour HLT, IF masque dans la fonction | Retrait du sommeil idle et restauration du reste du quantum |
| veut_reveil_precis | commande shell | Bascule SeqCst puis attente bornee des EN_ARMEMENT ; OFF n'arme plus de sommeil, les IRQ/retours HLT restaurent le quantum |

`programme_deadline` est l'unique ecrivain de IA32_TSC_DEADLINE et de son
cache ARME_TSC. Seul le CPU concerne le modifie, IF=0 ; aucun NMI n'appelle
ce controleur. L'etat arme est publie apres le WRMSR. Une echeance qui passe
pendant la decision reste livrable/pending dans l'APIC : le vecteur a deja
ete acquitte et IF reste masque jusqu'au retour. Il n'y a pas de lecture
MSR supplementaire sur le chemin critique. Une nouvelle date plus eloignee
est ecrite aussi : l'ancien one-shot n'est jamais pris pour un periodic.

Les six etats distinguent NonProgramme, Quantum, Interactif, Idle, Expire,
Invalide. Une valeur future Invalide/Expire ne permet pas l'elision. La
decision pure est testee contre un modele materiel one-shot SMP1/SMP4 qui
consomme ses echeances independamment des entrees IRQ. Sept mutations
executables doivent echouer, notamment comparaison par egalite seule et
expiration a borne exclusive. Les conversions non entieres et la marge due
gardent leurs tests historiques.

## Vecteur partage : choix conservateur

RESCHEDULE_VECTOR (0xF1) est programme dans le LVT du timer (TSC ou LAPIC),
envoye par reschedule_cpu et par broadcast_reschedule. Le dernier exclut
le CPU appelant ; reschedule_cpu refuse aussi une destination locale.
Les shootdowns ont leur propre vecteur. L'IDT utilise une porte interrupt,
sans IST pour 0xF1 : IF=0, un EOI pour chaque entree du vecteur, profondeur
IRQ suivie par GsGuard puis par la pile en cas de commutation.

La separation physique de vecteurs n'est pas introduite sans mesure de son
benefice et validation APIC. Un IPI et une expiration peuvent coalescer dans
l'IRR : chaque entree sert toutes les echeances dues et toutes les demandes
pendantes, sans choisir une seule cause. `tir_anticipe` signifie seulement
avant le quantum ; `tirs` et `tirs_anticipes` se recoupent. Aucune demande
distante/ciblee n'est supprimee. Les gardes de verrou, transition, registre
et IRQ imbriquee restent exigees pour la publication/preemption.

TCG n'execute pas les WRMSR TSC-deadline. Ses tests d'integration ne sont
donc pas une preuve materielle de ce controleur ; KVM reste indispensable.
