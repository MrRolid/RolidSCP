# Nápověda RolidSCP

RolidSCP je dvoupanelový správce souborů pro lokální disky a SFTP servery. Každý panel má záložky;
záložka je jedno místo (lokální složka nebo složka na serveru) s vlastním názvem a barvou. Všechny
funkce jsou v menu, většina i na nástrojové liště a ve spodní liště funkčních kláves. Seznam všech
klávesových zkratek je na druhé stránce tohoto okna.

**F10** otevře lištu menu bez myši: **←** **→** přechází mezi menu, **↑** **↓** po položkách, **→** do
podnabídky, **Enter** položku vybere, **Esc** menu zavře.

Se čtečkou obrazovky (Orca) seznam čte položku pod kurzorem (název, „složka“ nebo velikost a zda je
označená), tlačítka a dialogy své názvy.

## Panely a záložky

- **Tab** přepíná mezi levým a pravým panelem. Aktivní panel má výraznější hlavičku.
- **Enter** nebo dvojklik otevře složku, archiv (jako složku jen pro čtení) nebo soubor ve výchozí
  aplikaci. Programy a skripty se nikdy nespustí bez dotazu; soubory na serveru se nikdy nespouštějí.
  **Backspace** jde o úroveň výš.
- **Ctrl+T** otevře nabídku pro novou záložku: domovská složka, oblíbené složky, uložené servery a
  poslední místa; psaním ji filtrujete. **Ctrl+W** záložku zavře, **Ctrl+Tab** přepíná záložky. Pravé
  tlačítko na záložce nabídne Duplikovat, Přejmenovat, Barvu, Zavřít a Zavřít ostatní záložky; dvojklik
  ji přejmenuje. Bublina nad záložkou ukáže server, adresu a cestu. **Ctrl+K** najde záložku nebo
  uložený server podle názvu.
- Kontextová menu z klávesnice: **Shift+F10** nebo klávesa **Menu** otevře menu položky pod kurzorem
  (na `..` nebo v prázdné složce menu složky), **Ctrl+Shift+F10** nebo **Ctrl+Menu** menu aktivní
  záložky. **↑ ↓** přechází, **→** otevře podnabídku (ve Změnit barvu se barvami prochází **↓** nebo
  **Tab**), **Enter** vybere, **Esc** zavře. Obojí je i v liště menu: **Soubor → Kontextové menu
  položky…** a **Nastavení → Menu záložky…**.
- **Ctrl+D** přidá složku do oblíbených nebo ji z nich odebere. Oblíbené a poslední složky jsou pod
  tlačítkem ★ lišty (při skryté liště `*`); **Soubor → Oblíbené složky…** je pojmenuje a seřadí.
- V nabídce **Ctrl+T** (a tlačítka **+** u záložek) jsou oblíbené vždy vidět: **＋ Přidat aktuální
  složku** přidá složku aktivního panelu (i na serveru), ✏ oblíbenou přejmenuje a 🗑 ji odebere
  (potvrďte ještě jedním kliknutím, **Esc** vrátí zpět).
- Psaním filtrujete seznam; **Esc** filtr zruší.
- **Ctrl+F1**, **Ctrl+F2** a **Ctrl+Shift+F1** přepínají stručný výpis, podrobný výpis a
  miniatury. **Alt+←** a **Alt+→** jdou zpět a vpřed v historii složek.
- Všechny dialogy se ovládají stejně: **Enter** stiskne zvýrazněné tlačítko, **Esc** nebo ✕ zruší,
  **Tab** přechází mezi poli. Dialog lze posunout tažením za titulek; ⓘ ukáže další nápovědu.
- Záložky, barvy a složky se průběžně ukládají a při dalším spuštění se obnoví.
- Nad každým panelem je lišta: rozbalovací pole se složkami na cestě od kořene (**Alt+↓**; šipky,
  **Enter**, **Esc**), u místního panelu i s disky tohoto počítače a jejich velikostí; dále Zpět,
  Vpřed, O úroveň výš, Kořenová složka (**`Ctrl+\`**), Domovská složka (**Ctrl+H**), Obnovit a oblíbené
  (★). V archivu začíná pole složkou, ve které archiv leží; ve výsledcích hledání není dostupné.
  **Zobrazit → Lišta nad panely** lištu skryje; tlačítka `/ .. ~ *` se pak vrátí do řádku s cestou.
- Každá část cesty je tlačítko: kliknutí otevře tu složku (i na serveru a v archivu; kliknutí na
  složku, ve které jste, ji obnoví), prostřední tlačítko ji otevře v nové záložce vedle aktuální a
  část pod myší se zvýrazní. Cesta delší než řádek se zkrátí zleva třemi tečkami „…“ (kliknutí na ně
  otevře rozbalovací pole cesty); poslední část zůstane vždy celá. Cestu napíšete po dvojkliku na
  prázdné místo řádku, kliknutím na tužku na jeho konci nebo zkratkou **Ctrl+G**.
- Vpravo řádek s cestou ukazuje volné místo na disku (tenký pruh a „Volné: X z Y“), jeho souborový
  systém (ext4, btrfs, nfs4, fuse.sshfs…) a u disků tohoto počítače odznak se stavem disku; v úzkém
  panelu se vynechá nejdřív „Volné:“, pak velikost disku a pak souborový systém. V archivu ukazuje
  disk, na kterém archiv leží. Kliknutím se otevřou podrobnosti.
- Pravé tlačítko na prázdném místě seznamu (nebo na `..`) nabídne **Nový soubor…**, **Nová složka**,
  **Obnovit**, **Přidat složku do oblíbených**, **Otevřít terminál zde** a **Vlastnosti disku…** (volné
  místo, souborový systém a stav disku).
- V podrobném výpisu oddělují sloupce tenké čáry. Tažením čáry mezi nadpisy sloupců změníte šířku
  sloupce napravo od ní; dvojklik přizpůsobí sloupec obsahu. Název zabírá zbytek panelu, takže se širším
  oknem roste; v úzkém panelu se skryjí Práva a potom Změněno. Šířky se pamatují zvlášť pro levý a pravý
  panel (i po restartu); **Zobrazit → Výchozí šířky sloupců** vrátí výchozí stav. Název nebo přípona,
  která se nevejde, končí „…“; po najetí myší se ukáže celá.
- Výpisy se obnovují samy. Složka na tomto počítači ukáže změny z jiných programů během chvilky. Složka
  na serveru (nebo na síťovém disku) se obnoví, když se k ní vrátíte – jiná záložka, druhý panel nebo
  návrat do okna –, je-li výpis starší než 5 sekund, a dokud je okno aktivní, také každých 30 sekund
  (**Zobrazit → Automatické obnovování**: vypnuto, 10 s, 30 s, 1 min nebo 5 min; složka s více než
  20 000 položkami nejvýš jednou za 5 minut). Kurzor, označení, posun, řazení i spočítané velikosti
  složek zůstanou. Během přejmenování v seznamu, tažení nebo otevřeného dialogu se nic neobnovuje a
  ztracené spojení se nezkouší. Po vašich vlastních operacích se obnoví všechny záložky s danou složkou;
  **Ctrl+R** obnoví hned.

## Označování souborů

- **Insert**, **mezerník** nebo **Ctrl+klik** označí položku; **Ctrl+A** označí vše nebo nic.
- **Shift+klik** označí položky od naposledy kliknuté nebo označené až po tu, na kterou klikáte.
  **Shift** se **↑ ↓ PgUp PgDn Home End** označuje odtud až po kurzor, jak se pohybuje; návrat zpět
  odznačí jen to, co označil tento rozsah. **Nástroje → Označit k začátku / Označit ke konci** dělá
  totéž co **Shift+Home** a **Shift+End**. Klik bez klávesy posune kurzor a označení ponechá. Začátek
  rozsahu se po obnovení, seřazení nebo rychlém filtru najde znovu podle názvu; `..` se nikdy
  neoznačí. Tažení dál kopíruje a **Shift** držený při puštění souborů je přesune (samotný Shift+klik
  nikdy nic nepřesouvá).
- **Num+** označí skupinu podle masky, např. `*.jpg;*.png`, **Alt+−** označení skupiny zruší,
  **Alt+I** obrátí výběr.
- Dialogy masky **Num+** a **Alt+−** si pamatují poslední masku a volbu **Včetně složek**, každý
  zvlášť a i po restartu. Na serveru označeném jako produkční se dialog otevře vždy s vypnutou
  volbou **Včetně složek** a to, co v něm zvolíte, se nezapamatuje.
- Řádek pod seznamem ukazuje počet označených položek a jejich velikost.
- **Alt+mezerník** (nebo **Shift+mezerník**) spočítá velikost složky pod kurzorem nebo označených
  složek, **Alt+Shift+Enter** všech složek v panelu.

## Kopírování, přesun, přejmenování

- **F5** zkopíruje označené položky (nebo položku pod kurzorem) do složky druhého panelu, **F6** je
  přesune. Obojí funguje mezi disky i servery, také mezi dvěma servery.
- Cílovou složku kopírování a přesunu lze upravit, vybrat z posledních cílů nebo zvolit tlačítkem
  **Procházet…** (na disku i na serveru).
- Soubory můžete táhnout myší do druhého panelu, na složku, na `..` nebo na záložku (podržením nad
  záložkou se přepne). Tažení kopíruje, se **Shiftem** přesouvá, **Esc** tažení zruší. Vždy se nejdřív
  zeptá dialog Kopírovat nebo Přesunout.
- Když soubor v cíli už existuje, vyberete: přeskočit, přepsat jen starší, přepsat, nebo navázat
  přerušený přenos. Výchozí volbu nastavíte v menu Nastavení.
- Soubory se zapisují pod dočasným názvem a přejmenují se až po dokončení, takže přerušený přenos
  nikdy nenechá napůl zapsaný soubor pod skutečným názvem.
- Navazuje se jen rozpracovaný soubor (`.filepart`) téhož zdrojového souboru: vedle něj se ukládá
  jeho cesta, velikost a čas změny. Rozpracovaný soubor jiného nebo neznámého zdroje zůstane
  nedotčený a program to ohlásí; pro nový začátek zvolte **Přepsat**.
- Když se cílový disk zaplní nebo dojde disková kvóta, přenos se zastaví a řekne to; starší soubor
  stejného jména zůstane, jak byl, a zapsaná část se ponechá. Uvolněte místo a ve frontě přenosů
  stiskněte **Zkusit znovu**: přenos pokračuje tam, kde skončil. Server často hlásí jen „Failure“;
  program se pak podívá na volné místo na serveru.
- Přesun mezi disky nebo servery smaže zdrojový soubor, jen když se během přenosu nezměnil (velikost
  a čas změny) a kopie má stejnou velikost. S volbou **Před smazáním zdroje ověřit kontrolním
  součtem** (při přesunu na server výchozí zapnuto) se navíc oba znovu přečtou a porovnají; při
  rozdílu to program ohlásí a zdroj zůstane.
- **F2** nebo **Shift+F6** přejmenuje přímo v seznamu: název bez přípony je vybraný, **Enter**
  potvrdí, **Esc** zruší.
- **F7** vytvoří složku. **F8** přesune soubory na discích tohoto počítače do koše prostředí
  (s potvrzením): správci souborů ho ukazují a soubory umí vrátit; soubor na jiném disku jde do
  vlastního koše toho disku, a kde žádný není (disk jen pro čtení), nic se nesmaže a program to řekne.
  **Shift+F8** maže natrvalo; soubory na serveru se mažou vždy natrvalo (otázka říká NA SERVERU). Koš
  jde vypnout v **Nastavení → Kopírování a přesun**. Mazání běží ve frontě přenosů s počtem smazaných položek;
  **Zrušit** ho zastaví (co už je smazané, zůstane smazané).
- Roury (FIFO), sockety, zařízení a symbolické odkazy, které nikam nevedou (cíl chybí nebo vede
  v kruhu), se nekopírují: přeskočí se a vypíšou ve zprávě na konci, zbytek kopie pokračuje.
- Neobvyklé názvy se kopírují přesně: konce řádků, zpětné lomítko, názvy o 255 bajtech a na discích
  tohoto počítače i názvy, které nejsou platné UTF-8. Seznam je ukáže na jednom řádku: řídicí znaky
  jako `\n` nebo `\t`, každý neplatný bajt jako `\xFF`. Název, který není platné UTF-8, na server
  poslat nejde (SFTP bere názvy jako text); program to oznámí.
- Fronta přenosů běží nejvýš se třemi přenosy současně. Omezení rychlosti je v dialogu kopírování.
- **Co kopie zachová** kromě obsahu: oprávnění (čtení, zápis, spouštění; setuid, setgid a sticky se
  vynechávají) a čas změny. Nezachová vlastníka a skupinu (kopie patří uživateli, který ji zapisuje),
  ACL, rozšířené atributy (ani štítky SELinuxu a capabilities souborů) ani pevné odkazy (z každého
  odkazu je samostatný soubor). SFTP ACL ani rozšířené atributy přenést vůbec neumí. Přepsaný soubor je
  nahrazen novým, takže zmizí i jeho vlastní ACL a atributy; nový soubor dostane to, co složka dává
  novým souborům (například výchozí ACL).
- **Časy** se zobrazují v časovém pásmu tohoto počítače, i časy souborů na serveru, ať má server
  pásmo jakékoli. **Přepsat jen starší** porovnává časy na sekundu a nezávisle na časových pásmech.
  Kopie si nese čas zdroje, takže špatné hodiny serveru u zkopírovaných souborů nevadí; vadí u souborů
  změněných na serveru, které dostanou čas serveru: když jdou hodiny serveru o hodinu napřed, vypadá
  soubor změněný na něm o hodinu novější.
- **Tažení ven do jiných aplikací** (X11): soubory nebo složky vytáhněte z okna, třeba do správce
  souborů nebo editoru. Lokální soubory se kopírují, se **Shiftem** přesunou. Soubory ze serveru nebo
  z archivu se jen kopírují: aplikace, která umí přímé uložení (XDS), je dostane stažené do své složky
  přes frontu přenosů; jinak se nejdřív stáhnou do soukromé dočasné složky (nad 1 GiB se aplikace
  zeptá), která se smaže, až ji nic nepoužívá, a při ukončení. Některé aplikace na soubory čekají jen
  asi 30 sekund, velké stahování tudy proto nemusí projít. Po návratu nad okno tažení pokračuje uvnitř;
  **Esc** ho zruší. Pod Waylandem viz **Přetahování pod Waylandem** níže.

## Přetahování pod Waylandem <!-- wayland -->

- Pod Waylandem zatím nejde přetahovat soubory do jiných programů ani z nich (do správce souborů,
  poštovního programu, chatu), ani jedním směrem. Knihovna oken, na které je program postavený, pod
  Waylandem přetahování nemá; má ho až její příští verze a program ji použije, až bude hotová.
  Přetahování mezi panely a záložkami funguje.
- **Řešení:** zobrazit okno přes XWayland jako okno X11. Přetahování pak funguje všude, kde ho
  prostředí předává mezi programy pro X11 a pro Wayland. GNOME to dělá oběma směry (vyzkoušeno);
  v testu s KDE Plasma (KWin 6.3) šly soubory do programu přetáhnout, ale ne z něj, stejně jako
  z jiných programů pro X11.
- Zapněte **Nastavení → Přetahování → Pod Waylandem používat X11 (XWayland)** a spusťte program znovu.
  Ve výchozím stavu je volba vypnutá. Nebo program jednou spusťte z terminálu takto:
  `WAYLAND_DISPLAY= rolidscp` (deb a rpm), nebo `WAYLAND_DISPLAY= ./RolidSCP-0.9.1-x86_64.AppImage` se
  jménem vašeho AppImage.
- **Co tím ztratíte:** písmo je méně ostré při zlomkovém škálování (například 125 % nebo 150 %):
  XWayland kreslí okno v měřítku 1 a prostředí ho roztáhne; při 100 % a 200 % zůstává ostré. U monitorů
  s různým škálováním má okno jedno měřítko pro všechny. **Jiné programy pro X11 mohou číst, co do okna
  píšete, i hlavní heslo**; okno pro Wayland programy od sebe odděluje. Schránka a vstupní metody jdou
  přes XWayland.
- Když pod Waylandem poprvé přetáhnete soubory ven z okna, řádek dole v okně řekne, že to nefunguje;
  **Co dělat…** otevře tuto část nápovědy. Ukáže ji i **Nápověda → Přetahování pod Waylandem…**, spolu
  s tím, na čem okno právě běží.

## Servery

- **Ctrl+O** otevře správce serverů. Server zadáte jako `host` nebo `user@host:port`, uložíte ho
  jako profil s názvem a barvou záložky a připojíte se **Enterem**.
- **Duplikovat…** pod seznamem uložených serverů vytvoří kopii se všemi nastaveními pod novým názvem
  („Web (kopie)“). Když má server uložené heslo, vybereš, jestli se zkopíruje také (k tomu je potřeba
  hlavní heslo); standardně ne. Neuložené změny ve formuláři se nekopírují.
- Server můžete označit jako **produkční**: jeho záložky mají červenou hlavičku a nebezpečné akce se
  ptají ještě jednou (mazání nejde potvrdit Enterem).
- Na produkčním serveru nejde Enterem potvrdit ani kopírování, nahrávání, přesun a vytvoření složky
  a kopírování nikdy nezačíná volbou Přepsat (předvybrané je Přeskočit existující).
- **Připojit přes** jiný uložený server (přestupní server, v OpenSSH ProxyJump): program se nejdřív
  připojí k tomuto serveru, jeho vlastním klíčem nebo uloženým heslem, a odtud k cílovému. Adresa
  cílového serveru je pak ta, kterou vidí přestupní server, například adresa v jeho místní síti.
  Kontrolují se klíče obou serverů a dotaz na neznámý klíč řekne, kterého z nich se týká. Přestupní
  server může mít vlastní přestupní server (nejvýš čtyři za sebou). Po výpadku se spojení obnoví
  stejnou cestou a stejně vede i terminál (**Ctrl+P**). Uložený server, přes který vedou jiné, nejde
  smazat, dokud jim nevyberete jinou cestu.
- Klíč serveru se kontroluje proti dvěma seznamům: vlastnímu seznamu známých serverů programu
  (`~/.config/rolidscp/known_hosts`) a `~/.ssh/known_hosts`, který program jen čte a nikdy nemění.
  Nový klíč se přidá do vlastního seznamu programu jen po potvrzení otisku; změněný klíč se odmítne,
  stejně jako klíč označený `@revoked`. Když se oba seznamy u serveru neshodují (jeden má klíč, který
  server poslal, druhý jiný), program se nepřipojí a vypíše oba řádky: zeptejte se správce serveru,
  který klíč je správný, a špatný řádek smažte.
- Přihlášení zkouší ssh-agent, vaše klíče a pak heslo. Uložená hesla jsou šifrovaná a chráněná
  hlavním heslem.
- Uložená hesla se znovu zamknou po nastavené době od posledního použití (nabídka Nastavení → hlavní
  heslo, výchozí 60 sekund); stavový řádek ukazuje zbývající čas. Zamčení smaže i hesla držená pro
  znovupřipojení.
- Když spojení spadne, RolidSCP se sám připojí znovu a přerušené přenosy pokračují. Přenos
  pokračuje, dokud se mezi výpadky posouvá; po pěti výpadcích za sebou bez posunu se zastaví a
  můžete ho spustit znovu z fronty.
- Když ze serveru 15 sekund nic nepřijde, ač na něj program čeká, bere se spojení jako mrtvé a hned
  se naváže znovu (krátká zkouška po 5 sekundách odliší pomalý server od mrtvé linky). V prvních
  třech minutách po výpadku se to zkouší každých pár sekund, pak každých 10 sekund, po deseti
  minutách každých 30 sekund. Nahrávání pokračuje od poslední části, kterou server potvrdil.
- Totéž platí, když je síťový kabel na několik minut vytažený nebo server usne: program to zkouší
  dál a přenos pokračuje, jakmile je server znovu dostupný.
- Na serveru bez shellu nebo bez některých nástrojů (`du`, `find`, `df`, `sha256sum`, třeba BusyBox
  nebo účet jen pro SFTP) se vše dělá jen přes SFTP: velikosti složek a hledání procházením složek,
  kontrolní součty čtením souborů.
- Vadný nebo zlomyslný server nemůže program přimět zapsat mimo cílovou složku: názvy s `/`, `..`
  nebo prázdné se odmítnou, řídicí znaky v názvech se zobrazí jako `\xNN`. Výpis složky s víc než 5
  miliony položek se zastaví a soubor, který server posílá daleko za jeho velikost, se nepřevezme.
- **Ctrl+P** otevře systémový terminál (`$TERMINAL` nebo `x-terminal-emulator`) ve složce panelu.
  Na serveru spustí `ssh` do této složky se stejnou kontrolou klíče serveru jako RolidSCP (neznámý
  nebo změněný klíč spojení ukončí). Uložené heslo předá jednorázový pomocník (`SSH_ASKPASS`), který
  odpoví jen na dotaz na heslo k tomuto serveru, a to jednou; jsou-li hesla zamčená, nejdřív se
  zeptá na hlavní heslo, jinak se ssh zeptá samo. U produkčního serveru se před otevřením zeptá.
  Heslo nikdy nejde do příkazové řádky, souboru ani logu.
- Po připojení se program zeptá serveru na jeho čas (`date -u +%s`, jen tam, kde má server shell,
  nejvýš 5 sekund). Když se hodiny serveru liší o víc než minutu, řekne to jednou řádek dole, ukáže to
  bublina záložky a dialogy kopírování a přesunu varují pod „Přepsat jen starší“: soubory změněné na
  tom serveru mají časy posunuté o stejně. Varování lze vypnout v **Nastavení → Připojení k
  serverům**.
- **Ctrl+L** přepne aktivní panel zpět na lokální disk.

## Prohlížení souborů (F3)

- **F3** otevře soubor v prohlížeči, ve vlastním okně (otevřít jich lze více najednou). Najdete ho i v
  místní nabídce, v menu Soubor a na liště kláves. Prohlížeč nikdy nic nespouští a nic nenačítá
  zvenčí: HTML, XML i SVG ukazuje jako text.
- Soubory libovolné velikosti se otevřou hned: čte se jen část na obrazovce. Soubor na serveru se
  nestahuje celý, soubor v archivu se nejdřív rozbalí do dočasného souboru. Čísla řádků s **~** jsou
  odhad, dokud se řádky nespočítají.
- **1**, **2** a **3** přepínají text, šestnáctkový výpis a obrázek; binární soubory se samy ukážou
  šestnáctkově.
- **W** zalamuje řádky, **N** ukáže čísla řádků, **Ctrl+G** skočí na řádek, procento nebo pozici,
  **Ctrl+Home** a **Ctrl+End** na začátek a konec. Menu Kódování a Zvýraznění mění kódování a jazyk
  zvýraznění syntaxe (u souborů nad 20 MB je vypnuté, viz **Nastavení → Nastavení… → Prohlížeč (F3)**).
- **F** sleduje konec rostoucího souboru, i na serveru; posun nahoru sledování pozastaví. Když se
  soubor změní, lišta nabídne jeho znovunačtení. Ztracené spojení ukáže lišta a zobrazení samo
  pokračuje, jakmile je spojení zpět.
- **Ctrl+F** nebo **F7** hledá text (velikost písmen, celá slova, regulární výraz); **F3** a
  **Shift+F3** jdou na další a předchozí výskyt. Celý soubor se prohledává na pozadí; na serveru
  hledá `grep` přímo tam, když to jde, jinak se soubor čte přes SFTP, a lišta hledání to uvádí.
- Vybírá se myší nebo Shiftem a šipkami; **Ctrl+C** zkopíruje výběr (nejvýše 32 MB).
- Obrázky (JPEG, PNG, GIF i animovaný, WebP, BMP): **F** přizpůsobí obrázek oknu, **0** ukáže 100 %,
  kolečko přibližuje, tažení posouvá, **R** a **L** otáčejí zobrazení, šipky a **PgUp**/**PgDn**
  ukážou předchozí a další obrázek ve složce. Obrázek ze serveru se stáhne jen do 50 MB (lze změnit).
- **F4** soubor upraví jako v panelu; **Esc** nebo **Q** prohlížeč zavře.

## Úpravy a otevírání souborů

- **F4** otevře soubor v externím editoru. Soubor ze serveru se po každém uložení nahraje zpět;
  pokud se mezitím na serveru změnil, program se zeptá, co dělat.
- Místní kopie je v soukromé složce na disku (`~/.local/state/rolidscp/edit`, jen pro vás) a po
  skončení úprav se smaže. Nahrání, které selže z jiného důvodu než ztráty spojení, se dvakrát
  zopakuje a pak se ohlásí. Čas posledního nahrání se ukazuje na sekundy.
- Server bez atomického přejmenování dostane změněný soubor po krocích (původní se nejdřív odsune
  stranou). Když se mezitím přeruší spojení nebo program, krok se při dalším připojení k tomu
  serveru dokončí nebo vrátí a program řekne, kterých souborů se to týkalo.
- **Shift+F4** (také **Soubor → Nový soubor…** a místní nabídka) vytvoří nový soubor a otevře ho
  v externím editoru. Nabídne se jméno pod kurzorem. Na tomto počítači prázdný soubor vznikne hned, na
  serveru až prvním uložením, takže zavření editoru bez uložení nic nevytvoří. Existující jméno otevře
  ten soubor k úpravě.
- **Shift+Enter** nebo kontextová nabídka (pravé tlačítko) nabízí **Otevřít s…**, práva
  (**Ctrl+Shift+A**) a kopírování názvu, cesty nebo adresy `sftp://` do schránky.
- Po uložení v editoru (**F4**, **Shift+F4**) se u souboru na tomto počítači v panelu aktualizuje
  velikost a datum.

## Archivy

- **Alt+F5** zabalí označené položky do zip, 7z, tar, tar.gz nebo tar.zst, volitelně s heslem
  (zip a 7z).
- **Alt+F9** rozbalí archiv do druhého panelu s ochranou proti nebezpečným cestám a archivním
  bombám. **Alt+Shift+F9** archiv otestuje.
- Balení nebo rozbalování ukončené pádem nebo zabitím programu nechá svůj nedokončený soubor
  `.filepart` jen do příštího spuštění programu, které ho smaže.
- **Enter** na archivu ho otevře jako složku jen pro čtení; **F5** z něj kopíruje soubory ven.
- Časy v zipu se ukládají na sekundu a v UTC (jako to dělá Info-ZIP), takže sedí i v jiném časovém
  pásmu. Zip z programu, který zapisuje jen starý čas DOS (bez časového pásma, jen sudé sekundy), se
  čte v časovém pásmu tohoto počítače.

## Hledání a kontrolní součty

- **Ctrl+F7** (nebo **Ctrl+F**) hledá podle názvu (maska, text nebo regulární výraz) a podle obsahu.
  Na serveru hledání běží přímo tam přes `find` a `grep`, pokud to jde.
- Kontrolní součty (MD5, SHA-1, SHA-256, SHA-512, BLAKE3, CRC32) jsou v menu Soubor a v kontextové
  nabídce; lze je uložit ve formátu pro `sha256sum -c`.

## Stav disku (SMART)

- Odznak v řádku s cestou ukazuje stav disku: **v pořádku**, **varování** (oranžově) nebo **kritický
  stav** (červeně). Kliknutím se zobrazí srozumitelné vysvětlení a doporučení.
- Podrobnosti ukazují model a sériové číslo, typ (HDD, SSD, NVMe), opotřebení a odhad zbývající
  životnosti, teplotu, dobu provozu, počet zapnutí, neočekávaná vypnutí, vadné a nestabilní sektory,
  chyby přenosu (CRC; obvykle kabel nebo konektor) a zapsaná data. Všechny atributy jsou v části
  Pokročilé.
- Data pocházejí z UDisks2 (bez práv správce), případně ze `smartctl`, pokud běží bez nich. Čtou se
  na pozadí, nejvýš jednou denně pro každý disk (uložené v `~/.cache/rolidscp/smart.json`);
  **Načíst znovu** disk přečte hned.
- Varování: opotřebení SSD od 60 %, jakékoli vadné, nestabilní nebo nečitelné sektory, pevný disk
  od 45 °C nebo SSD od 60 °C. Kritický stav: opotřebení od 95 %, víc než 50 vadných sektorů nebo
  jejich rostoucí počet, pevný disk od 55 °C nebo SSD od 75 °C, kritické varování NVMe nebo nízká
  rezerva, neúspěšný autotest. Teplota se kontroluje každých 5 minut.
- Síťové souborové systémy, virtuální disky a USB adaptéry bez SMART mají šedý odznak; jeho popisek i
  podrobnosti řeknou proč.
- Odznak je malá ikona srdečního tepu: šedá, když stav není známý, zelená, když je disk v pořádku,
  oranžová nebo červená s nejvážnějším důvodem vedle (např. „Opotřebení 87 %“, „Vadné sektory“ nebo
  „61 °C“). Po najetí myší ukáže všechny důvody. Pruh volného místa zoranžoví pod 10 % volného místa a
  zčervená pod 5 %.

## Nastavení

- **Nastavení → Nastavení…** (**Ctrl+,**) otevře nastavení ve vlastním okně. Vlevo je strom oddílů:
  Vzhled (jazyk a datum, písmo, panely, barvy podle typu souboru, miniatury), Soubory (kopírování a
  přesun, archivy, přetahování), Nástroje (prohlížeč, externí editor) a Servery a zabezpečení (připojení k
  serverům, uložená hesla).
- Pole nahoře (**Ctrl+F**) najde nastavení ve všech oddílech najednou, anglicky i v jazyce okna, s
  diakritikou i bez ní; **Esc** pole vymaže.
- **Uložit** uloží a okno zavře, **Použít** uloží a nechá ho otevřené, **Zrušit** nebo zavření okna
  změny zahodí. Uložení změní jen to, co jsi změnil v okně: co se mezitím změnilo v menu nebo v jiném
  okně programu, zůstane. V menu zůstávají rychlé položky (omezení rychlosti, volby při konfliktu,
  automatické obnovování, externí editor); jde o stejná nastavení.
- Jazyk se standardně řídí systémem (`LC_ALL`, `LC_MESSAGES`, `LANG`). V Nastavení ho lze vybrat
  a přepne se hned. Vlastní překlad `~/.config/rolidscp/lang/<jazyk>.po` se použije místo
  vestavěného.

## Soubory a log

- Konfigurace: `~/.config/rolidscp/` (čitelná jen pro vás).
- **Program spuštěný dvakrát:** obě okna spolupracují. Servery, nastavení a uložená hesla, která
  jedno okno uloží, se do několika sekund objeví i ve druhém, a když je mění obě, nic se neztratí
  (kde obě změnila totéž, platí pozdější změna). Panely si ukládá každé okno samo; které se otevřou
  příště, určí okno zavřené jako poslední.
- **Poškozený soubor nastavení** (upravený ručně, useknutý) nestojí všechno, co v něm je: použije
  se, co jde přečíst, zbytek vyjmenuje chybový řádek a soubor v původním stavu zůstane vedle jako
  `<jméno>.damaged`. Návrat ke starší verzi a zpět nastavení zachová (vyzkoušeno až po 0.8.13); od
  0.8.20 zůstávají v souborech i hodnoty, které starší verze nezná.
- Log: `~/.local/state/rolidscp/rolidscp.log` (menu Nápověda → Otevřít log). Nikdy neobsahuje hesla.
- AppImage lze aktualizovat programem AppImageUpdate (nebo `appimageupdatetool
  RolidSCP-<verze>-x86_64.AppImage`): najde nejnovější vydání na GitHubu a stáhne jen části, které
  se změnily. Nový AppImage se uloží vedle původního pod názvem vydání, původní zůstane.
