# Hjelp til RolidSCP

RolidSCP er en filbehandler med to paneler for lokale disker og SFTP-servere. Hvert panel har faner; en fane er
ett sted (en lokal mappe eller en mappe på en server) med eget navn og egen farge. Alle funksjoner finnes i
menyene, de fleste også på verktøylinjen og funksjonstastlinjen nederst. Listen over alle
hurtigtaster står på den andre siden i dette vinduet.

## Paneler og faner

- **Tab** bytter mellom venstre og høyre panel. Det aktive panelet har en tydeligere overskrift.
- **Enter** eller et dobbeltklikk åpner en mappe, et arkiv (som skrivebeskyttet mappe) eller en fil i
  standardprogrammet. Programmer og skript startes aldri uten å spørre; filer på en server
  kjøres aldri. **Backspace** går ett nivå opp.
- **Ctrl+T** åpner en meny for en ny fane: hjemmemappen, favoritter, lagrede servere og nylige steder;
  skriv for å filtrere. **Ctrl+W** lukker fanen, **Ctrl+Tab** bytter fane. Høyreklikk på en fane gir
  Dupliser, Gi nytt navn, Farge, Lukk og Lukk andre faner; dobbeltklikk gir den nytt navn. Verktøytipset
  til en fane viser server, adresse og sti. **Ctrl+K** finner en fane eller en lagret server etter navn.
- **Ctrl+D** legger mappen til i favorittene eller fjerner den. Favorittene og de nylige mappene ligger
  i menyen til linjens ★-knapp (`*` når linjen er skjult); **Fil → Favorittmapper…** gir dem navn og
  rekkefølge.
- I menyen **Ctrl+T** (og knappen **+** ved fanene) vises favorittene alltid: **＋ Legg til gjeldende
  mappe** legger til mappen i det aktive panelet (også på en server), ✏ gir en favoritt nytt navn og 🗑
  fjerner den (klikk én gang til for å bekrefte, **Esc** går tilbake).
- Begynn å skrive for å filtrere listen; **Esc** tømmer filteret.
- **Ctrl+F1**, **Ctrl+F2** og **Ctrl+Shift+F1** bytter mellom kort visning, detaljert visning og
  miniatyrbilder. **Alt+←** og **Alt+→** går tilbake og fremover i mappehistorikken.
- Alle dialoger fungerer likt: **Enter** trykker den uthevede knappen, **Esc** eller ✕ avbryter, **Tab**
  går mellom feltene. En dialog flyttes ved å dra i tittellinjen; ⓘ viser mer hjelp.
- Faner, farger og mapper lagres hele tiden og gjenopprettes ved neste oppstart.
- Over hvert panel er det en linje: en nedtrekksliste med mappene i stien fra roten (**Alt+↓**;
  piltaster, **Enter**, **Esc**), for et lokalt panel også med diskene i denne datamaskinen og
  størrelsen deres; deretter Tilbake, Fram, Ett nivå opp, Rotmappe (**`Ctrl+\`**), Hjemmemappe
  (**Ctrl+H**), Oppdater og favorittene (★). I et arkiv begynner listen med mappen arkivet ligger i; i
  søkeresultater er den ikke tilgjengelig. **Vis → Linje over panelene** skjuler linjen; knappene `/ ..
  ~ *` kommer da tilbake i stiraden.
- Klikk på stien for å skrive inn en annen. Til høyre viser stiraden den ledige plassen på disken (en
  tynn stolpe og «Ledig: X av Y»), filsystemet (ext4, btrfs, nfs4, fuse.sshfs…) og, for disker i denne
  datamaskinen, et merke med diskens helse; i et smalt panel utelates først «Ledig:», så diskens
  størrelse og så filsystemet. I et arkiv viser den disken arkivet ligger på. Klikk for detaljer.
- Et høyreklikk på den tomme plassen i listen (eller på `..`) gir **Ny fil…**, **Ny mappe**,
  **Oppdater**, **Legg til mappen i favoritter**, **Åpne terminal her** og **Diskegenskaper…** (ledig
  plass, filsystem og diskens helse).
- I den detaljerte listen skiller tynne linjer kolonnene. Dra en linje mellom kolonnetitlene for å endre
  bredden på kolonnen til høyre for den; et dobbeltklikk tilpasser kolonnen til innholdet. Navn tar
  resten av panelet og blir derfor bredere med et bredere vindu; i et smalt panel skjules Rettigheter og
  deretter Endret. Breddene huskes hver for seg for venstre og høyre panel (også etter omstart); **Vis →
  Tilbakestill kolonnebredder** gir standardverdiene tilbake. Et navn eller en endelse som ikke får
  plass, slutter med «…»; hold musen over for å se hele.
- Listene holder seg oppdatert selv. En mappe på denne datamaskinen viser endringer fra andre programmer
  i løpet av et øyeblikk. En mappe på en server (eller på en nettverksdisk) oppdateres når du kommer
  tilbake til den – en annen fane, det andre panelet eller tilbake til vinduet – hvis listen er eldre
  enn 5 sekunder, og mens vinduet er aktivt også hvert 30. sekund (**Vis → Automatisk oppdatering**: av,
  10 s, 30 s, 1 min eller 5 min; en mappe med mer enn 20 000 elementer høyst hvert 5. minutt). Markøren,
  merkingene, rulleposisjonen, sorteringen og beregnede mappestørrelser blir stående. Ingenting
  oppdateres mens du gir nytt navn i listen, drar eller en dialog er åpen, og en tapt forbindelse
  forsøkes ikke. Etter dine egne operasjoner oppdateres alle faner som viser mappen; **Ctrl+R**
  oppdaterer med en gang.

## Merke filer

- **Insert**, **Space** eller **Ctrl+klikk** merker et element; **Ctrl+A** merker alt eller ingenting.
- **Num+** merker en gruppe etter en maske som `*.jpg;*.png`, **Alt+−** fjerner merkingen av en gruppe,
  **Alt+I** inverterer merkingen.
- Linjen under listen viser hvor mange elementer som er merket, og størrelsen deres.
- **Alt+Space** (eller **Shift+Space**) beregner størrelsen på mappen under markøren eller på de merkede
  mappene, **Alt+Shift+Enter** på alle mapper i panelet.

## Kopiere, flytte, gi nytt navn

- **F5** kopierer de merkede elementene (eller elementet under markøren) til mappen i det andre panelet,
  **F6** flytter dem. Begge virker mellom disker og servere, også mellom to servere.
- Målmappen for kopiering og flytting kan redigeres, velges blant nylige mål eller velges med **Bla
  gjennom…** (på disken eller på serveren).
- Dra filer med musen til det andre panelet, til en mappe, til `..` eller til en fane (hold dem over en
  fane for å bytte til den). Dra kopierer, med **Shift** flytter, **Esc** avbryter dragingen. Dialogen
  Kopier eller Flytt spør alltid først.
- Når en fil allerede finnes i målet, velger du: hopp over, overskriv bare eldre filer, overskriv eller
  gjenoppta en avbrutt overføring. Standardvalget kan angis i Innstillinger-menyen.
- Filer skrives under et midlertidig navn og får det riktige navnet når de er ferdige, slik at en avbrutt
  overføring aldri etterlater en halvskrevet fil under det virkelige navnet.
- Gjenopptak fortsetter bare en delfil (`.filepart`) fra den samme kildefilen: stien, størrelsen og
  endringstiden lagres ved siden av. En delfil fra en annen eller ukjent kilde blir ikke rørt og
  meldes; velg **Overskriv** for å starte forfra.
- Flytting mellom disker eller servere sletter en kildefil bare når den ikke ble endret under
  overføringen (størrelse og endringstid) og kopien har samme størrelse. Med **Kontroller med
  kontrollsum før kilden slettes** (slått på som standard ved flytting til en server) leses begge på
  nytt og sammenlignes først; en forskjell meldes, og kilden blir.
- **F2** eller **Shift+F6** gir nytt navn direkte i listen: navnet uten filendelse er merket,
  **Enter** bekrefter, **Esc** avbryter.
- **F7** oppretter en mappe, **F8** sletter (med bekreftelse).
- Overføringskøen kjører opptil tre overføringer samtidig. Hastighetsgrensen finnes i kopieringsdialogen.
- **Dra ut til andre programmer** (X11): dra filer eller mapper ut av vinduet, for eksempel til en
  filbehandler eller et redigeringsprogram. Lokale filer kopieres, med **Shift** flyttes de. Filer fra en
  server eller et arkiv kopieres bare: et program som støtter direkte lagring (XDS), får dem lastet ned til
  sin mappe gjennom overføringskøen; ellers lastes de først ned til en privat midlertidig mappe (over
  1 GiB blir du spurt), som slettes når ingenting bruker den lenger, og ved avslutning. Noen programmer
  venter bare omtrent 30 sekunder på filene, så store nedlastinger kommer kanskje ikke fram den veien.
  Tilbake over vinduet fortsetter dragingen inne i det; **Esc** avbryter. Ikke tilgjengelig på Wayland.

## Servere

- **Ctrl+O** åpner Serverbehandling. Skriv inn serveren som `host` eller `user@host:port`, lagre den som en
  profil med navn og fanefarge, og koble til med **Enter**.
- Merk en server som **produksjonsserver**: fanene får rød overskrift, og farlige handlinger spør
  én gang til (sletting kan ikke bekreftes med Enter).
- På en produksjonsserver kan heller ikke kopiering, opplasting, flytting og oppretting av en mappe
  bekreftes med Enter, og kopiering starter aldri med Overskriv (Hopp over eksisterende er
  forhåndsvalgt).
- Servernøkkelen kontrolleres mot `~/.ssh/known_hosts`. En ny nøkkel legges til først når du har
  bekreftet fingeravtrykket; en endret nøkkel avvises.
- Innloggingen prøver ssh-agent, nøklene dine og deretter passordet. Lagrede passord lagres kryptert,
  beskyttet av et hovedpassord.
- Lagrede passord låses igjen en valgt tid etter siste bruk (menyen Innstillinger → hovedpassord, 60
  sekunder som standard); statuslinjen viser gjenværende tid. Låsing glemmer også passordene som holdes
  for gjentilkobling.
- Når tilkoblingen brytes, kobler RolidSCP til på nytt av seg selv, og avbrutte overføringer fortsetter.
- **Ctrl+P** åpner systemterminalen (`$TERMINAL` eller `x-terminal-emulator`) i panelets mappe. På en
  server kjører den `ssh` inn i mappen, med samme kontroll av vertsnøkkelen som RolidSCP (en ukjent
  eller endret nøkkel avslutter forbindelsen). Et lagret passord sendes av en engangshjelper
  (`SSH_ASKPASS`) som bare svarer på passordspørsmålet for den serveren, én gang; er passordene låst,
  blir du først spurt om hovedpassordet, ellers spør ssh selv. Produksjonsservere spør før terminalen
  åpnes. Passordet havner aldri på en kommandolinje, i en fil eller i loggen.
- **Ctrl+L** setter det aktive panelet tilbake til den lokale disken.

## Vise filer (F3)

- **F3** åpner filen i fremviseren, et eget vindu (flere kan være åpne samtidig). Den finnes også i
  hurtigmenyen, i Fil-menyen og på tastelinjen. Fremviseren kjører aldri noe og laster ingenting
  utenfra: HTML, XML og SVG vises som tekst.
- Filer av enhver størrelse åpnes straks: bare delen på skjermen leses. En fil på en server lastes
  ikke ned i sin helhet; en fil i et arkiv pakkes først ut til en midlertidig fil. Linjenumre med
  **~** er anslag til linjene er talt.
- **1**, **2** og **3** bytter mellom tekst, heksadesimalt og bilde; binærfiler vises heksadesimalt av
  seg selv.
- **W** bryter linjer, **N** viser linjenumre, **Ctrl+G** går til en linje, en prosent eller en
  posisjon, **Ctrl+Home** og **Ctrl+End** til begynnelsen og slutten. Menyene Tegnkoding og Utheving
  endrer tegnkodingen og språket for syntaksuthevingen (av for filer over 20 MB, se
  **Innstillinger → Innstillinger … → Fremviser (F3)**).
- **F** følger slutten av en fil som vokser, også på en server; å rulle opp setter det på pause. Når
  filen endres, tilbyr en linje å laste den på nytt. En tapt tilkobling vises i en linje, og visningen
  fortsetter av seg selv når den er tilbake.
- **Ctrl+F** eller **F7** søker etter tekst (store/små bokstaver, hele ord, regulært uttrykk); **F3**
  og **Shift+F3** går til neste og forrige treff. Hele filen søkes gjennom i bakgrunnen; på en server
  søker `grep` der når det går, ellers leses filen over SFTP, og søkelinjen sier hvilken.
- Merk med musen eller med Shift og pilene; **Ctrl+C** kopierer merkingen (høyst 32 MB).
- Bilder (JPEG, PNG, GIF også animert, WebP, BMP): **F** tilpasser bildet til vinduet, **0** viser
  100 %, hjulet zoomer, dra flytter, **R** og **L** roterer visningen, pilene og **PgUp**/**PgDn**
  viser forrige og neste bilde i mappen. Et bilde på en server lastes bare ned opptil 50 MB
  (kan endres).
- **F4** redigerer filen som i panelet; **Esc** eller **Q** lukker fremviseren.

## Redigere og åpne filer

- **F4** åpner filen i et eksternt redigeringsprogram. En fil fra en server lastes opp igjen etter hver
  lagring; hvis den er endret på serveren i mellomtiden, blir du spurt hva som skal skje.
- Den lokale kopien ligger i en privat mappe på disken (`~/.local/state/rolidscp/edit`, bare for
  deg) og slettes når redigeringen avsluttes. En opplasting som mislykkes av en annen grunn enn
  mistet forbindelse, prøves igjen to ganger og meldes deretter. Tidspunktet for siste opplasting
  vises med sekunder.
- En server uten atomisk navneendring får en endret fil i trinn (originalen flyttes først til side).
  Brytes forbindelsen eller programmet underveis, fullføres eller angres trinnet ved neste
  tilkobling til den serveren, og du får vite hvilke filer det gjaldt.
- **Shift+F4** (også **Fil → Ny fil…** og hurtigmenyen) oppretter en ny fil og åpner den i det eksterne
  redigeringsprogrammet. Navnet under markøren foreslås. På denne datamaskinen opprettes den tomme filen
  med en gang; på en server ved første lagring, så å lukke uten å lagre oppretter ingenting. Et
  eksisterende navn åpner den filen for redigering.
- **Shift+Enter** eller hurtigmenyen (høyreklikk) tilbyr **Åpne med …**, rettigheter
  (**Ctrl+Shift+A**) og kopiering av navn, sti eller `sftp://`-adresse til utklippstavlen.
- Etter lagring i redigeringsprogrammet (**F4**, **Shift+F4**) oppdateres størrelsen og datoen for en
  fil på denne datamaskinen i panelet.

## Arkiver

- **Alt+F5** pakker de merkede elementene i zip, 7z, tar, tar.gz eller tar.zst, eventuelt med passord
  (zip og 7z).
- **Alt+F9** pakker ut et arkiv i det andre panelet, med beskyttelse mot farlige stier og
  arkivbomber. **Alt+Shift+F9** tester et arkiv.
- **Enter** på et arkiv åpner det som en skrivebeskyttet mappe; **F5** kopierer filer ut av det.

## Søk og kontrollsummer

- **Ctrl+F7** (eller **Ctrl+F**) søker etter navn (maske, tekst eller regulært uttrykk) og etter innhold.
  På en server kjøres søket der med `find` og `grep` når det er mulig.
- Kontrollsummer (MD5, SHA-1, SHA-256, SHA-512, BLAKE3, CRC32) finnes i Fil-menyen og i
  hurtigmenyen; de kan lagres i formatet til `sha256sum -c`.

## Diskens helse (SMART)

- Merket i stiraden viser diskens tilstand: **OK**, **advarsel** (oransje) eller **kritisk** (rød).
  Klikk for en forklaring på vanlig språk og et råd.
- Detaljene viser modell og serienummer, type (HDD, SSD, NVMe), slitasje og anslått gjenværende
  levetid, temperatur, driftstid, antall oppstarter, uventede avslutninger, dårlige og ustabile
  sektorer, overføringsfeil (CRC; som regel en kabel eller kontakt) og skrevne data. Alle
  attributtene er under Avansert.
- Dataene kommer fra UDisks2 (uten administratorrettigheter), eller fra `smartctl` når det kjører uten
  dem. De leses i bakgrunnen, høyst én gang om dagen per disk (lagret i
  `~/.cache/rolidscp/smart.json`); **Last inn på nytt** leser disken nå.
- Advarsel: SSD-slitasje fra 60 %, dårlige, ustabile eller uleselige sektorer, en harddisk fra 45 °C
  eller en SSD fra 60 °C. Kritisk: slitasje fra 95 %, mer enn 50 dårlige sektorer eller et økende
  antall, en harddisk fra 55 °C eller en SSD fra 75 °C, en kritisk NVMe-advarsel eller lite reserve,
  en mislykket selvtest. Temperaturen kontrolleres hvert 5. minutt.
- Nettverksfilsystemer, virtuelle disker og USB-adaptere uten SMART har et grått merke; verktøytipset og
  detaljene sier hvorfor.
- Merket er et lite pulsikon: grått når tilstanden ikke er kjent, grønt når disken er i orden, oransje
  eller rødt med den alvorligste grunnen ved siden av (for eksempel «Slitasje 87 %», «Dårlige sektorer»
  eller «61 °C»). Hold musen over for å se alle grunnene. Linjen for ledig plass blir oransje under 10 %
  ledig plass og rød under 5 %.

## Innstillinger

- **Innstillinger → Innstillinger …** inneholder språk, datoformat, skrift (innebygd eller fra
  systemet), tekststørrelse, hinting og kontrast, verktøylinjen, farger etter filtype, miniatyrbilder og
  grenser for arkiver.
- Språket følger systemet som standard (`LC_ALL`, `LC_MESSAGES`, `LANG`). Det kan velges i
  Innstillinger og endres med en gang. Din egen oversettelse `~/.config/rolidscp/lang/<language>.po` brukes
  i stedet for den innebygde.

## Filer og logg

- Konfigurasjon: `~/.config/rolidscp/` (kan bare leses av deg).
- Logg: `~/.local/state/rolidscp/rolidscp.log` (Hjelp-menyen → Åpne logg (innloggingsdiagnostikk)). Den inneholder aldri passord.
