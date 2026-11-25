# Aide de RolidSCP

RolidSCP est un gestionnaire de fichiers à deux panneaux pour les disques locaux et les serveurs SFTP.
Chaque panneau a des onglets ; un onglet est un emplacement (un dossier local ou un dossier sur un
serveur) avec son propre nom et sa propre couleur. Toutes les fonctions se trouvent dans les menus, la
plupart aussi dans la barre d'outils et dans la barre des touches de fonction en bas. La liste de tous
les raccourcis clavier se trouve sur la deuxième page de cette fenêtre.

**F10** ouvre la barre de menus sans souris : **←** **→** passent d'un menu à l'autre, **↑** **↓**
parcourent les éléments, **→** entre dans un sous-menu, **Entrée** choisit un élément, **Échap** ferme
le menu.

Avec un lecteur d'écran (Orca), la liste annonce l'élément sous le curseur (son nom, « dossier » ou la
taille, et s'il est sélectionné), les boutons et les dialogues leur nom.

## Panneaux et onglets

- **Tab** passe du panneau gauche au panneau droit et inversement. Le panneau actif a un en-tête plus
  marqué.
- **Enter** ou un double-clic ouvre un dossier, une archive (comme dossier en lecture seule) ou un
  fichier dans son application par défaut. Les programmes et les scripts ne sont jamais lancés sans
  demander ; les fichiers sur un serveur ne sont jamais exécutés. **Backspace** remonte d'un niveau.
- **Ctrl+T** ouvre un menu pour un nouvel onglet : dossier personnel, favoris, serveurs enregistrés et
  emplacements récents ; tapez pour le filtrer. **Ctrl+W** ferme l'onglet, **Ctrl+Tab** passe d'un
  onglet à l'autre. Un clic droit sur un onglet propose Dupliquer, Renommer, Couleur, Fermer et Fermer
  les autres onglets ; un double-clic le renomme. L'info-bulle d'un onglet montre son serveur, son
  adresse et son chemin. **Ctrl+K** trouve un onglet ou un serveur enregistré par son nom.
- Menus contextuels au clavier : **Shift+F10** ou la touche **Menu** ouvre le menu de l'élément sous le
  curseur (sur `..` ou dans un dossier vide, le menu du dossier), **Ctrl+Shift+F10** ou **Ctrl+Menu** le
  menu de l'onglet actif. **↑ ↓** déplacent, **→** ouvre un sous-menu (dans Changer la couleur, **↓**
  ou **Tab** parcourent les couleurs), **Entrée** choisit, **Échap** ferme. Les deux sont aussi dans
  la barre de menus : **Fichier → Menu contextuel de l'élément…** et **Paramètres → Menu de
  l'onglet…**.
- **Ctrl+D** ajoute le dossier aux favoris ou l'en retire. Les favoris et les dossiers récents sont
  dans le menu du bouton ★ de la barre (`*` quand la barre est masquée) ; **Fichier → Dossiers
  favoris…** permet de les nommer et de les ordonner.
- Dans le menu **Ctrl+T** (et le bouton **+** près des onglets), les favoris sont toujours affichés :
  **＋ Ajouter le dossier actuel** ajoute le dossier du panneau actif (aussi sur un serveur), ✏ renomme un
  favori et 🗑 le retire (cliquez encore une fois pour confirmer, **Échap** revient en arrière).
- Commencez à taper pour filtrer la liste ; **Esc** efface le filtre.
- **Ctrl+F1**, **Ctrl+F2** et **Ctrl+Shift+F1** basculent entre la vue courte, la vue détaillée et
  les miniatures. **Alt+←** et **Alt+→** reculent et avancent dans l'historique des dossiers.
- Toutes les boîtes de dialogue fonctionnent de la même façon : **Entrée** active le bouton mis en
  évidence, **Échap** ou ✕ annule, **Tab** passe d'un champ à l'autre. On déplace une boîte de dialogue
  par sa barre de titre ; ⓘ affiche plus d'aide.
- Les onglets, les couleurs et les dossiers sont enregistrés en permanence et restaurés au prochain
  démarrage.
- Au-dessus de chaque panneau se trouve une barre : une liste déroulante des dossiers du chemin depuis
  la racine (**Alt+↓** ; flèches, **Entrée**, **Échap**), pour un panneau local aussi les disques de
  cet ordinateur avec leur taille ; puis Précédent, Suivant, Niveau supérieur, Dossier racine
  (**`Ctrl+\`**), Dossier personnel (**Ctrl+H**), Actualiser et les favoris (★). Dans une archive, la
  liste commence par le dossier où se trouve l'archive ; dans les résultats d'une recherche, elle n'est
  pas disponible. **Affichage → Barre au-dessus des panneaux** masque la barre ; les boutons `/ .. ~ *`
  reviennent alors dans la ligne du chemin.
- Chaque partie du chemin est un bouton : un clic ouvre ce dossier (aussi sur un serveur et dans une
  archive ; un clic sur le dossier où vous êtes l'actualise), le bouton du milieu l'ouvre dans un
  nouvel onglet à côté de l'onglet actuel, et la partie sous la souris est mise en évidence. Un
  chemin trop long pour la ligne est raccourci par la gauche avec « … » (un clic dessus ouvre la
  liste déroulante du chemin) ; la dernière partie reste toujours entière. Pour saisir un chemin,
  double-cliquez sur la partie vide de la ligne, cliquez sur le crayon à son extrémité ou appuyez
  sur **Ctrl+G**.
- À droite, la ligne du chemin montre l'espace libre du disque (une fine barre et « Libre : X sur Y
  »), son système de fichiers (ext4, btrfs, nfs4, fuse.sshfs…) et, pour les disques de cet
  ordinateur, un badge avec l'état du disque ; dans un panneau étroit, « Libre : » disparaît
  d'abord, puis la taille du disque, puis le système de fichiers. Dans une archive, elle montre le
  disque où se trouve l'archive. Un clic ouvre le détail.
- Un clic droit sur l'espace vide de la liste (ou sur `..`) propose **Nouveau fichier…**, **Nouveau
  dossier**, **Actualiser**, **Ajouter le dossier aux favoris**, **Ouvrir un terminal ici** et
  **Propriétés du disque…** (espace libre, système de fichiers et état du disque).
- Dans la liste détaillée, de fines lignes séparent les colonnes. Faites glisser une ligne entre les
  titres des colonnes pour changer la largeur de la colonne à sa droite ; un double clic ajuste la
  colonne à son contenu. Nom occupe le reste du panneau et s’élargit donc avec la fenêtre ; dans un
  panneau étroit, Permissions puis Modifié sont masqués. Les largeurs sont gardées séparément pour le
  panneau de gauche et celui de droite (même après un redémarrage) ; **Afficher → Réinitialiser la
  largeur des colonnes** rétablit les valeurs par défaut. Un nom ou une extension qui ne tient pas se
  termine par « … » ; survolez-le pour le voir en entier.
- Les listes se tiennent à jour toutes seules. Un dossier de cet ordinateur montre en un instant les
  changements faits par d’autres programmes. Un dossier sur un serveur (ou sur un disque réseau) est
  actualisé quand vous y revenez – un autre onglet, l’autre panneau ou le retour dans la fenêtre – si la
  liste date de plus de 5 secondes, et tant que la fenêtre est active, aussi toutes les 30 secondes
  (**Afficher → Actualisation automatique** : désactivée, 10 s, 30 s, 1 min ou 5 min ; un dossier de
  plus de 20 000 éléments au plus toutes les 5 minutes). Le curseur, les sélections, la position de
  défilement, le tri et les tailles de dossiers calculées restent. Rien n’est actualisé pendant que vous
  renommez dans la liste, faites glisser ou qu’un dialogue est ouvert, et une connexion perdue n’est pas
  réessayée. Après vos propres opérations, tous les onglets qui montrent le dossier sont actualisés ;
  **Ctrl+R** actualise tout de suite.

## Sélection des fichiers

- **Insert**, **Space** ou **Ctrl+clic** sélectionne un élément ; **Ctrl+A** sélectionne tout ou rien.
- **Shift+clic** sélectionne les éléments du dernier cliqué ou sélectionné jusqu’à celui que vous
  cliquez. **Shift** avec **↑ ↓ PgUp PgDn Home End** sélectionne de là jusqu’au curseur pendant qu’il
  se déplace ; en revenant, seul ce que cette plage a sélectionné est désélectionné. **Outils →
  Sélectionner jusqu’au début / Sélectionner jusqu’à la fin** fait la même chose que **Shift+Home** et
  **Shift+End**. Un clic sans touche déplace le curseur et garde la sélection. Le début de la plage est
  retrouvé par son nom après une actualisation, un tri ou le filtre rapide ; `..` n’est jamais
  sélectionné. Le glisser copie toujours, et **Shift** maintenue au moment de déposer les fichiers les
  déplace (Shift+clic seul ne déplace jamais rien).
- **Num+** sélectionne un groupe par un masque comme `*.jpg;*.png`, **Alt+−** désélectionne un
  groupe, **Alt+I** inverse la sélection.
- Les dialogues de masque de **Num+** et **Alt+−** retiennent le dernier masque et l'état de
  **Inclure les dossiers**, chacun pour soi et aussi après un redémarrage. Sur un serveur marqué
  comme production, **Inclure les dossiers** est toujours désactivé à l'ouverture du dialogue, et le
  choix fait alors n'est pas retenu.
- La ligne sous la liste indique le nombre d'éléments sélectionnés et leur taille.
- **Alt+Space** (ou **Shift+Space**) calcule la taille du dossier sous le curseur ou des dossiers
  sélectionnés, **Alt+Shift+Enter** celle de tous les dossiers du panneau.

## Copier, déplacer, renommer

- **F5** copie les éléments sélectionnés (ou l'élément sous le curseur) vers le dossier de l'autre
  panneau, **F6** les déplace. Les deux fonctionnent entre disques et serveurs, et aussi entre deux
  serveurs.
- Le dossier cible de la copie et du déplacement peut être modifié, choisi parmi les cibles récentes ou
  sélectionné avec **Parcourir…** (sur le disque ou sur le serveur).
- Faites glisser des fichiers avec la souris vers l'autre panneau, sur un dossier, sur `..` ou sur un
  onglet (maintenez-les au-dessus d'un onglet pour l'afficher). Glisser copie, avec **Shift** déplace,
  **Échap** annule le glissement. La boîte de dialogue Copier ou Déplacer demande toujours d'abord.
- Lorsqu'un fichier existe déjà dans la cible, vous choisissez : ignorer, écraser seulement les plus
  anciens, écraser, ou reprendre un transfert interrompu. Le choix par défaut se règle dans le menu
  Paramètres.
- Les fichiers sont écrits sous un nom temporaire et renommés une fois complets : un transfert
  interrompu ne laisse donc jamais un fichier à moitié écrit sous le vrai nom.
- La reprise ne continue qu’un fichier partiel (`.filepart`) du même fichier source : son chemin, sa
  taille et sa date de modification sont gardés à côté. Un fichier partiel d’une autre source ou
  d’une source inconnue n’est pas touché et est signalé ; choisissez **Écraser** pour recommencer.
- Quand le disque cible est plein ou que le quota disque est épuisé, le transfert s’arrête et le dit ;
  un fichier plus ancien du même nom reste tel quel et la partie écrite est conservée. Libérez de
  l’espace et appuyez sur **Réessayer** dans la file des transferts : le transfert reprend là où il
  s’est arrêté. Un serveur ne signale souvent que « Failure » ; le programme regarde alors l’espace
  libre sur le serveur.
- Un déplacement entre disques ou serveurs ne supprime un fichier source que s’il n’a pas changé
  pendant le transfert (taille et date de modification) et que la copie a la même taille. Avec
  **Vérifier par somme de contrôle avant de supprimer la source** (activé par défaut lors d’un
  déplacement vers un serveur), les deux sont d’abord relus et comparés ; une différence est
  signalée et la source reste.
- **F2** ou **Shift+F6** renomme directement dans la liste : le nom sans l'extension est sélectionné,
  **Enter** confirme, **Esc** annule.
- **F7** crée un dossier, **F8** supprime (avec confirmation). La suppression s’exécute dans la file
  des transferts avec le nombre d’éléments supprimés ; **Annuler** l’arrête (ce qui est déjà
  supprimé le reste).
- Les tubes (FIFO), sockets, périphériques et liens symboliques qui ne mènent nulle part (cible
  absente ou boucle) ne sont pas copiés : ils sont ignorés et listés dans le rapport final, le reste
  de la copie continue.
- Les noms inhabituels sont copiés à l’identique : sauts de ligne, barre oblique inverse, noms de
  255 octets et, sur les disques de cet ordinateur, noms qui ne sont pas en UTF-8 valide. La liste
  les affiche sur une ligne : caractères de contrôle comme `\n` ou `\t`, chaque octet invalide comme
  `\xFF`. Un nom qui n’est pas en UTF-8 valide ne peut pas être envoyé à un serveur (SFTP prend les
  noms comme du texte) ; c’est signalé.
- La file de transferts exécute jusqu'à trois transferts à la fois. La limite de vitesse se trouve
  dans la boîte de dialogue de copie.
- **Ce qu'une copie conserve** en plus du contenu : les permissions (lecture, écriture, exécution ;
  setuid, setgid et sticky sont omis) et la date de modification. Elle ne conserve pas le propriétaire
  ni le groupe (la copie appartient à l'utilisateur qui l'écrit), les ACL, les attributs étendus (ni les
  étiquettes SELinux ni les capabilities de fichier) ni les liens physiques (chaque lien devient un
  fichier à part). SFTP ne peut pas du tout transmettre les ACL ni les attributs étendus. Un fichier
  écrasé est remplacé par un nouveau, ses propres ACL et attributs disparaissent donc aussi ; le nouveau
  fichier reçoit ce que le dossier donne aux nouveaux fichiers (par exemple une ACL par défaut).
- **Les dates** sont affichées dans le fuseau horaire de cet ordinateur, celles des fichiers d'un
  serveur aussi, quel que soit le fuseau du serveur. **Écraser seulement les plus anciens** compare les
  dates à la seconde près, indépendamment des fuseaux horaires. Une copie garde la date de sa source,
  une horloge de serveur déréglée ne gêne donc pas pour les fichiers copiés ; elle gêne pour les
  fichiers modifiés sur le serveur, qui reçoivent l'heure du serveur : si son horloge avance d'une
  heure, un fichier modifié là-bas paraît plus récent d'une heure.
- **Glisser vers d'autres applications** (X11) : faites glisser des fichiers ou des dossiers hors de la
  fenêtre, par exemple vers un gestionnaire de fichiers ou un éditeur. Les fichiers locaux sont copiés,
  avec **Shift** déplacés. Les fichiers d'un serveur ou d'une archive sont seulement copiés : une
  application qui prend en charge l'enregistrement direct (XDS) les reçoit téléchargés dans son dossier
  par la file des transferts ; sinon ils sont d'abord téléchargés dans un dossier temporaire privé (au-delà
  de 1 Gio, une confirmation est demandée), supprimé quand plus rien ne l'utilise et à la fermeture.
  Certaines applications n'attendent les fichiers qu'environ 30 secondes : un gros téléchargement peut
  donc échouer par ce chemin. De retour au-dessus de la fenêtre, le glisser continue à l'intérieur ;
  **Échap** l'annule. Sous Wayland, voir **Glisser-déposer sous Wayland** plus bas.

## Glisser-déposer sous Wayland <!-- wayland -->

- Sous Wayland, on ne peut pas encore glisser des fichiers vers d'autres programmes ni depuis eux (un
  gestionnaire de fichiers, un logiciel de courrier, une messagerie), dans aucun sens. La bibliothèque
  de fenêtres sur laquelle repose le programme n'a pas de glisser-déposer sous Wayland ; sa prochaine
  version l'a, et le programme l'utilisera quand elle sera prête. Le glissement entre les panneaux et
  les onglets fonctionne.
- **Le remède :** afficher la fenêtre par XWayland, comme une fenêtre X11. Le glissement fonctionne
  alors partout où le bureau le transmet entre programmes X11 et Wayland. GNOME le fait dans les deux
  sens (testé) ; dans un test avec KDE Plasma (KWin 6.3), on pouvait glisser des fichiers dans le
  programme, mais pas en dehors, comme pour les autres programmes X11.
- Activez **Paramètres → Glisser-déposer → Sous Wayland, utiliser X11 (XWayland)** et relancez le
  programme. C'est désactivé par défaut. Ou lancez-le une fois ainsi depuis un terminal :
  `WAYLAND_DISPLAY= rolidscp` (deb et rpm), ou `WAYLAND_DISPLAY= ./RolidSCP-0.9.1-x86_64.AppImage` avec
  le nom de votre AppImage.
- **Ce que vous perdez :** le texte est moins net avec une mise à l'échelle fractionnaire (par exemple
  125 % ou 150 %) : XWayland dessine la fenêtre à l'échelle 1 et le bureau l'étire ; à 100 % et 200 %
  il reste net. Avec des écrans à des échelles différentes, la fenêtre a une seule échelle pour tous.
  **D'autres programmes X11 peuvent lire ce que vous tapez dans la fenêtre, le mot de passe principal
  aussi** ; une fenêtre Wayland sépare les programmes. Le presse-papiers et les méthodes de saisie
  passent par XWayland.
- La première fois que vous glissez des fichiers hors de la fenêtre sous Wayland, la ligne en bas de
  la fenêtre dit que cela ne fonctionne pas ; **Que faire…** ouvre cette partie de l'aide. **Aide →
  Glisser-déposer sous Wayland…** l'affiche aussi, avec ce sur quoi la fenêtre tourne maintenant.

## Serveurs

- **Ctrl+O** ouvre le gestionnaire de serveurs. Saisissez le serveur sous la forme `host` ou
  `user@host:port`, enregistrez-le comme profil avec un nom et une couleur d'onglet, puis connectez-vous
  avec **Enter**.
- **Dupliquer…** sous la liste des serveurs enregistrés crée une copie avec tous ses réglages sous un
  nouveau nom (« Web (copie) »). Si le serveur a un mot de passe enregistré, vous choisissez s'il est
  copié aussi (il faut alors le mot de passe principal) ; par défaut, il ne l'est pas. Les
  modifications non enregistrées du formulaire ne sont pas copiées.
- Marquez un serveur comme **serveur de production** : ses onglets ont un en-tête rouge et les actions
  dangereuses demandent une confirmation supplémentaire (la suppression ne peut pas être confirmée avec
  Enter).
- Sur un serveur de production, la copie, l’envoi, le déplacement et la création d’un dossier ne se
  confirment pas non plus avec Entrée, et la copie ne commence jamais par Écraser (Ignorer les
  existants est présélectionné).
- **Se connecter via** un autre serveur enregistré (un serveur de rebond, ProxyJump dans OpenSSH) :
  le programme se connecte d'abord à ce serveur, avec sa propre clé ou son mot de passe enregistré,
  puis de là au serveur cible. L'adresse du serveur cible est alors celle que voit le serveur de
  rebond, par exemple une adresse de son réseau local. Les clés des deux serveurs sont vérifiées, et
  une question sur une clé inconnue indique duquel il s'agit. Un serveur de rebond peut lui-même en
  avoir un (au plus quatre à la suite). Après une coupure, la connexion est rétablie par le même
  chemin, et le terminal (**Ctrl+P**) passe aussi par là. Un serveur enregistré par lequel passent
  d'autres serveurs ne peut pas être supprimé tant qu'ils n'ont pas un autre chemin.
- La clé du serveur est vérifiée dans deux listes : la liste des serveurs connus propre au
  programme (`~/.config/rolidscp/known_hosts`) et `~/.ssh/known_hosts`, que le programme lit seulement
  et ne modifie jamais. Une nouvelle clé n'est ajoutée à la liste du programme qu'après confirmation
  de son empreinte ; une clé modifiée est refusée, de même qu'une clé marquée `@revoked`. Quand les
  deux listes ne s'accordent pas sur un serveur (l'une a la clé envoyée par le serveur, l'autre une
  clé différente), le programme ne se connecte pas et indique les deux lignes : demandez à
  l'administrateur du serveur quelle clé est la bonne et supprimez la mauvaise ligne.
- La connexion essaie ssh-agent, vos clés, puis le mot de passe. Les mots de passe enregistrés sont
  conservés chiffrés, protégés par un mot de passe principal.
- Les mots de passe enregistrés se verrouillent à nouveau après un délai réglable depuis leur dernière
  utilisation (menu Paramètres → mot de passe principal, 60 secondes par défaut) ; la barre d'état
  affiche le temps restant. Le verrouillage efface aussi les mots de passe gardés pour la reconnexion.
- Si la connexion est coupée, RolidSCP se reconnecte de lui-même et les transferts interrompus
  reprennent. Un transfert continue tant qu'il progresse entre les coupures ; après cinq coupures
  d'affilée sans progrès, il s'arrête et vous pouvez le relancer depuis la file d'attente.
- Quand rien ne vient du serveur pendant 15 secondes alors que le programme l'attend, la connexion
  est considérée comme morte et rétablie aussitôt (une courte vérification après 5 secondes
  distingue un serveur lent d'une ligne morte). Dans les trois premières minutes après une coupure,
  une tentative a lieu toutes les quelques secondes, puis toutes les 10 secondes, et après dix
  minutes toutes les 30 secondes. Un envoi reprend à la dernière partie confirmée par le serveur.
- Il en va de même quand le câble réseau est débranché pendant plusieurs minutes ou que le serveur
  se met en veille : le programme continue d’essayer, et le transfert reprend dès que le serveur est
  de nouveau joignable.
- Sur un serveur sans shell ou sans certains outils (`du`, `find`, `df`, `sha256sum`, par exemple
  BusyBox ou un compte SFTP seul), tout passe par SFTP seul : la taille des dossiers et la recherche
  en parcourant les dossiers, les sommes de contrôle en lisant les fichiers.
- Un serveur défaillant ou malveillant ne peut pas faire écrire le programme hors du dossier cible :
  les noms contenant `/`, `..` ou vides sont refusés, les caractères de contrôle des noms sont
  affichés comme `\xNN`. Une liste de dossier de plus de 5 millions d'éléments est arrêtée, et un
  fichier que le serveur envoie bien au-delà de sa taille n'est pas pris.
- **Ctrl+P** ouvre le terminal du système (`$TERMINAL` ou `x-terminal-emulator`) dans le dossier du
  panneau. Sur un serveur, il lance `ssh` dans ce dossier, avec la même vérification de la clé du
  serveur que RolidSCP (une clé inconnue ou modifiée met fin à la connexion). Un mot de passe
  enregistré est transmis par un assistant à usage unique (`SSH_ASKPASS`) qui ne répond qu'à la
  demande de mot de passe de ce serveur, une seule fois ; si les mots de passe sont verrouillés, le
  mot de passe principal est demandé d'abord, sinon ssh le demande lui-même. Pour un serveur de
  production, une confirmation est demandée avant. Le mot de passe ne passe jamais par une ligne de
  commande, un fichier ou le journal.
- Après la connexion, le programme demande l'heure au serveur (`date -u +%s`, seulement si le serveur
  a un shell, 5 secondes au plus). Si l'horloge du serveur est décalée de plus d'une minute, la ligne
  du bas le signale une fois, l'info-bulle de l'onglet l'affiche et les dialogues de copie et de
  déplacement avertissent sous « Écraser les plus anciens » : les fichiers modifiés sur ce serveur ont
  des dates décalées d'autant. L'avertissement se désactive dans **Paramètres → Connexions aux
  serveurs**.
- **Ctrl+L** ramène le panneau actif sur le disque local.

## Afficher des fichiers (F3)

- **F3** ouvre le fichier dans la visionneuse, une fenêtre à part (plusieurs peuvent être ouvertes à
  la fois). Elle est aussi dans le menu contextuel, le menu Fichier et la barre des touches. La
  visionneuse n'exécute jamais rien et ne charge rien de l'extérieur : HTML, XML et SVG sont affichés
  comme du texte.
- Les fichiers de toute taille s'ouvrent aussitôt : seule la partie à l'écran est lue. Un fichier sur
  un serveur n'est pas téléchargé en entier ; un fichier dans une archive est d'abord extrait dans un
  fichier temporaire. Les numéros de ligne avec **~** sont estimés tant que les lignes ne sont pas
  comptées.
- **1**, **2** et **3** passent du texte à l'hexadécimal et à l'image ; les fichiers binaires
  s'affichent d'eux-mêmes en hexadécimal.
- **W** renvoie les lignes à la ligne, **N** affiche les numéros de ligne, **Ctrl+G** va à une ligne,
  à un pourcentage ou à une position, **Ctrl+Home** et **Ctrl+End** vont au début et à la fin. Les
  menus Encodage et Coloration changent l'encodage et le langage de la coloration syntaxique
  (désactivée au-delà de 20 Mo, voir **Paramètres → Paramètres… → Visionneuse (F3)**).
- **F** suit la fin d'un fichier qui grandit, aussi sur un serveur ; remonter met le suivi en pause.
  Quand le fichier change, une barre propose de le recharger. Une connexion perdue est signalée par
  une barre et l'affichage reprend tout seul dès son retour.
- **Ctrl+F** ou **F7** recherche du texte (casse, mots entiers, expression régulière) ; **F3** et
  **Shift+F3** vont à l'occurrence suivante et précédente. Tout le fichier est fouillé en arrière-plan ;
  sur un serveur, `grep` cherche sur place quand c'est possible, sinon le fichier est lu par SFTP, et
  la barre de recherche indique la méthode.
- Sélectionnez à la souris ou avec Shift et les flèches ; **Ctrl+C** copie la sélection (32 Mo au plus).
- Images (JPEG, PNG, GIF aussi animé, WebP, BMP) : **F** ajuste l'image à la fenêtre, **0** l'affiche
  à 100 %, la molette zoome, le glisser la déplace, **R** et **L** font pivoter l'affichage, les
  flèches et **PgUp**/**PgDn** montrent l'image précédente et suivante du dossier. Une image sur un
  serveur n'est téléchargée que jusqu'à 50 Mo (réglable).
- **F4** modifie le fichier comme dans le panneau ; **Esc** ou **Q** ferme la visionneuse.

## Modifier et ouvrir des fichiers

- **F4** ouvre le fichier dans un éditeur externe. Un fichier d'un serveur y est renvoyé après chaque
  enregistrement ; s'il a été modifié sur le serveur entre-temps, on vous demande quoi faire.
- La copie locale est conservée dans un dossier privé sur le disque (`~/.local/state/rolidscp/edit`,
  pour vous seul) et supprimée à la fin de la modification. Un envoi qui échoue pour une autre
  raison qu’une connexion perdue est réessayé deux fois, puis signalé. L’heure du dernier envoi est
  affichée avec les secondes.
- Un serveur sans renommage atomique reçoit un fichier modifié par étapes (l’original est d’abord
  mis de côté). Si la connexion ou le programme s’interrompt entre-temps, l’étape est terminée ou
  annulée à la connexion suivante à ce serveur, et vous êtes informé des fichiers concernés.
- **Shift+F4** (aussi **Fichier → Nouveau fichier…** et le menu contextuel) crée un nouveau fichier et
  l'ouvre dans l'éditeur externe. Le nom sous le curseur est proposé. Sur cet ordinateur, le fichier vide
  est créé aussitôt ; sur un serveur, il l'est au premier enregistrement, donc fermer l'éditeur sans
  enregistrer ne crée rien. Un nom existant ouvre ce fichier pour modification.
- **Shift+Enter** ou le menu contextuel (clic droit) propose **Ouvrir avec…**, les permissions
  (**Ctrl+Shift+A**) et la copie du nom, du chemin ou de l'adresse `sftp://` dans le presse-papiers.
- Après un enregistrement dans l’éditeur (**F4**, **Shift+F4**), la taille et la date d’un fichier de
  cet ordinateur sont mises à jour dans le panneau.

## Archives

- **Alt+F5** compresse les éléments sélectionnés en zip, 7z, tar, tar.gz ou tar.zst, avec un mot de
  passe en option (zip et 7z).
- **Alt+F9** extrait une archive dans l'autre panneau, avec une protection contre les chemins
  dangereux et les bombes d'archive. **Alt+Shift+F9** teste une archive.
- Une compression ou une extraction interrompue par un plantage ou l'arrêt forcé du programme ne
  laisse son fichier `.filepart` inachevé que jusqu'au prochain démarrage du programme, qui le
  supprime.
- **Enter** sur une archive l'ouvre comme dossier en lecture seule ; **F5** en copie des fichiers vers
  l'extérieur.
- Les dates dans un zip sont gardées à la seconde près et en UTC (comme le fait Info-ZIP), elles sont
  donc justes dans un autre fuseau horaire aussi. Un zip d'un programme qui n'écrit que l'ancienne heure
  DOS (sans fuseau horaire, secondes paires seulement) est lu dans le fuseau horaire de cet ordinateur.

## Recherche et sommes de contrôle

- **Ctrl+F7** (ou **Ctrl+F**) recherche par nom (masque, texte ou expression régulière) et par
  contenu. Sur un serveur, la recherche s'exécute sur place avec `find` et `grep` si possible.
- Les sommes de contrôle (MD5, SHA-1, SHA-256, SHA-512, BLAKE3, CRC32) se trouvent dans le menu
  Fichier et dans le menu contextuel ; elles peuvent être enregistrées au format de `sha256sum -c`.

## État du disque (SMART)

- Le badge de la ligne du chemin montre l'état du disque : **OK**, **avertissement** (orange) ou
  **critique** (rouge). Un clic affiche une explication claire et un conseil.
- Le détail montre le modèle et le numéro de série, le type (HDD, SSD, NVMe), l'usure et la durée de
  vie restante estimée, la température, la durée de fonctionnement, les mises sous tension, les arrêts
  inattendus, les secteurs défectueux et instables, les erreurs de transfert (CRC ; en général un câble
  ou un connecteur) et les données écrites. Tous les attributs sont dans la partie Avancé.
- Les données viennent d'UDisks2 (sans droits d'administrateur), ou de `smartctl` s'il fonctionne
  sans eux. Elles sont lues en arrière-plan, au plus une fois par jour et par disque (gardées dans
  `~/.cache/rolidscp/smart.json`) ; **Recharger** relit le disque tout de suite.
- Avertissement : usure d'un SSD à partir de 60 %, des secteurs défectueux, instables ou illisibles,
  un disque dur à partir de 45 °C ou un SSD à partir de 60 °C. Critique : usure à partir de 95 %, plus
  de 50 secteurs défectueux ou un nombre qui augmente, un disque dur à partir de 55 °C ou un SSD à
  partir de 75 °C, un avertissement critique NVMe ou une réserve faible, un autotest échoué. La
  température est vérifiée toutes les 5 minutes.
- Les systèmes de fichiers réseau, les disques virtuels et les adaptateurs USB sans SMART ont un badge
  gris ; son info-bulle et le détail disent pourquoi.
- Le badge est une petite icône de pouls : gris quand l’état est inconnu, vert quand le disque va bien,
  orange ou rouge avec la raison la plus grave à côté (par exemple « Usure 87 % », « Secteurs
  défectueux » ou « 61 °C »). Survolez-le pour voir toutes les raisons. La barre de l’espace libre
  devient orange sous 10 % d’espace libre et rouge sous 5 %.

## Paramètres

- **Paramètres → Paramètres…** (**Ctrl+,**) ouvre les paramètres dans une fenêtre à part. À gauche, un
  arbre de sections : Apparence (langue et dates, police, panneaux, couleurs par type de fichier,
  miniatures), Fichiers (copie et déplacement, archives, glisser-déposer), Outils (visionneuse, éditeur externe) et
  Serveurs et sécurité (connexions aux serveurs, mots de passe enregistrés).
- Le champ du haut (**Ctrl+F**) trouve un paramètre dans toutes les sections à la fois, en anglais ou
  dans la langue de la fenêtre, avec ou sans accents ; **Échap** vide le champ.
- **Enregistrer** enregistre et ferme la fenêtre, **Appliquer** enregistre et la laisse ouverte,
  **Annuler** ou la fermeture de la fenêtre abandonne les modifications. L'enregistrement ne change
  que ce que vous avez modifié dans la fenêtre : ce qui a été changé entre-temps dans les menus ou
  dans une autre fenêtre du programme est conservé. Les menus gardent leurs éléments rapides (limite
  de vitesse, choix en cas de conflit, actualisation automatique, éditeur externe) ; ce sont les mêmes
  paramètres.
- Par défaut, la langue suit celle du système (`LC_ALL`, `LC_MESSAGES`, `LANG`). Elle peut être
  choisie dans les Paramètres et change immédiatement. Votre propre traduction
  `~/.config/rolidscp/lang/<langue>.po` est utilisée à la place de celle intégrée.

## Fichiers et journal

- Configuration : `~/.config/rolidscp/` (lisible uniquement par vous).
- **Le programme lancé deux fois :** les deux fenêtres travaillent ensemble. Les serveurs, les
  paramètres et les mots de passe enregistrés par une fenêtre apparaissent dans l'autre en quelques
  secondes, et si les deux les modifient, rien n'est perdu (là où les deux ont modifié la même
  chose, la modification la plus récente l'emporte). Chaque fenêtre enregistre ses propres onglets ;
  la fenêtre fermée en dernier décide des onglets ouverts la prochaine fois.
- **Un fichier de paramètres endommagé** (modifié à la main, tronqué) ne coûte pas tout ce qu'il
  contient : ce qui peut être lu est utilisé, le reste est cité dans la ligne d'erreur, et le
  fichier tel qu'il était est conservé à côté sous `<nom>.damaged`. Revenir à une version antérieure
  puis revenir conserve les paramètres (testé jusqu'à la 0.8.13) ; à partir de la 0.8.20, les
  valeurs que la version antérieure ne connaît pas restent dans les fichiers.
- Journal : `~/.local/state/rolidscp/rolidscp.log` (menu Aide → Ouvrir le journal (diagnostic de
  connexion)). Il ne contient jamais de mots de passe.
- L’AppImage peut être mise à jour avec AppImageUpdate (ou `appimageupdatetool
  RolidSCP-<version>-x86_64.AppImage`) : il trouve la version la plus récente sur GitHub et ne
  télécharge que les parties qui ont changé. La nouvelle AppImage est enregistrée à côté de
  l’ancienne sous le nom de la version ; l’ancienne reste.
