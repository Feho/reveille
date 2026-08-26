<!-- SPDX-License-Identifier: GPL-2.0-only -->

# Reveille — descriptif complet du logiciel

> Document destiné à être lu par un agent IA qui n'a pas accès au code. Il décrit ce que fait
> Reveille, à qui il s'adresse, chaque écran, chaque état, la copie exacte affichée, et les
> contraintes qui ont façonné les décisions. Rien ici ne demande de lire une ligne de source.
>
> Version décrite : `main` au 21 août 2026 (v1 Windows, build de développement non packagé).
> L'interface du logiciel est **en anglais** ; les libellés cités le sont donc verbatim.

---

## 1. En une phrase

Reveille est un lanceur de jeu pour **Medal of Honor: Allied Assault** (MOHAA, 2002) qui trouve
l'installation du joueur, liste les serveurs qui répondent *maintenant*, dit exactement ce qui
manque sur le disque pour jouer sur chacun d'eux, télécharge ce qui manque, puis lance le jeu
connecté au serveur choisi.

## 2. Le problème qu'il résout

MOHAA a 24 ans et une communauté résiduelle mais réelle. Trois frictions tuent l'arrivée d'un
nouveau joueur :

1. **Le navigateur de serveurs intégré au jeu ment par omission.** Il liste tout ce qui est
   enregistré auprès du master GameSpy, y compris les endpoints qui ne répondent plus. Mesure
   réelle du 19 août 2026 : **169 enregistrés, 127 qui répondent vraiment**. Environ un tiers de
   la liste est du bruit — ce qui donne l'impression que « le jeu est mort » alors qu'il ne l'est
   pas.
2. **Les serveurs vivants tournent sur des cartes personnalisées.** Un joueur qui vient
   d'installer le jeu ne les a pas. Se connecter à un tel serveur échoue, ou le joueur est éjecté
   quand la rotation atteint une carte absente. Trouver l'archive `.pk3` correspondante est une
   chasse manuelle sur des forums.
3. **Rien ne dit à l'avance ce qui manque.** Le jeu ne le découvre qu'à la connexion.

Mesure de contrôle : sur un corpus figé de 114 serveurs, un joueur avec **l'installation retail
d'origine et rien d'autre** est déjà compatible avec **~73 %** de la population vivante ; les
cartes manquantes concernent **~14 %** des serveurs. Ce n'est donc pas le blocage principal, mais
c'est le dernier kilomètre — et c'est précisément là que le joueur abandonne aujourd'hui.

## 3. Public visé

- **Cible principale : le joueur revenant ou débutant** (« utilisateur lambda »). Il vient de
  réinstaller un vieux jeu, il veut être en partie en moins de dix minutes, il ne connaît ni
  `pk3`, ni `fs_game`, ni checksum.
- **Cible secondaire : l'habitué / l'admin**, qui veut voir les faits bruts (protocole, version du
  moteur, checksum publié ou non, rotation complète) sans qu'on les lui cache.

L'application est **Windows uniquement** en v1, en **thème sombre exclusivement**.

## 4. Le vocabulaire du domaine (indispensable pour comprendre les écrans)

| Terme | Sens dans Reveille |
|---|---|
| **Master list** | Annuaire GameSpy qui renvoie la liste des serveurs *enregistrés*. Être enregistré ne prouve pas être vivant. |
| **Sweep / browse** | Le balayage : interroger le master, puis interroger chaque serveur individuellement pour savoir s'il répond et ce qu'il tourne. Dure typiquement 30 à 60 s. |
| **Non-result** | Un endpoint enregistré qui n'a pas donné de résultat exploitable (timeout, injoignable, réponse illisible, doublon, pas de port de jeu publié). Reveille les **compte et les classe** au lieu de les afficher comme des serveurs. |
| **Rotation** (`sv_maplist`) | La liste des cartes que le serveur enchaîne. **Seuls ~98 serveurs sur 127 la publient.** |
| **Map now** (`mapname`) | La carte tournant à l'instant. Peut ne pas figurer dans la rotation publiée. |
| **Clients** | Le nombre de slots occupés rapporté par le serveur. **Ce ne sont pas forcément des humains** — c'est un contrat produit, voir §8. |
| **Bots** | Quantité **disjointe** des clients, jamais additionnée ni fondue dedans. |
| **moh-db** | Catalogue communautaire externe de cartes. Il publie des noms, des tailles, des compteurs de téléchargement — **mais aucune empreinte cryptographique**. |
| **pk3** | Archive contenant une ou plusieurs cartes, posée dans le dossier `main` du jeu. |
| **Retail / OpenMoHAA** | Deux moteurs possibles : le binaire d'origine, ou le port open source moderne. Reveille sait lancer les deux. |

## 5. Les quatre états de compatibilité (le cœur du produit)

Pour chaque serveur, Reveille compare ce que le serveur publie (rotation ∪ carte en cours) au
contenu réellement présent sur le disque, et produit **exactement un** de ces quatre états :

| État | Signification |
|---|---|
| **Compatible** | Toutes les cartes publiées, y compris celle en cours, sont sur le disque. |
| **Needs N maps** | N cartes de la rotation manquent, et le catalogue peut probablement les fournir. |
| **No source** | Au moins une carte manquante n'existe dans aucun catalogue joignable. |
| **Can't tell** | Le serveur ne publie pas de rotation. Seule la carte en cours a pu être vérifiée. |

Trois subtilités qui structurent l'interface :

- **`Compatible` ne veut pas dire « tu vas entrer ».** Bans, serveur plein, limites de ping sont
  décidés par le serveur au moment de la connexion et ne sont pas prévisibles. `Compatible`
  signifie seulement : *rien de vérifiable ne cloche*.
- **`No source` n'interdit pas de jouer.** On peut jouer jusqu'à ce que la rotation atteigne la
  carte absente, puis on est éjecté. Refuser la connexion inventerait un problème que le moteur
  n'a pas.
- **Le seul blocage dur** est que **la carte en cours** soit absente *et* introuvable au
  catalogue : cette connexion-là est rejetée immédiatement par le serveur, donc Reveille ne la
  propose pas.

## 6. Le pipeline, vu du joueur

```
   [1] Trouver le jeu            → dossier d'installation + niveau de confiance de l'identification
   [2] Indexer le contenu local  → toutes les cartes présentes (archives pk3 + fichiers libres)
   [3] Balayer les serveurs      → master → interrogation par serveur → lignes qui arrivent en flux
   [4] Classer chaque serveur    → un des quatre états ci-dessus, calculé localement, sans réseau
   [5] Prévisualiser un join     → interroger moh-db pour chaque carte manquante
   [6] Télécharger + lancer      → installer, réindexer, reclasser, puis démarrer le jeu
```

Les étapes 3, 5 et 6 sont longues, streamées et **annulables**. Aucune n'est une boîte noire : la
progression est toujours chiffrée dès qu'un total est connu.

---

# Les écrans

Fenêtre : **1180×760 par défaut, minimum 920×620**, redimensionnable, centrée. Fond quasi noir.
Il n'y a que **deux surfaces** dans toute l'application (Setup et Servers), plus une boîte de
dialogue. Il n'existe **pas** d'écran de réglages.

## Écran A — Setup (« où est le jeu ? »)

Affiché **uniquement** tant qu'aucune installation n'est résolue. Ce n'est pas une page d'accueil
marketing : il répond à une seule question, et dit avec quelle confiance il y répond. On y revient
volontairement en cliquant la puce du chemin dans la barre de titre.

```
                ┌──────────────────────────────────────────────┐
                │  REVEILLE            FIRST RUN               │
                │                                              │
                │  Found your game                             │
                │  Read from the game files on disk.           │
                │                                              │
                │  D:\Jeux\EA GAMES\MOHDA          [Verified]  │
                │  Contains   Allied Assault                   │
                │  Client     1.11 · 1 binary                  │
                │                                              │
                │  [ Use this install ]     [ Choose another ] │
                └──────────────────────────────────────────────┘
```

**États successifs de cet écran**

| État | Titre | Sous-titre |
|---|---|---|
| Détection en cours | `Looking for Allied Assault` | `Checking the usual locations.` |
| Trouvé au démarrage | `Found your game` | `Read from the game files on disk.` |
| Trouvé après clic sur la puce | `Your game folder` | idem |
| Rien trouvé (1er lancement) | `Show Reveille your game` | `Nothing was found automatically. Pick the folder once and Reveille remembers it.` |
| Dossier saisi invalide | `Show Reveille your game` | `No Medal of Honor installation there.` |
| Détection en échec | — | `Detection failed. Pick the folder instead.` |

**La puce de confiance** (à droite du chemin ; l'explication complète est en infobulle) :

- `Verified` (vert) — le binaire correspond à un build dont Reveille a l'empreinte.
- `Recognised` (laiton) — c'est bien un client Medal of Honor, mais ce build précis n'est pas dans
  le corpus. *Identifié par le nom, pas par le hash.*
- `Data only` (neutre) — les données du jeu sont là, aucun exécutable client. Reveille peut
  indexer les cartes ; lancer nécessite un client.

**Saisie manuelle** (quand rien n'est trouvé, ou après « Choose another ») : un champ texte
(placeholder `D:\Games\MOHAA`), un bouton `Browse…` qui ouvre le sélecteur de dossier natif, un
bouton primaire `Check this folder` (désactivé tant que le champ est vide), et l'indice
« The folder holding **main** and the game client. »

Si plusieurs produits sont détectés, ils sont listés (`Allied Assault · Spearhead`) et le
sélecteur **Game** de la barre d'outils choisit lequel jouer ; Reveille gère les trois.

Le dossier accepté est mémorisé ; au lancement suivant, cet écran est **sauté** silencieusement.

---

## Écran B — Servers (toute la session se passe ici)

```
┌────────────────────────────────────────────────────────────────────────────────────┐
│ REVEILLE                                            D:\Jeux\EA GAMES\MOHDA  ◄── puce│
├────────────────────────────────────────────────────────────────────────────────────┤
│ ⌕ Search server names  ☐ Has people                            ▓▓▓▓▓░░ 78/190 [Stop]│
├──────────────────────────────────────────────┬─────────────────────────────────────┤
│ SERVER          CLIENTS   MAP NOW  PING  RUNS  NEEDS│ SERVER                        │
│ harzCore          40/64  dm/mohdm6  41 ms 1.11 │  <[TFC]> Objective                 │
│  62.75.x.x:12203  +8 bots                      │  173.249.214.104:12203             │
│ <[TFC]> Objective  1/32  obj/bluts  84 ms 1.11  + 7 maps                            │
│ [FORTE] Public    21/32  dm/mohdm6 137 ms 1.11  not published │ Engine  MOHAA 1.11 … │
│ =MB= Revival Mie   0/20  dm_stanalie 29 ms 1.12 + 1 map    │ Now       obj/bluts    │
│ …                                              │  Rotation  14 maps                 │
│                                                │                                    │
│                                                │  JOIN CHECK                        │
│                                                │  Needs 7 maps                      │
│                                                │  9.1 MB  to fetch · 6 files        │
│                                                │                                    │
│                                                │  MAPS                              │
│                                                │  On disk — nothing to do           │
│                                                │  Matched in the catalogue          │
│                                                │  Needs your choice                 │
│                                                │  No source                         │
├──────────────────────────────────────────────┼─────────────────────────────────────┤
│ 106 of 190 answered · 88 clients reported · 108 bots, counted separately            │
│ [84 registered but not listed]                    stopped early  14:32 │ [Get 9.1 MB & join] │
└────────────────────────────────────────────────┴────────────────────────────────────┘
```

### B.1 Barre de titre

Le mot-symbole `REVEILLE` à gauche ; à droite une **puce** affichant le chemin d'installation
courant (le préfixe Windows `\\?\` est retiré à l'affichage). Cliquer la puce renvoie à l'écran
Setup pour changer de dossier. C'est le seul « réglage » de l'application.

### B.2 Barre d'outils

De gauche à droite :

1. **Champ de recherche** — filtre sur le nom d'hôte uniquement, en direct. Raccourci `/` pour y
   aller, `Échap` pour vider et revenir à la liste.
2. **`Has people`** — bascule ; masque les serveurs à 0 client. Désactivée par défaut. C'est la
   seule bascule : un `Hide unavailable maps` a existé, puis a été retiré, car il filtrait sur un
   état que la liste n'affiche plus — des lignes disparaissaient sans que rien à l'écran ne dise
   pourquoi.
3. **Zone d'action**, à droite, qui contient soit :
   - **au repos** : un bouton primaire `Find servers` (première fois) ou `Refresh` (ensuite) ;
   - **pendant un balayage** : une jauge + un compteur `78/190` + un bouton `Stop`. Tant que le
     master n'a pas répondu, la jauge est indéterminée et le compteur affiche `contacting master`.
     Après clic sur `Stop`, le bouton devient `Stopping…` et se désactive — les sondes déjà en vol
     doivent encore expirer, et le dire vaut mieux que laisser le bouton paraître inerte.

La bascule et le tri sont **persistés** entre les sessions.

> Il n'existe volontairement **aucun filtre « seulement compatibles »**. Voir §7.1.

### B.3 Le tableau des serveurs

Six colonnes. Tri par clic sur l'en-tête, flèche ▲/▼ sur la colonne active. Tri par défaut :
**clients décroissants**.

| Colonne | Contenu | Triable |
|---|---|---|
| **Server** | Nom d'hôte, et en dessous l'adresse `ip:port` en petit | oui |
| **Clients** | `21/32` — occupés / capacité. Sur une seconde ligne, `+8 bots` si le serveur en déclare. `—` si le serveur ne publie rien (jamais `0`) | oui |
| **Map now** | La carte en cours, telle que le serveur l'orthographie (`obj/obj_team2`) | oui |
| **Ping** | `84 ms` — l'aller-retour **mesuré** de la requête `getstatus` de ce balayage. Un seul échantillon UDP, pris pendant que quinze autres sondes étaient en vol. Ce n'est **ni** le ping en jeu, **ni** `sv_minPing`/`sv_maxPing` (la barrière d'admission du serveur). Infobulle : « Time for one status request to this server and back, measured once during this check. Not the in-game ping. » Ni couleur, ni barres, ni bandes : une mesure, pas un verdict | oui (croissant d'abord) |
| **Runs** | Version courte du moteur (`1.11`, `1.12+0.83.0`). Infobulle : la chaîne complète | non |
| **Needs** | Le coût, voir ci-dessous | non |

**La colonne `Needs` — la décision de design la plus importante du produit :**

| État | Cellule | Couleur |
|---|---|---|
| `Compatible` | **vide** | — |
| `Needs N maps` | `+ 7 maps` | aucune (encre normale) |
| `No source` | `7 maps unavailable` | rouge doux — **la seule cellule colorée de tout le tableau** |
| `Can't tell` | `not published` | grisé, italique |
| `Can't tell` + carte en cours absente | `+ 1 map` | aucune |

Chaque cellule porte une infobulle avec l'explication complète, de sorte que le libellé court n'ait
jamais à porter tout le sens. **Il n'y a ni pastille verte, ni badge d'état, ni feu tricolore.**

**Sélection** : un clic (ou le focus clavier) sur une ligne la sélectionne et déclenche
immédiatement la prévisualisation dans le panneau de droite. La liste ne disparaît jamais : on peut
comparer deux serveurs sans navigation aller-retour.

**Lignes qui arrivent en flux** : pendant un balayage, les serveurs apparaissent au fur et à mesure
qu'ils répondent (repeints regroupés à ~4 Hz pour ne pas concurrencer le balayage). À la fin, la
liste est remplacée par la liste faisant autorité — les doublons d'endpoints ne sont dédupliqués
qu'à ce moment.

**États vides du tableau :**

| Situation | Titre | Texte |
|---|---|---|
| Balayage en cours, rien encore | `Checking servers` | `Rows appear as each server answers.` |
| Des serveurs existent, les filtres ne laissent rien passer | `Nothing matches` | `No server matches the current search and filters.` |
| Rien n'a jamais été balayé | `No servers yet` | `Nothing has been checked yet.` |

### B.4 Barre d'état (bas de fenêtre)

Séquence : **`106` of 190 answered** · **`88` clients reported** (infobulle : « Occupied slots
reported by every server. Not verified as people. ») · **`108` bots, counted separately** ·
un **bouton** `84 registered but not listed` · puis, à droite, `stopped early` si l'utilisateur a
interrompu, et l'heure de fin du balayage.

Avant tout balayage : `Not checked yet`. En cas d'échec : le message d'erreur.

### B.5 Boîte de dialogue « Registered but not listed »

Ouverte par le bouton de la barre d'état. Titre : **Registered but not listed**. Corps :
« Registered with the master list, but no usable reply. The in-game browser lists these anyway. »
suivi d'une liste `nombre → raison` en langage clair, où **l'étape compte autant que la raison** :

- `did not answer the game query` / `did not answer the server-list query`
- `was unreachable for the game query`
- `answered the server-list query with a reply Reveille could not read`
- `is the same server registered twice`
- `did not publish a game port`

C'est la réponse directe au « le jeu est mort » : l'écart entre 190 et 106 est **montré et
expliqué**, pas caché.

---

## Écran C — Le panneau de détail (colonne de droite, ~380 px)

Ce n'est pas un écran séparé : il vit à droite de la liste en permanence. C'est là que se prend la
décision, donc c'est là que les **noms canoniques des quatre états** apparaissent (le tableau ne
les répète pas).

Au repos : `No server selected` / « Pick one to see what it needs. »

Une fois un serveur sélectionné, le panneau empile, de haut en bas :

### C.1 En-tête
Libellé `SERVER`, le nom d'hôte en grand, l'adresse `ip:port` en dessous (sélectionnable au
curseur, pour la copier).

### C.2 Faits du serveur
Une liste clé/valeur : **Now** (carte en cours ou `not published`), **Rotation** (`14 maps` ou
`not published`), et si publié : **Reserved** (`2 slots held back`).

La version du moteur n'apparaît pas ici : la liste la porte déjà dans la colonne **Runs**, et la
compatibilité est énoncée par le Join check, pas par un numéro de version que le joueur devrait
interpréter. La **join window** n'y figure pas non plus — elle ne change rien à la réussite du join.

### C.3 Join check
Libellé `JOIN CHECK`, puis le **nom d'état** en titre : `Compatible` / `Needs 7 maps` /
`No source` / `Can't tell`, avec son explication en infobulle.

Pendant que le catalogue est interrogé, une jauge déterminée s'affiche :
`looking up obj/obj_team2 · 3/7`.

Une fois la résolution terminée, un **chiffre principal** : `9.1 MB` suivi de
« to fetch · 6 files · 1 awaiting a choice ».

### C.4 Maps
Titré **`MAPS`** (et non « Rotation »), parce que Reveille vérifie la rotation **et** la carte en
cours. Si le serveur ne publie pas de rotation : « No rotation published. Only the map running now
was checked. »

Le contenu est regroupé par *ce que le joueur doit décider*, pas par ce que le fil de données a
envoyé :

| Groupe | Contenu | Note |
|---|---|---|
| **On disk — nothing to do** | `7 maps: dm/mohdm1, dm/mohdm2, …, and 3 more` | replié en une ligne |
| **Matched in the catalogue** | `↓ obj/bluts   1.2 MB` + le nom du fichier `.pk3` en dessous | correspondance exacte par nom |
| **Needs your choice** | `? dm/stanalie  3 candidates` puis un groupe de **boutons radio**, chacun affichant `nom_de_fichier` + `4.1 MB · 320 downloads · tested` | **rien n'est présélectionné**, jamais |
| **No source** | `✕ obj/rarecustom  —` + « Not in any catalogue Reveille can reach. » | infobulle : « You can play here until the rotation reaches these maps, then you are dropped. » |
| **Missing locally** | Cartes manquantes que la résolution n'a pas encore couvertes : `not on disk` ou `different file` | état transitoire |

### C.5 Limites du verdict
Ces phrases n'apparaissent que si le fait correspondant est vrai, une phrase chacune, sans
alarmisme. Elles sont placées **à l'intérieur du Join check (C.3)**, sous le verdict, et non dans
une section à part : chacune dit pourquoi le verdict peut être plus faible qu'il n'en a l'air.
- « Sends no files — anything missing has to be here before you join. » (le serveur refuse
  d'envoyer les fichiers lui-même)
- « Publishes no map checksum, so only names are matched, not files. » (seuls 30 serveurs sur 127
  publient un checksum) — c'est ce qui empêche « Compatible » de promettre plus qu'un accord de noms

### C.6 Barre d'action (fixée en bas du panneau)

**Un seul bouton primaire.** Son libellé *est* le consentement :

| Situation | Libellé | Effet |
|---|---|---|
| `Compatible` | `Join` | lance directement |
| Il y a des fichiers à récupérer | `Get 9.1 MB & join` | télécharge puis lance |
| `Can't tell` | `Join without a rotation check` | lance en connaissance de cause |
| Plus rien de récupérable | `Join anyway` | lance en connaissance de cause |
| Installation en cours | `Working…` (désactivé) | — |
| Carte en cours absente **et** introuvable | `Cannot join yet` (désactivé) | seul blocage dur |

Messages contextuels au-dessus du bouton, selon le cas :
- « obj/bluts is running now and is not on disk. **Fetching is what makes this join work.** »
- « obj/bluts is running now and is not on disk. **Pick a source for it above.** »
- « obj/bluts is running now, is not on disk, and is not in the catalogue. **Joining would drop you
  immediately.** » (bouton désactivé)
- « 2 maps need a choice above. »

### C.7 Pendant le téléchargement
La section Maps est remplacée par `GETTING FILES` : une ligne par fichier, avec le nom de carte, un
état (`waiting` → `1.2 MB / 4.1 MB` → `checking archive` → `installed` / `failed`) et une jauge
pendant le transfert. **Un échec n'interrompt jamais la passe** : il est consigné sur sa ligne avec
sa raison, et les autres continuent.

### C.8 Après le lancement
Section `LAUNCHED` / `NOT LAUNCHED` :
- Succès : « The game is starting », puis « <jeu> is connecting. Bans, a full server and ping
  limits are the server's call from here. » — le jeu nommé est celui de la session (Allied
  Assault, Spearhead ou Breakthrough). C'est le **seul** endroit où cet avertissement universel
  est dit, au moment où il s'applique.
- `4 files installed into D:\Jeux\EA GAMES\MOHDA\main`.
- Si le dossier de jeu n'était pas inscriptible : un encart laiton donnant **le vrai chemin de
  repli** (`%APPDATA%\openmohaa\main`), jamais un euphémisme. Reveille ne déclenche jamais d'élévation
  UAC.
- `Not installed` : la liste des cartes en échec avec leur raison.
- Un bouton `Back to the check` pour revenir à l'analyse du serveur.

Le jeu est démarré avec la console activée et les vidéos d'introduction désactivées, pour que le
joueur voie le message du moteur si la connexion échoue et n'attende pas l'intro.

---

# Ce qui gouverne les décisions d'interface

## 7. Deux décisions fondatrices

### 7.1 « Un prix, pas un verdict »

Le design évident était une colonne de statut : vert *Compatible*, orange *Needs 3 maps*, rouge
*No source*, gris *Can't tell*. Il a été **rejeté délibérément**.

Un feu tricolore enseigne un seul comportement : *ne cliquer que sur le vert*. Or « Needs 3 maps »
n'est pas un défaut — c'est très exactement la raison d'être du logiciel, c'est un clic, ça prend
quelques secondes. Le marquer en orange à côté d'une alternative verte éloigne le joueur d'environ
un quart de la population vivante, et précisément des serveurs aux rotations les plus riches. Cela
reproduirait, à l'intérieur de Reveille, l'impression « le jeu est mort » que Reveille existe pour
corriger.

D'où : **la disponibilité est l'absence de travail, pas une récompense.** Un serveur prêt a une
cellule vide. Le travail supplémentaire s'affiche comme une étiquette de prix.

### 7.2 « L'ambiance dans le décor, jamais dans les données »

L'identité Seconde Guerre mondiale vit dans : le mot-symbole, l'accent laiton, les libellés
condensés en capitales, le fond quasi noir, les écrans de premier lancement et les états vides.

Elle reste **hors** des tableaux, des nombres, des puces, des formulaires — de tout ce que le joueur
lit pour décider. Une première version avait poussé l'ambiance dans le plan des données (titres
serif 75 px, grain de film sur toute la fenêtre, olive et rouille partout) : elle est devenue
illisible et a été jetée. **Un lanceur est un outil ; le jeu fournit l'atmosphère.**

## 8. Les règles d'honnêteté (contrats produit, pas préférences de style)

Enfreindre une de ces règles est considéré comme un bug, pas comme un choix esthétique.

| Règle | Comment l'interface la respecte |
|---|---|
| Ne jamais appeler un compte de clients « players » ou « humans » | La colonne s'appelle **Clients**. Infobulle : « Occupied slots reported by every server. Not verified as people. » |
| Les bots sont disjoints des clients | Rendus sur leur propre ligne (`+8 bots`), jamais additionnés. La barre d'état dit « counted separately ». |
| Ne jamais laisser croire qu'il reste des places | La capacité n'apparaît **que** comme dénominateur (`21/32`). `capacité − clients` n'est jamais calculé : ce n'est pas observable. |
| Ne jamais produire un booléen « puis-je entrer ? » | Quatre états, jamais une coche. `Compatible` est explicité : « the server still decides whether you get in ». |
| Ne jamais appeler l'aller-retour mesuré « le ping du jeu » | La colonne s'appelle **Ping** (c'est le mot que les joueurs cherchent) mais chaque explication est l'honnête : un échantillon unique, mesuré pendant ce balayage, et l'infobulle le dit. `RoundTripMillis` est un type distinct de `PingMillis` côté cœur, pour que les deux ne puissent pas être confondus. |
| Ne jamais présenter un téléchargement moh-db comme vérifié | Les candidats affichent `tested` (le drapeau du catalogue lui-même), jamais « verified ». |
| Ne jamais appliquer automatiquement une correspondance ambiguë | Les radios démarrent **sans sélection**. Le total exclut les cartes non tranchées, et le panneau dit combien attendent un choix. |
| Dire où les fichiers sont allés | Le chemin de repli réel est imprimé, pas un euphémisme. |
| Un échec est un non-résultat consigné, jamais une passe avortée | Les échecs d'installation sont listés un par un ; les endpoints muets sont comptés et ventilés par raison. |
| Dire « did not answer », pas « offline » | On sait le premier, pas le second. |
| Le silence reste du silence | « not published » quand le serveur n'a rien publié — jamais promu en coche. |

## 9. La règle de densité (une correction issue d'une revue utilisateur)

L'honnêteté est une **contrainte sur les affirmations, pas un permis d'expliquer**. Une version
antérieure respectait toutes les règles ci-dessus et était quand même mauvaise : elle plaidait son
raisonnement devant le joueur. Chaque liste avait un paragraphe la justifiant, chaque serveur
portait la même note permanente sur les bans et la capacité, chaque correspondance ambiguë
réexpliquait la politique de non-application automatique. Rien de tout cela ne changeait le clic
suivant, et le volume enterrait les deux ou trois lignes qui, elles, comptaient. Le retour du
propriétaire était : *« beaucoup de texte, aucune valeur réelle pour un utilisateur lambda ».*

La règle retenue : **une explication mérite un paragraphe seulement si elle change le clic
suivant.** Sinon c'est une infobulle sur la chose qu'elle explique, ou ce n'est pas dans
l'interface.

Corollaires appliqués : un avertissement vrai de *tous* les serveurs (bans, capacité, ping) est dit
**une fois, au moment où il s'applique** — après le lancement, pas avant chaque connexion. Un
serveur prêt ne dit rien : le silence est le rendu correct de « rien à faire ».

Dans la même revue, une case à cocher de confirmation qui précédait le bouton a été supprimée :
c'était deux clics pour une seule réponse. **Le consentement, c'est le clic ; le libellé, c'est
l'information.** Ce qui ne doit jamais revenir, c'est l'inférence *silencieuse* — lancer une partie
non vérifiée sans que le libellé le dise.

## 10. Système visuel

Palette (thème sombre unique ; contrastes mesurés contre les trois fonds) :

| Jeton | Valeur | Rôle |
|---|---|---|
| `--void` | `#090B0E` | barre de titre, barre d'état, champs en creux |
| `--bg` | `#13171C` | fond de fenêtre |
| `--panel` | `#1A1F26` | barre d'outils, panneau de détail, cartes |
| `--rise` | `#212831` | contrôles surélevés |
| `--ink` | `#E7E5E0` | texte principal (14,3:1) |
| `--dim` | `#98A1AC` | texte secondaire (6,9:1) |
| `--faint` | `#8A929E` | texte tertiaire (5,7:1) |
| `--brass` | `#D9A648` | accent, action primaire, mot-symbole (8,1:1) |
| `--ok` / `--warn` / `--bad` | `#5CA372` `#D98040` `#C4594C` | points, bordures, remplissages |
| `--ok-text` / `--warn-text` / `--bad-text` | `#7FC294` `#E8A06B` `#E08375` | **seules** couleurs d'état autorisées sur du texte |

Tous les jetons de texte passent WCAG AA (4,5:1) sur chaque surface où ils sont permis. `--bad`
mesure 4,18 et est donc **interdit sur du texte** — d'où l'existence de `--bad-text`.

**Typographie : aucune police embarquée, aucun CDN.** Une application de bureau doit s'afficher
avant qu'un réseau existe.
- Titres / libellés : `Bahnschrift` (la condensée livrée avec Windows 11), repli `Segoe UI
  Variable Display`, `Segoe UI`, `system-ui`.
- Corps : `Segoe UI Variable Text`, `Segoe UI`, `system-ui`.
- **Données** : `Cascadia Mono`, `Consolas`, `ui-monospace`. Chaque nombre, chemin, adresse, nom de
  carte et nom de fichier est en chasse fixe avec `tabular-nums`, pour que les colonnes s'alignent
  et que les chiffres se comparent.

## 11. Accessibilité et interaction

- La liste est un **vrai tableau** avec légende, en-têtes déclarés et `aria-sort` — pas une grille
  de boutons. Les lignes sont focalisables et portent `aria-selected`.
- **Clavier** : `↑` `↓` `Début` `Fin` déplacent la sélection ; `Entrée` et `Espace` activent ;
  `/` place le curseur dans la recherche ; `Échap` la vide ; `F5` ou `Ctrl+R` relance un balayage.
- Anneau de focus laiton visible sur chaque élément interactif ; les contours ne sont jamais
  supprimés.
- Une région `aria-live="polite"` annonce la progression du balayage et les changements d'état ;
  `role="alert"` est réservé aux vraies erreurs.
- Toute animation respecte `prefers-reduced-motion`.
- La progression est **déterminée** dès qu'un total est connu (`78/190`, octets), et indéterminée
  seulement pendant la poignée de main avec le master, où rien n'est encore connu.
- Toute opération longue est annulable : le balayage a un `Stop` ; sélectionner un autre serveur
  abandonne la résolution catalogue en cours.

---

## 12. Ce qui n'existe pas aujourd'hui (v1)

Utile pour ne pas proposer ce qui est déjà là, et pour savoir où sont les vrais trous.

- **Aucun écran de réglages.** La concurrence du balayage (16 sondes en parallèle), le délai de
  sonde (2,5 s) et le délai master (15 s) sont figés et exposés nulle part. Le seul réglage
  atteignable est le dossier de jeu, via la puce de la barre de titre.
- **Pas de favoris, pas d'historique, pas de « derniers serveurs joués ».**
- **Pas de latence en jeu.** La colonne `Ping` donne l'aller-retour d'**une** requête de statut
  pendant le balayage ; le ping réel d'une partie n'est jamais mesuré, et aucun historique ni
  seconde mesure n'existe pour lisser cet échantillon unique.
- **Pas de liste de joueurs**, ni de noms, ni de scores.
- **Pas de rafraîchissement automatique ni périodique.** Un balayage est toujours déclenché
  manuellement — sauf le tout premier, lancé automatiquement à l'entrée dans l'écran Servers.
- **Pas de suivi d'un serveur** (« préviens-moi quand il se remplit »).
- **Pas de gestion du contenu déjà installé** : on ne peut ni voir ni supprimer les cartes que
  Reveille a téléchargées.
- **Spearhead et Breakthrough** (les extensions) sont supportés par le moteur interne mais la v1
  cible Allied Assault en dur.
- **Windows uniquement** ; build non packagé, non signé.
- Aucun tri sur les colonnes `Runs` et `Needs`.
- `Runs` disparaît sous 1080 px de large, `Ping` sous 960 px : en dessous, les colonnes fixes ne
  laissent plus assez de place pour lire un nom de serveur.
- Le champ de recherche filtre **seulement** le nom d'hôte (ni carte, ni adresse, ni version).
- Aucune notion de profil joueur : pseudo, mot de passe de serveur, réglages de jeu.

## 13. Ce qui est déjà tranché et ne doit pas être re-proposé

Ces points ont été décidés avec le propriétaire du projet, contre l'option évidente, pour les
raisons documentées ci-dessus :

1. Pas de feu tricolore / pastille de statut / filtre « seulement compatibles » dans la liste.
2. Pas de case à cocher de confirmation avant le bouton de connexion.
3. Pas de fusion clients + bots, pas d'affichage de « places libres ».
4. Pas de thème clair en v1, pas de police téléchargée, pas de CDN.
5. Pas de paragraphes explicatifs qui ne changent pas le clic suivant.

---

## 14. Comment se servir de ce document

Deux angles utiles :

**Angle « utilisateur lambda ».** Simuler quelqu'un qui vient de réinstaller un jeu de 2002, qui ne
sait pas ce qu'est un `pk3`, et qui a dix minutes devant lui. Parcourir : premier lancement →
dossier trouvé → premier balayage (30 à 60 s) → choix d'un serveur peuplé → il manque 7 cartes →
téléchargement → lancement. Où hésite-t-il ? Que ne comprend-il pas ? Que fait-il si rien n'est
trouvé, si le balayage échoue, si un téléchargement rate, si le serveur choisi se vide pendant le
téléchargement, s'il ne sait pas quel candidat choisir dans « Needs your choice » ?

**Angle « expert UI/UX ».** Évaluer la hiérarchie de l'information, la charge cognitive de la
colonne `Needs`, la lisibilité du panneau de détail quand il empile faits + verdict + cinq groupes
de cartes + barre d'action, la gestion des états longs et des états vides, la découvrabilité du
changement de dossier (une simple puce dans la barre de titre), l'absence totale de réglages, et la
cohérence entre ce que la liste montre et ce que le panneau explique.

Dans les deux cas, les §12 (ce qui manque) et §13 (ce qui est verrouillé) délimitent l'espace des
propositions utiles.
