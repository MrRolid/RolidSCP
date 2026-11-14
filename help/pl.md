# Pomoc RolidSCP

RolidSCP to dwupanelowy menedżer plików dla dysków lokalnych i serwerów SFTP. Każdy panel ma karty;
karta to jedno miejsce (folder lokalny lub folder na serwerze) z własną nazwą i kolorem. Każda funkcja
jest w menu, większość także na pasku narzędzi i na pasku klawiszy funkcyjnych na dole. Listę
wszystkich skrótów klawiszowych znajdziesz na drugiej stronie tego okna.

## Panele i karty

- **Tab** przełącza między lewym i prawym panelem. Aktywny panel ma wyraźniejszy nagłówek.
- **Enter** lub dwuklik otwiera folder, archiwum (jako folder tylko do odczytu) albo plik w jego
  domyślnej aplikacji. Programy i skrypty nigdy nie są uruchamiane bez pytania; pliki na serwerze
  nigdy nie są uruchamiane. **Backspace** przechodzi poziom wyżej.
- **Ctrl+T** otwiera menu nowej karty: folder domowy, ulubione, zapisane serwery i ostatnie miejsca;
  pisanie filtruje listę. **Ctrl+W** zamyka kartę, **Ctrl+Tab** przełącza karty. Prawy przycisk na
  karcie oferuje Duplikuj, Zmień nazwę, Kolor, Zamknij i Zamknij pozostałe karty; dwuklik zmienia nazwę.
  Dymek nad kartą pokazuje serwer, adres i ścieżkę. **Ctrl+K** znajduje kartę lub zapisany serwer po
  nazwie.
- **Ctrl+D** dodaje folder do ulubionych albo go z nich usuwa. Ulubione i ostatnie foldery są w menu
  przycisku ★ paska (`*`, gdy pasek jest ukryty); **Plik → Ulubione foldery…** pozwala je nazwać i
  uporządkować.
- W menu **Ctrl+T** (i przycisku **+** przy kartach) ulubione są zawsze widoczne: **＋ Dodaj bieżący
  folder** dodaje folder aktywnego panelu (także na serwerze), ✏ zmienia nazwę ulubionego, a 🗑 go
  usuwa (potwierdź jeszcze jednym kliknięciem, **Esc** wraca).
- Zacznij pisać, aby filtrować listę; **Esc** czyści filtr.
- **Ctrl+F1**, **Ctrl+F2** i **Ctrl+Shift+F1** przełączają między widokiem skróconym, szczegółowym
  i miniaturami. **Alt+←** i **Alt+→** przechodzą wstecz i dalej w historii folderów.
- Wszystkie okna dialogowe działają tak samo: **Enter** naciska wyróżniony przycisk, **Esc** lub ✕
  anuluje, **Tab** przechodzi między polami. Okno można przesunąć, ciągnąc za pasek tytułu; ⓘ pokazuje
  więcej pomocy.
- Karty, kolory i foldery są zapisywane na bieżąco i przywracane przy następnym uruchomieniu.
- Nad każdym panelem jest pasek: lista rozwijana z folderami na ścieżce od korzenia (**Alt+↓**;
  strzałki, **Enter**, **Esc**), w panelu lokalnym także z dyskami tego komputera i ich rozmiarem;
  dalej Wstecz, Dalej, Poziom wyżej, Folder główny (**`Ctrl+\`**), Folder domowy (**Ctrl+H**), Odśwież i
  ulubione (★). W archiwum lista zaczyna się od folderu, w którym leży archiwum; w wynikach
  wyszukiwania jest niedostępna. **Widok → Pasek nad panelami** ukrywa pasek; przyciski `/ .. ~ *`
  wracają wtedy do wiersza ze ścieżką.
- Kliknięcie ścieżki pozwala wpisać inną. Po prawej wiersz ze ścieżką pokazuje wolne miejsce na dysku
  (cienki pasek i „Wolne: X z Y”), jego system plików (ext4, btrfs, nfs4, fuse.sshfs…) oraz, dla dysków
  tego komputera, plakietkę ze stanem dysku; w wąskim panelu znika najpierw „Wolne:”, potem rozmiar
  dysku, a potem system plików. W archiwum pokazuje dysk, na którym leży archiwum. Kliknięcie otwiera
  szczegóły.
- Prawy przycisk na pustym miejscu listy (lub na `..`) oferuje **Nowy plik…**, **Nowy folder**,
  **Odśwież**, **Dodaj folder do ulubionych**, **Otwórz terminal tutaj** i **Właściwości dysku…**
  (wolne miejsce, system plików i stan dysku).
- W szczegółowej liście kolumny oddzielają cienkie linie. Przeciągnij linię między nagłówkami kolumn,
  aby zmienić szerokość kolumny po jej prawej stronie; dwuklik dopasuje kolumnę do zawartości. Nazwa
  zajmuje resztę panelu, więc rośnie wraz z szerszym oknem; w wąskim panelu znikają Uprawnienia, a potem
  Zmieniono. Szerokości są zapamiętywane osobno dla lewego i prawego panelu (także po ponownym
  uruchomieniu); **Wyświetl → Przywróć domyślne szerokości kolumn** przywraca wartości domyślne. Nazwa
  lub rozszerzenie, które się nie mieści, kończy się „…”; po najechaniu myszą widać całość.
- Listy aktualizują się same. Folder na tym komputerze pokazuje zmiany wprowadzone przez inne programy w
  ciągu chwili. Folder na serwerze (lub na dysku sieciowym) odświeża się, gdy do niego wracasz – inna
  karta, drugi panel lub powrót do okna – jeśli lista jest starsza niż 5 sekund, a dopóki okno jest
  aktywne, także co 30 sekund (**Wyświetl → Automatyczne odświeżanie**: wyłączone, 10 s, 30 s, 1 min lub
  5 min; folder z ponad 20 000 elementów najwyżej co 5 minut). Kursor, zaznaczenia, położenie
  przewinięcia, sortowanie i obliczone rozmiary folderów pozostają. Podczas zmiany nazwy na liście,
  przeciągania lub otwartego okna dialogowego nic się nie odświeża, a utracone połączenie nie jest
  ponawiane. Po Twoich własnych operacjach odświeżają się wszystkie karty z tym folderem; **Ctrl+R**
  odświeża od razu.

## Zaznaczanie plików

- **Insert**, **Space** lub **Ctrl+klik** zaznacza element; **Ctrl+A** zaznacza wszystko lub nic.
- **Num+** zaznacza grupę wg maski, np. `*.jpg;*.png`, **Alt+−** odznacza grupę, **Alt+I**
  odwraca zaznaczenie.
- Wiersz pod listą pokazuje, ile elementów jest zaznaczonych i jaki mają rozmiar.
- **Alt+Space** (lub **Shift+Space**) oblicza rozmiar folderu pod kursorem lub zaznaczonych
  folderów, **Alt+Shift+Enter** wszystkich folderów w panelu.

## Kopiowanie, przenoszenie, zmiana nazwy

- **F5** kopiuje zaznaczone elementy (lub element pod kursorem) do folderu drugiego panelu,
  **F6** je przenosi. Oba działają między dyskami i serwerami, także między dwoma serwerami.
- Folder docelowy kopiowania i przenoszenia można edytować, wybrać z ostatnich celów albo wskazać
  przyciskiem **Przeglądaj…** (na dysku i na serwerze).
- Pliki można przeciągać myszą do drugiego panelu, na folder, na `..` albo na kartę (przytrzymanie nad
  kartą przełącza na nią). Przeciąganie kopiuje, z **Shift** przenosi, **Esc** anuluje przeciąganie.
  Okno Kopiuj lub Przenieś zawsze najpierw pyta.
- Gdy plik już istnieje w miejscu docelowym, wybierz: pomiń, nadpisz tylko starsze, nadpisz lub
  wznów przerwany transfer. Domyślny wybór można ustawić w menu Ustawienia.
- Pliki są zapisywane pod nazwą tymczasową i dopiero po ukończeniu otrzymują właściwą nazwę, więc
  przerwany transfer nigdy nie zostawia częściowo zapisanego pliku pod prawdziwą nazwą.
- Wznawiany jest tylko plik częściowy (`.filepart`) tego samego pliku źródłowego: obok niego
  zapisywane są jego ścieżka, rozmiar i czas modyfikacji. Plik częściowy z innego lub nieznanego
  źródła zostaje nietknięty i jest zgłaszany; aby zacząć od nowa, wybierz **Nadpisz**.
- Przenoszenie między dyskami lub serwerami usuwa plik źródłowy tylko wtedy, gdy nie zmienił się
  podczas transferu (rozmiar i czas modyfikacji), a kopia ma ten sam rozmiar. Z opcją **Sprawdź sumą
  kontrolną przed usunięciem źródła** (domyślnie włączona przy przenoszeniu na serwer) oba pliki są
  najpierw ponownie odczytywane i porównywane; różnica jest zgłaszana, a źródło zostaje.
- **F2** lub **Shift+F6** zmienia nazwę bezpośrednio na liście: nazwa bez rozszerzenia jest
  zaznaczona, **Enter** potwierdza, **Esc** anuluje.
- **F7** tworzy folder, **F8** usuwa (z potwierdzeniem). Usuwanie działa w kolejce transferów
  z liczbą usuniętych elementów; **Anuluj** je zatrzymuje (to, co już usunięto, pozostaje usunięte).
- Potoki (FIFO), gniazda, urządzenia i dowiązania symboliczne, które nigdzie nie prowadzą (brak celu
  lub pętla), nie są kopiowane: są pomijane i wymienione w raporcie na końcu, reszta kopii trwa dalej.
- Nietypowe nazwy są kopiowane dokładnie: znaki nowego wiersza, ukośnik wsteczny, nazwy o długości
  255 bajtów, a na dyskach tego komputera także nazwy, które nie są poprawnym UTF-8. Lista pokazuje
  je w jednym wierszu: znaki sterujące jako `\n` lub `\t`, każdy niepoprawny bajt jako `\xFF`.
  Nazwy, która nie jest poprawnym UTF-8, nie można wysłać na serwer (SFTP przyjmuje nazwy jako
  tekst); program to zgłasza.
- Kolejka transferów wykonuje do trzech transferów naraz. Limit prędkości jest w oknie kopiowania.
- **Przeciąganie do innych aplikacji** (X11): przeciągnij pliki lub foldery poza okno, np. do menedżera
  plików albo edytora. Pliki lokalne są kopiowane, z **Shift** przenoszone. Pliki z serwera lub archiwum
  są tylko kopiowane: aplikacja obsługująca bezpośredni zapis (XDS) dostaje je pobrane do swojego
  folderu przez kolejkę transferów; w przeciwnym razie najpierw są pobierane do prywatnego folderu
  tymczasowego (powyżej 1 GiB z pytaniem), który jest usuwany, gdy nic go już nie używa, oraz przy
  zamknięciu. Niektóre aplikacje czekają na pliki tylko około 30 sekund, więc duże pobieranie może tą
  drogą nie przejść. Po powrocie nad okno przeciąganie trwa dalej wewnątrz; **Esc** je anuluje. Niedostępne
  w Waylandzie.

## Serwery

- **Ctrl+O** otwiera menedżer serwerów. Wpisz serwer jako `host` lub `user@host:port`, zapisz go
  jako profil z nazwą i kolorem karty i połącz się klawiszem **Enter**.
- Oznacz serwer jako **serwer produkcyjny**: jego karty mają czerwony nagłówek, a niebezpieczne
  działania wymagają dodatkowego potwierdzenia (usuwania nie można potwierdzić klawiszem Enter).
- Na serwerze produkcyjnym również kopiowania, wysyłania, przenoszenia i tworzenia folderu nie da
  się potwierdzić klawiszem Enter, a kopiowanie nigdy nie zaczyna się od opcji Nadpisz (wstępnie
  wybrane jest Pomiń istniejące).
- Klucz serwera jest sprawdzany w `~/.ssh/known_hosts`. Nowy klucz jest dodawany dopiero po
  potwierdzeniu jego odcisku; zmieniony klucz jest odrzucany.
- Logowanie próbuje kolejno ssh-agent, Twoje klucze, a potem hasło. Zapisane hasła są przechowywane
  w postaci zaszyfrowanej, chronione hasłem głównym.
- Zapisane hasła blokują się ponownie po ustawionym czasie od ostatniego użycia (menu Ustawienia → hasło
  główne, domyślnie 60 sekund); pasek stanu pokazuje pozostały czas. Zablokowanie usuwa też hasła
  przechowywane do ponownego łączenia.
- Gdy połączenie zostanie zerwane, RolidSCP sam łączy się ponownie, a przerwane transfery są
  kontynuowane.
- **Ctrl+P** otwiera terminal systemowy (`$TERMINAL` lub `x-terminal-emulator`) w folderze panelu. Na
  serwerze uruchamia `ssh` do tego folderu z tym samym sprawdzaniem klucza serwera co RolidSCP
  (nieznany lub zmieniony klucz kończy połączenie). Zapisane hasło przekazuje jednorazowy pomocnik
  (`SSH_ASKPASS`), który odpowiada tylko na pytanie o hasło do tego serwera i tylko raz; gdy hasła
  są zablokowane, najpierw pyta o hasło główne, w przeciwnym razie ssh pyta samo. Przy serwerze
  produkcyjnym najpierw pyta o potwierdzenie. Hasło nigdy nie trafia do wiersza poleceń, pliku ani
  dziennika.
- **Ctrl+L** przełącza aktywny panel z powrotem na dysk lokalny.

## Podgląd plików (F3)

- **F3** otwiera plik w podglądzie, we własnym oknie (można otworzyć kilka naraz). Jest też w menu
  kontekstowym, w menu Plik i na pasku klawiszy. Podgląd nigdy niczego nie uruchamia i niczego nie
  wczytuje z zewnątrz: HTML, XML i SVG pokazuje jako tekst.
- Pliki dowolnej wielkości otwierają się od razu: czytana jest tylko część na ekranie. Plik na
  serwerze nie jest pobierany w całości, plik w archiwum jest najpierw rozpakowywany do pliku
  tymczasowego. Numery wierszy z **~** są przybliżone, dopóki wiersze nie zostaną policzone.
- **1**, **2** i **3** przełączają tekst, widok szesnastkowy i obraz; pliki binarne same pokazują się
  szesnastkowo.
- **W** zawija wiersze, **N** pokazuje numery wierszy, **Ctrl+G** przechodzi do wiersza, procentu lub
  pozycji, **Ctrl+Home** i **Ctrl+End** na początek i koniec. Menu Kodowanie i Podświetlanie zmieniają
  kodowanie i język podświetlania składni (wyłączone dla plików ponad 20 MB, zob.
  **Ustawienia → Ustawienia… → Podgląd (F3)**).
- **F** śledzi koniec rosnącego pliku, także na serwerze; przewinięcie w górę wstrzymuje śledzenie.
  Gdy plik się zmieni, pasek proponuje ponowne wczytanie. Utracone połączenie pokazuje pasek, a widok
  sam wraca do działania po jego przywróceniu.
- **Ctrl+F** lub **F7** szuka tekstu (wielkość liter, całe słowa, wyrażenie regularne); **F3** i
  **Shift+F3** przechodzą do następnego i poprzedniego wystąpienia. Cały plik jest przeszukiwany w
  tle; na serwerze szuka tam `grep`, jeśli się da, w przeciwnym razie plik jest czytany przez SFTP, a
  pasek wyszukiwania mówi, który sposób użyto.
- Zaznacza się myszą lub Shiftem i strzałkami; **Ctrl+C** kopiuje zaznaczenie (najwyżej 32 MB).
- Obrazy (JPEG, PNG, GIF także animowany, WebP, BMP): **F** dopasowuje obraz do okna, **0** pokazuje
  100 %, kółko przybliża, przeciąganie przesuwa, **R** i **L** obracają widok, strzałki i
  **PgUp**/**PgDn** pokazują poprzedni i następny obraz w folderze. Obraz z serwera jest pobierany
  tylko do 50 MB (można zmienić).
- **F4** edytuje plik jak w panelu; **Esc** lub **Q** zamyka podgląd.

## Edycja i otwieranie plików

- **F4** otwiera plik w edytorze zewnętrznym. Plik z serwera jest wysyłany z powrotem po każdym
  zapisie; jeśli w międzyczasie zmienił się na serwerze, zostaniesz zapytany, co zrobić.
- Lokalna kopia jest w prywatnym folderze na dysku (`~/.local/state/rolidscp/edit`, tylko dla
  ciebie) i jest usuwana po zakończeniu edycji. Wysyłanie, które nie powiedzie się z innego powodu
  niż utrata połączenia, jest powtarzane dwa razy, a potem zgłaszane. Czas ostatniego wysłania jest
  pokazywany z sekundami.
- Serwer bez atomowej zmiany nazwy dostaje zmieniony plik w krokach (oryginał jest najpierw
  odkładany na bok). Jeśli w międzyczasie przerwie się połączenie lub program, krok zostanie
  dokończony lub cofnięty przy następnym połączeniu z tym serwerem, a program poinformuje, których
  plików to dotyczyło.
- **Shift+F4** (także **Plik → Nowy plik…** i menu kontekstowe) tworzy nowy plik i otwiera go
  w zewnętrznym edytorze. Proponowana jest nazwa pod kursorem. Na tym komputerze pusty plik powstaje od
  razu, na serwerze dopiero przy pierwszym zapisie, więc zamknięcie edytora bez zapisu niczego nie
  tworzy. Istniejąca nazwa otwiera ten plik do edycji.
- **Shift+Enter** lub menu kontekstowe (prawy klik) oferuje **Otwórz za pomocą…**, uprawnienia
  (**Ctrl+Shift+A**) oraz kopiowanie nazwy, ścieżki lub adresu `sftp://` do schowka.
- Po zapisaniu w edytorze (**F4**, **Shift+F4**) rozmiar i data pliku na tym komputerze są aktualizowane
  w panelu.

## Archiwa

- **Alt+F5** pakuje zaznaczone elementy do zip, 7z, tar, tar.gz lub tar.zst, opcjonalnie z hasłem
  (zip i 7z).
- **Alt+F9** rozpakowuje archiwum do drugiego panelu, z ochroną przed niebezpiecznymi ścieżkami
  i bombami archiwalnymi. **Alt+Shift+F9** testuje archiwum.
- **Enter** na archiwum otwiera je jako folder tylko do odczytu; **F5** kopiuje z niego pliki.

## Wyszukiwanie i sumy kontrolne

- **Ctrl+F7** (lub **Ctrl+F**) wyszukuje po nazwie (maska, tekst lub wyrażenie regularne) i po
  zawartości. Na serwerze wyszukiwanie w miarę możliwości odbywa się na nim za pomocą `find`
  i `grep`.
- Sumy kontrolne (MD5, SHA-1, SHA-256, SHA-512, BLAKE3, CRC32) są w menu Plik i w menu
  kontekstowym; można je zapisać w formacie `sha256sum -c`.

## Stan dysku (SMART)

- Plakietka w wierszu ze ścieżką pokazuje stan dysku: **OK**, **ostrzeżenie** (pomarańczowo) lub **stan
  krytyczny** (czerwono). Kliknięcie pokazuje zrozumiałe wyjaśnienie i zalecenie.
- Szczegóły pokazują model i numer seryjny, typ (HDD, SSD, NVMe), zużycie i szacowany pozostały czas
  życia, temperaturę, czas pracy, liczbę włączeń, nieoczekiwane wyłączenia, uszkodzone i niestabilne
  sektory, błędy transmisji (CRC; zwykle kabel lub złącze) i zapisane dane. Wszystkie atrybuty są w
  części Zaawansowane.
- Dane pochodzą z UDisks2 (bez uprawnień administratora) lub ze `smartctl`, jeśli działa bez nich.
  Są odczytywane w tle, najwyżej raz dziennie dla każdego dysku (zapisane w
  `~/.cache/rolidscp/smart.json`); **Wczytaj ponownie** odczytuje dysk od razu.
- Ostrzeżenie: zużycie SSD od 60 %, jakiekolwiek uszkodzone, niestabilne lub nieczytelne sektory,
  dysk twardy od 45 °C lub SSD od 60 °C. Stan krytyczny: zużycie od 95 %, ponad 50 uszkodzonych
  sektorów lub ich rosnąca liczba, dysk twardy od 55 °C lub SSD od 75 °C, krytyczne ostrzeżenie NVMe
  lub mała rezerwa, nieudany autotest. Temperatura jest sprawdzana co 5 minut.
- Sieciowe systemy plików, dyski wirtualne i adaptery USB bez SMART mają szary znaczek; jego podpowiedź
  i szczegóły mówią dlaczego.
- Znaczek to mała ikona pulsu: szara, gdy stan nie jest znany, zielona, gdy dysk jest w porządku,
  pomarańczowa lub czerwona z najpoważniejszym powodem obok (np. „Zużycie 87 %”, „Uszkodzone sektory”
  lub „61 °C”). Po najechaniu myszą pokazuje wszystkie powody. Pasek wolnego miejsca robi się
  pomarańczowy poniżej 10 % wolnego miejsca i czerwony poniżej 5 %.

## Ustawienia

- **Ustawienia → Ustawienia…** zawiera język, format daty, czcionkę (wbudowaną lub systemową),
  rozmiar tekstu, hinting i kontrast, pasek narzędzi, kolory typów plików, miniatury i limity
  archiwów.
- Język domyślnie jest zgodny z systemem (`LC_ALL`, `LC_MESSAGES`, `LANG`). Można go wybrać
  w Ustawieniach i zmienia się od razu. Własne tłumaczenie `~/.config/rolidscp/lang/<language>.po`
  jest używane zamiast wbudowanego.

## Pliki i log

- Konfiguracja: `~/.config/rolidscp/` (czytelna tylko dla Ciebie).
- Log: `~/.local/state/rolidscp/rolidscp.log` (menu Pomoc → Otwórz log). Nigdy nie zawiera haseł.
