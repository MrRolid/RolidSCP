# Hjælp til RolidSCP

RolidSCP er en filhåndtering med to paneler til lokale diske og SFTP-servere. Hvert panel har faneblade;
et faneblad er ét sted (en lokal mappe eller en mappe på en server) med sit eget navn og sin egen farve.
Alle funktioner findes i menuerne, de fleste også på værktøjslinjen og på linjen med funktionstaster
nederst. Listen over alle tastaturgenveje står på den anden side i dette vindue.

## Paneler og faneblade

- **Tab** skifter mellem venstre og højre panel. Det aktive panel har en tydeligere overskrift.
- **Enter** eller et dobbeltklik åbner en mappe, et arkiv (som skrivebeskyttet mappe) eller en fil i
  dens standardprogram. Programmer og scripts startes aldrig uden at spørge; filer på en server køres
  aldrig. **Backspace** går et niveau op.
- **Ctrl+T** åbner en menu til et nyt faneblad: hjemmemappen, favoritter, gemte servere og seneste
  steder; skriv for at filtrere. **Ctrl+W** lukker fanebladet, **Ctrl+Tab** skifter faneblad. Højreklik
  på et faneblad giver Dupliker, Omdøb, Farve, Luk og Luk andre faneblade; dobbeltklik omdøber det.
  Værktøjstippet for et faneblad viser server, adresse og sti. **Ctrl+K** finder et faneblad eller en
  gemt server efter navn.
- **Ctrl+D** føjer mappen til favoritterne eller fjerner den. Favoritterne og de seneste mapper findes
  i menuen under bjælkens ★-knap (`*` når bjælken er skjult); **Filer → Favoritmapper…** navngiver og
  ordner dem.
- I menuen **Ctrl+T** (og knappen **+** ved fanerne) vises favoritterne altid: **＋ Tilføj den aktuelle
  mappe** tilføjer mappen i det aktive panel (også på en server), ✏ omdøber en favorit og 🗑 fjerner
  den (klik én gang til for at bekræfte, **Esc** går tilbage).
- Begynd at skrive for at filtrere listen; **Esc** rydder filteret.
- **Ctrl+F1**, **Ctrl+F2** og **Ctrl+Shift+F1** skifter mellem kort visning, detaljeret visning og
  miniaturer. **Alt+←** og **Alt+→** går tilbage og frem i mappehistorikken.
- Alle dialoger fungerer ens: **Enter** trykker på den fremhævede knap, **Esc** eller ✕ annullerer,
  **Tab** skifter mellem felterne. En dialog flyttes ved at trække i titellinjen; ⓘ viser mere hjælp.
- Faneblade, farver og mapper gemmes løbende og gendannes ved næste start.
- Over hvert panel er der en bjælke: en rulleliste med mapperne i stien fra roden (**Alt+↓**;
  piletaster, **Enter**, **Esc**), for et lokalt panel også med diskene i denne computer og deres
  størrelse; derefter Tilbage, Frem, Et niveau op, Rodmappe (**`Ctrl+\`**), Hjemmemappe (**Ctrl+H**),
  Opdatér og favoritterne (★). I et arkiv begynder listen med mappen, som arkivet ligger i; i
  søgeresultater er den ikke tilgængelig. **Vis → Bjælke over panelerne** skjuler bjælken; knapperne `/
  .. ~ *` vender så tilbage til stirækken.
- Klik på stien for at skrive en anden. Til højre viser stirækken den ledige plads på disken (en tynd
  bjælke og »Ledig: X af Y«), dens filsystem (ext4, btrfs, nfs4, fuse.sshfs…) og, for diske i denne
  computer, et mærke med diskens tilstand; i et smalt panel udelades først »Ledig:«, så diskens
  størrelse og så filsystemet. I et arkiv viser den disken, som arkivet ligger på. Klik for detaljer.
- Et højreklik på den tomme plads i listen (eller på `..`) giver **Ny fil…**, **Ny mappe**,
  **Opdatér**, **Føj mappen til favoritter**, **Åbn terminal her** og **Diskegenskaber…** (ledig plads,
  filsystem og diskens tilstand).
- I den detaljerede liste adskiller tynde linjer kolonnerne. Træk en linje mellem kolonnetitlerne for at
  ændre bredden på kolonnen til højre for den; et dobbeltklik tilpasser kolonnen til indholdet. Navn
  fylder resten af panelet og bliver derfor bredere med et bredere vindue; i et smalt panel skjules
  Rettigheder og derefter Ændret. Bredderne huskes hver for sig for venstre og højre panel (også efter
  genstart); **Vis → Nulstil kolonnebredder** giver standardværdierne tilbage. Et navn eller en endelse,
  der ikke kan være der, ender med »…«; hold musen over for at se det hele.
- Listerne holder sig selv opdateret. En mappe på denne computer viser ændringer fra andre programmer i
  løbet af et øjeblik. En mappe på en server (eller på et netværksdrev) opdateres, når du vender tilbage
  til den – et andet faneblad, det andet panel eller tilbage til vinduet – hvis listen er ældre end
  5 sekunder, og mens vinduet er aktivt også hvert 30. sekund (**Vis → Automatisk opdatering**: fra,
  10 s, 30 s, 1 min eller 5 min; en mappe med mere end 20 000 elementer højst hvert 5. minut). Markøren,
  markeringerne, rullepositionen, sorteringen og beregnede mappestørrelser bliver stående. Intet
  opdateres, mens du omdøber i listen, trækker eller en dialog er åben, og en tabt forbindelse forsøges
  ikke. Efter dine egne handlinger opdateres alle faneblade, der viser mappen; **Ctrl+R** opdaterer med
  det samme.

## Markering af filer

- **Insert**, **Space** eller **Ctrl+klik** markerer et element; **Ctrl+A** markerer alle eller ingen.
- **Num+** markerer en gruppe efter en maske som `*.jpg;*.png`, **Alt+−** fjerner markeringen af en
  gruppe, **Alt+I** inverterer markeringen.
- Linjen under listen viser, hvor mange elementer der er markeret, og deres størrelse.
- **Alt+Space** (eller **Shift+Space**) beregner størrelsen af mappen under markøren eller af de
  markerede mapper, **Alt+Shift+Enter** af alle mapper i panelet.

## Kopiering, flytning, omdøbning

- **F5** kopierer de markerede elementer (eller elementet under markøren) til mappen i det andet panel,
  **F6** flytter dem. Begge virker mellem diske og servere, også mellem to servere.
- Målmappen for kopiering og flytning kan redigeres, vælges blandt de seneste mål eller vælges med
  **Gennemse…** (på disken eller på serveren).
- Træk filer med musen til det andet panel, til en mappe, til `..` eller til et faneblad (hold dem over
  et faneblad for at skifte til det). Træk kopierer, med **Shift** flyttes, **Esc** annullerer trækket.
  Dialogen Kopiér eller Flyt spørger altid først.
- Når en fil allerede findes i målet, vælger du: spring over, overskriv kun ældre filer, overskriv
  eller genoptag en afbrudt overførsel. Standardvalget kan angives i menuen Indstillinger.
- Filer skrives under et midlertidigt navn og omdøbes, når de er færdige, så en afbrudt overførsel
  aldrig efterlader en halvt skrevet fil under det rigtige navn.
- Genoptagelse fortsætter kun en delfil (`.filepart`) fra den samme kildefil: dens sti, størrelse og
  ændringstid gemmes ved siden af. En delfil fra en anden eller ukendt kilde bliver ikke rørt og
  meldes; vælg **Overskriv** for at starte forfra.
- Flytning mellem diske eller servere sletter kun en kildefil, når den ikke ændrede sig under
  overførslen (størrelse og ændringstid), og kopien har samme størrelse. Med **Kontrollér med
  kontrolsum før kilden slettes** (slået til som standard ved flytning til en server) læses begge
  igen og sammenlignes først; en forskel meldes, og kilden bliver.
- **F2** eller **Shift+F6** omdøber direkte i listen: navnet uden filendelse er markeret, **Enter**
  bekræfter, **Esc** annullerer.
- **F7** opretter en mappe, **F8** sletter (med bekræftelse).
- Overførselskøen kører op til tre overførsler ad gangen. Hastighedsgrænsen findes i kopieringsdialogen.
- **Træk ud til andre programmer** (X11): træk filer eller mapper ud af vinduet, for eksempel til en
  filhåndtering eller en editor. Lokale filer kopieres, med **Shift** flyttes de. Filer fra en server eller
  et arkiv kopieres kun: et program, der understøtter direkte gemning (XDS), får dem downloadet til sin
  mappe gennem overførselskøen; ellers downloades de først til en privat midlertidig mappe (over 1 GiB
  bliver du spurgt), som slettes, når intet bruger den længere, og ved afslutning. Nogle programmer venter
  kun cirka 30 sekunder på filerne, så store downloads kommer måske ikke igennem den vej. Tilbage over
  vinduet fortsætter trækket indeni; **Esc** annullerer det. Ikke tilgængeligt på Wayland.

## Servere

- **Ctrl+O** åbner serverhåndteringen. Angiv serveren som `host` eller `user@host:port`, gem den som
  en profil med et navn og en fanebladsfarve, og forbind med **Enter**.
- Markér en server som **produktionsserver**: dens faneblade får en rød overskrift, og farlige
  handlinger spørger en ekstra gang (sletning kan ikke bekræftes med Enter).
- På en produktionsserver kan kopiering, upload, flytning og oprettelse af en mappe heller ikke
  bekræftes med Enter, og kopiering starter aldrig med Overskriv (Spring eksisterende over er valgt
  på forhånd).
- Serverens nøgle kontrolleres mod `~/.ssh/known_hosts`. En ny nøgle tilføjes først, når du har
  bekræftet dens fingeraftryk; en ændret nøgle afvises.
- Login prøver ssh-agent, dine nøgler og derefter adgangskoden. Gemte adgangskoder opbevares
  krypteret, beskyttet af en hovedadgangskode.
- Gemte adgangskoder låses igen en valgt tid efter sidste brug (menuen Indstillinger → hovedadgangskode,
  60 sekunder som standard); statuslinjen viser den resterende tid. Låsning glemmer også adgangskoderne,
  der holdes til genforbindelse.
- Når forbindelsen afbrydes, forbinder RolidSCP igen af sig selv, og afbrudte overførsler fortsætter.
- **Ctrl+P** åbner systemets terminal (`$TERMINAL` eller `x-terminal-emulator`) i panelets mappe. På en
  server kører den `ssh` ind i mappen med samme kontrol af værtsnøglen som RolidSCP (en ukendt eller
  ændret nøgle afslutter forbindelsen). En gemt adgangskode overføres af en engangshjælper
  (`SSH_ASKPASS`), der kun svarer på spørgsmålet om adgangskoden til den server og kun én gang; er
  adgangskoderne låst, bliver du først spurgt om hovedadgangskoden, ellers spørger ssh selv.
  Produktionsservere spørger, før terminalen åbnes. Adgangskoden havner aldrig på en kommandolinje, i
  en fil eller i loggen.
- **Ctrl+L** skifter det aktive panel tilbage til den lokale disk.

## Visning af filer (F3)

- **F3** åbner filen i fremviseren, et vindue for sig (flere kan være åbne på én gang). Den findes også
  i genvejsmenuen, i menuen Filer og på tastelinjen. Fremviseren kører aldrig noget og indlæser intet
  udefra: HTML, XML og SVG vises som tekst.
- Filer af enhver størrelse åbnes straks: kun delen på skærmen læses. En fil på en server hentes ikke
  hel; en fil i et arkiv pakkes først ud til en midlertidig fil. Linjenumre med **~** er skøn, indtil
  linjerne er talt.
- **1**, **2** og **3** skifter mellem tekst, hexadecimal og billede; binære filer vises selv som hex.
- **W** ombryder linjer, **N** viser linjenumre, **Ctrl+G** går til en linje, en procentdel eller en
  position, **Ctrl+Home** og **Ctrl+End** til begyndelsen og slutningen. Menuerne Tegnkodning og
  Fremhævning ændrer tegnkodningen og sproget for syntaksfremhævningen (slået fra for filer over
  20 MB, se **Indstillinger → Indstillinger… → Fremviser (F3)**).
- **F** følger slutningen af en fil, der vokser, også på en server; at rulle op sætter det på pause.
  Når filen ændres, tilbyder en linje at genindlæse den. En mistet forbindelse vises i en linje, og
  visningen fortsætter af sig selv, når den er tilbage.
- **Ctrl+F** eller **F7** søger efter tekst (store/små bogstaver, hele ord, regulært udtryk); **F3**
  og **Shift+F3** går til næste og forrige forekomst. Hele filen gennemsøges i baggrunden; på en
  server søger `grep` dér, når det kan lade sig gøre, ellers læses filen over SFTP, og søgelinjen
  siger hvilken.
- Markér med musen eller med Shift og pilene; **Ctrl+C** kopierer markeringen (højst 32 MB).
- Billeder (JPEG, PNG, GIF også animeret, WebP, BMP): **F** tilpasser billedet til vinduet, **0** viser
  100 %, hjulet zoomer, træk flytter, **R** og **L** drejer visningen, pilene og **PgUp**/**PgDn** viser
  forrige og næste billede i mappen. Et billede på en server hentes kun op til 50 MB (kan ændres).
- **F4** redigerer filen som i panelet; **Esc** eller **Q** lukker fremviseren.

## Redigering og åbning af filer

- **F4** åbner filen i en ekstern editor. En fil fra en server uploades tilbage efter hver gemning;
  hvis den i mellemtiden er ændret på serveren, bliver du spurgt, hvad der skal ske.
- Den lokale kopi ligger i en privat mappe på disken (`~/.local/state/rolidscp/edit`, kun for dig)
  og slettes, når redigeringen slutter. En upload, der mislykkes af en anden grund end mistet
  forbindelse, prøves igen to gange og meldes derefter. Tidspunktet for den seneste upload vises med
  sekunder.
- En server uden atomisk omdøbning får en ændret fil i trin (originalen flyttes først til side).
  Hvis forbindelsen eller programmet afbrydes undervejs, gøres trinnet færdigt eller fortrydes ved
  næste forbindelse til den server, og du får at vide, hvilke filer det gjaldt.
- **Shift+F4** (også **Filer → Ny fil…** og genvejsmenuen) opretter en ny fil og åbner den i den eksterne
  editor. Navnet under markøren foreslås. På denne computer oprettes den tomme fil med det samme; på en
  server ved første gemning, så at lukke editoren uden at gemme opretter intet. Et eksisterende navn
  åbner den fil til redigering.
- **Shift+Enter** eller genvejsmenuen (højreklik) tilbyder **Åbn med…**, rettigheder
  (**Ctrl+Shift+A**) og kopiering af navn, sti eller `sftp://`-adresse til udklipsholderen.
- Efter en gemning i editoren (**F4**, **Shift+F4**) opdateres størrelsen og datoen for en fil på denne
  computer i panelet.

## Arkiver

- **Alt+F5** pakker de markerede elementer i zip, 7z, tar, tar.gz eller tar.zst, eventuelt med
  adgangskode (zip og 7z).
- **Alt+F9** udpakker et arkiv til det andet panel med beskyttelse mod farlige stier og arkivbomber.
  **Alt+Shift+F9** tester et arkiv.
- **Enter** på et arkiv åbner det som skrivebeskyttet mappe; **F5** kopierer filer ud af det.

## Søgning og kontrolsummer

- **Ctrl+F7** (eller **Ctrl+F**) søger efter navn (maske, tekst eller regulært udtryk) og efter
  indhold. På en server køres søgningen dér med `find` og `grep`, når det er muligt.
- Kontrolsummer (MD5, SHA-1, SHA-256, SHA-512, BLAKE3, CRC32) findes i menuen Filer og i
  genvejsmenuen; de kan gemmes i formatet fra `sha256sum -c`.

## Diskens tilstand (SMART)

- Mærket i stirækken viser diskens tilstand: **OK**, **advarsel** (orange) eller **kritisk** (rød).
  Klik for en forklaring i almindeligt sprog og et råd.
- Detaljerne viser model og serienummer, type (HDD, SSD, NVMe), slid og anslået resterende levetid,
  temperatur, driftstid, antal opstarter, uventede nedlukninger, dårlige og ustabile sektorer,
  overførselsfejl (CRC; som regel et kabel eller stik) og skrevne data. Alle attributter er under
  Avanceret.
- Data kommer fra UDisks2 (uden administratorrettigheder) eller fra `smartctl`, når det kører uden
  dem. De læses i baggrunden, højst én gang om dagen pr. disk (gemt i
  `~/.cache/rolidscp/smart.json`); **Indlæs igen** læser disken nu.
- Advarsel: SSD-slid fra 60 %, dårlige, ustabile eller ulæselige sektorer, en harddisk fra 45 °C
  eller en SSD fra 60 °C. Kritisk: slid fra 95 %, mere end 50 dårlige sektorer eller et stigende
  antal, en harddisk fra 55 °C eller en SSD fra 75 °C, en kritisk NVMe-advarsel eller lav reserve,
  en mislykket selvtest. Temperaturen kontrolleres hvert 5. minut.
- Netværksfilsystemer, virtuelle diske og USB-adaptere uden SMART har et gråt mærke; dets værktøjstip og
  detaljerne siger hvorfor.
- Mærket er et lille pulsikon: gråt, når tilstanden ikke er kendt, grønt, når disken er i orden, orange
  eller rødt med den alvorligste årsag ved siden af (for eksempel »Slid 87 %«, »Dårlige sektorer« eller
  »61 °C«). Hold musen over for at se alle årsager. Bjælken for ledig plads bliver orange under 10 %
  ledig plads og rød under 5 %.

## Indstillinger

- **Indstillinger → Indstillinger…** indeholder sprog, datoformat, skrifttype (indbygget eller fra
  systemet), tekststørrelse, hinting og kontrast, værktøjslinjen, farver efter filtype, miniaturer og
  grænser for arkiver.
- Sproget følger som standard systemet (`LC_ALL`, `LC_MESSAGES`, `LANG`). Det kan vælges i
  Indstillinger og skifter med det samme. Din egen oversættelse `~/.config/rolidscp/lang/<language>.po`
  bruges i stedet for den indbyggede.

## Filer og log

- Konfiguration: `~/.config/rolidscp/` (kun læsbar for dig).
- Log: `~/.local/state/rolidscp/rolidscp.log` (menuen Hjælp → Åbn log (logindiagnostik)). Den
  indeholder aldrig adgangskoder.
