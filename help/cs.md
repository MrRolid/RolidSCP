# Nápověda RolidSCP

RolidSCP je dvoupanelový správce souborů pro lokální disky a SFTP servery. Každý panel má záložky;
záložka je jedno místo (lokální složka nebo složka na serveru) s vlastním názvem a barvou. Všechny
funkce jsou v menu, většina i na nástrojové liště a ve spodní liště funkčních kláves. Seznam všech
klávesových zkratek je na druhé stránce tohoto okna.

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
- Kliknutím na cestu lze napsat jinou. Vpravo řádek s cestou ukazuje volné místo na disku (tenký pruh a
  „Volné: X z Y“), jeho souborový systém (ext4, btrfs, nfs4, fuse.sshfs…) a u disků tohoto počítače
  odznak se stavem disku; v úzkém panelu se vynechá nejdřív „Volné:“, pak velikost disku a pak
  souborový systém. V archivu ukazuje disk, na kterém archiv leží. Kliknutím se otevřou podrobnosti.
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
- **Num+** označí skupinu podle masky, např. `*.jpg;*.png`, **Alt+−** označení skupiny zruší,
  **Alt+I** obrátí výběr.
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
- Přesun mezi disky nebo servery smaže zdrojový soubor, jen když se během přenosu nezměnil (velikost
  a čas změny) a kopie má stejnou velikost. S volbou **Před smazáním zdroje ověřit kontrolním
  součtem** (při přesunu na server výchozí zapnuto) se navíc oba znovu přečtou a porovnají; při
  rozdílu to program ohlásí a zdroj zůstane.
- **F2** nebo **Shift+F6** přejmenuje přímo v seznamu: název bez přípony je vybraný, **Enter**
  potvrdí, **Esc** zruší.
- **F7** vytvoří složku, **F8** maže (s potvrzením).
- Fronta přenosů běží nejvýš se třemi přenosy současně. Omezení rychlosti je v dialogu kopírování.
- **Tažení ven do jiných aplikací** (X11): soubory nebo složky vytáhněte z okna, třeba do správce
  souborů nebo editoru. Lokální soubory se kopírují, se **Shiftem** přesunou. Soubory ze serveru nebo
  z archivu se jen kopírují: aplikace, která umí přímé uložení (XDS), je dostane stažené do své složky
  přes frontu přenosů; jinak se nejdřív stáhnou do soukromé dočasné složky (nad 1 GiB se aplikace
  zeptá), která se smaže, až ji nic nepoužívá, a při ukončení. Některé aplikace na soubory čekají jen
  asi 30 sekund, velké stahování tudy proto nemusí projít. Po návratu nad okno tažení pokračuje uvnitř;
  **Esc** ho zruší. Na Waylandu není k dispozici.

## Servery

- **Ctrl+O** otevře správce serverů. Server zadáte jako `host` nebo `user@host:port`, uložíte ho
  jako profil s názvem a barvou záložky a připojíte se **Enterem**.
- Server můžete označit jako **produkční**: jeho záložky mají červenou hlavičku a nebezpečné akce se
  ptají ještě jednou (mazání nejde potvrdit Enterem).
- Na produkčním serveru nejde Enterem potvrdit ani kopírování, nahrávání, přesun a vytvoření složky
  a kopírování nikdy nezačíná volbou Přepsat (předvybrané je Přeskočit existující).
- Klíč serveru se kontroluje proti `~/.ssh/known_hosts`. Nový klíč se přidá jen po potvrzení
  otisku; změněný klíč se odmítne.
- Přihlášení zkouší ssh-agent, vaše klíče a pak heslo. Uložená hesla jsou šifrovaná a chráněná
  hlavním heslem.
- Uložená hesla se znovu zamknou po nastavené době od posledního použití (nabídka Nastavení → hlavní
  heslo, výchozí 60 sekund); stavový řádek ukazuje zbývající čas. Zamčení smaže i hesla držená pro
  znovupřipojení.
- Když spojení spadne, RolidSCP se sám připojí znovu a přerušené přenosy pokračují.
- **Ctrl+P** otevře systémový terminál (`$TERMINAL` nebo `x-terminal-emulator`) ve složce panelu.
  Na serveru spustí `ssh` do této složky se stejnou kontrolou klíče serveru jako RolidSCP (neznámý
  nebo změněný klíč spojení ukončí). Uložené heslo předá jednorázový pomocník (`SSH_ASKPASS`), který
  odpoví jen na dotaz na heslo k tomuto serveru, a to jednou; jsou-li hesla zamčená, nejdřív se
  zeptá na hlavní heslo, jinak se ssh zeptá samo. U produkčního serveru se před otevřením zeptá.
  Heslo nikdy nejde do příkazové řádky, souboru ani logu.
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
- **Enter** na archivu ho otevře jako složku jen pro čtení; **F5** z něj kopíruje soubory ven.

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

- **Nastavení → Nastavení…** obsahuje jazyk, formát data, písmo (vestavěné nebo systémové), velikost
  textu, hinting a kontrast, nástrojovou lištu, barvy typů souborů, miniatury a limity archivů.
- Jazyk se standardně řídí systémem (`LC_ALL`, `LC_MESSAGES`, `LANG`). V Nastavení ho lze vybrat
  a přepne se hned. Vlastní překlad `~/.config/rolidscp/lang/<jazyk>.po` se použije místo
  vestavěného.

## Soubory a log

- Konfigurace: `~/.config/rolidscp/` (čitelná jen pro vás).
- Log: `~/.local/state/rolidscp/rolidscp.log` (menu Nápověda → Otevřít log). Nikdy neobsahuje hesla.
