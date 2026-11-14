# RolidSCP-Hilfe

RolidSCP ist ein Zwei-Panel-Dateimanager für lokale Datenträger und SFTP-Server. Jedes Panel hat Tabs;
ein Tab ist ein Ort (ein lokaler Ordner oder ein Ordner auf einem Server) mit eigenem Namen und eigener
Farbe. Jede Funktion ist in den Menüs zu finden, die meisten auch in der Werkzeugleiste und in der
Funktionstastenleiste unten. Die Liste aller Tastenkürzel steht auf der zweiten Seite dieses Fensters.

## Panels und Tabs

- **Tab** wechselt zwischen linkem und rechtem Panel. Das aktive Panel hat einen kräftigeren Kopf.
- **Enter** oder ein Doppelklick öffnet einen Ordner, ein Archiv (als schreibgeschützten Ordner) oder
  eine Datei in ihrer Standardanwendung. Programme und Skripte werden nie ohne Nachfrage gestartet;
  Dateien auf einem Server werden nie ausgeführt. **Backspace** geht eine Ebene höher.
- **Ctrl+T** öffnet ein Menü für einen neuen Tab: Home-Ordner, Favoriten, gespeicherte Server und
  zuletzt besuchte Orte; Tippen filtert es. **Ctrl+W** schließt den Tab, **Ctrl+Tab** wechselt die Tabs.
  Ein Rechtsklick auf einen Tab bietet Duplizieren, Umbenennen, Farbe, Schließen und Andere Tabs
  schließen; ein Doppelklick benennt ihn um. Der Tooltip eines Tabs zeigt Server, Adresse und Pfad.
  **Ctrl+K** sucht einen Tab oder gespeicherten Server nach Namen.
- **Ctrl+D** fügt den Ordner zu den Favoriten hinzu oder entfernt ihn. Favoriten und zuletzt besuchte
  Ordner stehen im Menü der Schaltfläche ★ der Leiste (`*`, wenn die Leiste ausgeblendet ist); **Datei
  → Favoritenordner…** benennt und ordnet sie.
- Im Menü **Ctrl+T** (und hinter der Schaltfläche **+** bei den Tabs) sind die Favoriten immer zu sehen:
  **＋ Aktuellen Ordner hinzufügen** fügt den Ordner des aktiven Panels hinzu (auch auf einem Server), ✏
  benennt einen Favoriten um und 🗑 entfernt ihn (mit einem weiteren Klick bestätigen, **Esc** geht zurück).
- Einfach lostippen, um die Liste zu filtern; **Esc** löscht den Filter.
- **Ctrl+F1**, **Ctrl+F2** und **Ctrl+Shift+F1** wechseln zwischen Kurzansicht, Detailansicht und
  Miniaturen. **Alt+←** und **Alt+→** gehen im Ordnerverlauf zurück und vorwärts.
- Alle Dialoge funktionieren gleich: **Enter** drückt die hervorgehobene Schaltfläche, **Esc** oder ✕
  bricht ab, **Tab** wechselt zwischen den Feldern. Ein Dialog lässt sich an der Titelleiste
  verschieben; ⓘ zeigt weitere Hilfe.
- Tabs, Farben und Ordner werden laufend gespeichert und beim nächsten Start wiederhergestellt.
- Über jedem Panel liegt eine Leiste: eine Aufklappliste mit den Ordnern im Pfad ab der Wurzel
  (**Alt+↓**; Pfeiltasten, **Enter**, **Esc**), bei einem lokalen Panel auch mit den Laufwerken dieses
  Computers und ihrer Größe; dann Zurück, Vorwärts, Eine Ebene höher, Stammordner (**`Ctrl+\`**),
  Home-Ordner (**Ctrl+H**), Aktualisieren und die Favoriten (★). In einem Archiv beginnt die Liste mit
  dem Ordner, in dem das Archiv liegt; in Suchergebnissen ist sie nicht verfügbar. **Ansicht → Leiste
  über den Feldern** blendet die Leiste aus; die Schaltflächen `/ .. ~ *` kehren dann in die Pfadzeile
  zurück.
- Ein Klick auf den Pfad erlaubt, einen anderen einzugeben. Rechts zeigt die Pfadzeile den freien Platz
  der Platte (ein schmaler Balken und „Frei: X von Y“), ihr Dateisystem (ext4, btrfs, nfs4,
  fuse.sshfs…) und bei Platten dieses Computers ein Abzeichen mit ihrem Zustand; in einem schmalen
  Panel entfallen zuerst „Frei:“, dann die Größe der Platte und dann das Dateisystem. In einem Archiv
  zeigt sie die Platte, auf der das Archiv liegt. Ein Klick öffnet die Details.
- Ein Rechtsklick auf die leere Fläche der Liste (oder auf `..`) bietet **Neue Datei…**, **Neuer
  Ordner**, **Aktualisieren**, **Ordner zu Favoriten hinzufügen**, **Terminal hier öffnen** und
  **Laufwerkseigenschaften…** (freier Platz, Dateisystem und Zustand der Platte).
- In der ausführlichen Liste trennen dünne Linien die Spalten. Ziehen Sie eine Linie zwischen den
  Spaltentiteln, um die Breite der Spalte rechts davon zu ändern; ein Doppelklick passt die Spalte an
  ihren Inhalt an. Name nimmt den Rest des Panels ein und wächst mit einem breiteren Fenster; in einem
  schmalen Panel werden Rechte und dann Geändert ausgeblendet. Die Breiten werden für das linke und das
  rechte Panel getrennt gespeichert (auch nach einem Neustart); **Anzeigen → Spaltenbreiten
  zurücksetzen** stellt die Vorgaben wieder her. Ein Name oder eine Endung, die nicht passt, endet mit
  „…“; mit der Maus darüber sehen Sie sie vollständig.
- Die Listen halten sich selbst aktuell. Ein Ordner auf diesem Computer zeigt Änderungen anderer
  Programme nach einem Augenblick. Ein Ordner auf einem Server (oder auf einem Netzlaufwerk) wird
  aktualisiert, wenn Sie zu ihm zurückkehren – ein anderer Tab, das andere Panel oder zurück ins Fenster
  –, sofern die Liste älter als 5 Sekunden ist, und solange das Fenster aktiv ist, auch alle 30 Sekunden
  (**Anzeigen → Automatisch aktualisieren**: aus, 10 s, 30 s, 1 min oder 5 min; ein Ordner mit mehr als
  20 000 Einträgen höchstens alle 5 Minuten). Cursor, Markierungen, Bildlaufposition, Sortierung und
  berechnete Ordnergrößen bleiben erhalten. Während Sie in der Liste umbenennen, ziehen oder ein Dialog
  offen ist, wird nichts aktualisiert, und eine verlorene Verbindung wird nicht versucht. Nach Ihren
  eigenen Vorgängen werden alle Tabs mit diesem Ordner aktualisiert; **Ctrl+R** aktualisiert sofort.

## Dateien auswählen

- **Insert**, **Space** oder **Ctrl+Klick** wählt ein Element aus; **Ctrl+A** wählt alle oder keine aus.
- **Num+** wählt eine Gruppe per Maske wie `*.jpg;*.png` aus, **Alt+−** wählt eine ab, **Alt+I**
  kehrt die Auswahl um.
- Die Zeile unter der Liste zeigt, wie viele Elemente ausgewählt sind und wie groß sie sind.
- **Alt+Space** (oder **Shift+Space**) berechnet die Größe des Ordners unter dem Cursor oder der
  ausgewählten Ordner, **Alt+Shift+Enter** die aller Ordner im Panel.

## Kopieren, Verschieben, Umbenennen

- **F5** kopiert die ausgewählten Elemente (oder das Element unter dem Cursor) in den Ordner des
  anderen Panels, **F6** verschiebt sie. Beides funktioniert zwischen Datenträgern und Servern, auch
  zwischen zwei Servern.
- Der Zielordner beim Kopieren und Verschieben kann bearbeitet, aus den letzten Zielen gewählt oder mit
  **Durchsuchen…** ausgesucht werden (auf der Festplatte und auf dem Server).
- Dateien lassen sich mit der Maus in das andere Panel, auf einen Ordner, auf `..` oder auf einen Tab
  ziehen (über einem Tab halten wechselt zu ihm). Ziehen kopiert, mit **Shift** wird verschoben, **Esc**
  bricht das Ziehen ab. Der Dialog Kopieren oder Verschieben fragt immer zuerst.
- Existiert eine Datei im Ziel bereits, stehen zur Wahl: überspringen, nur ältere Dateien
  überschreiben, überschreiben oder eine unterbrochene Übertragung fortsetzen. Die Standardwahl lässt
  sich im Menü Einstellungen festlegen.
- Dateien werden unter einem temporären Namen geschrieben und erst nach Abschluss umbenannt, sodass
  eine abgebrochene Übertragung nie eine halb geschriebene Datei unter dem echten Namen hinterlässt.
- Fortgesetzt wird nur eine Teildatei (`.filepart`) derselben Quelldatei: Ihr Pfad, ihre Größe und
  Änderungszeit werden daneben gespeichert. Eine Teildatei einer anderen oder unbekannten Quelle
  bleibt unangetastet und wird gemeldet; wählen Sie **Überschreiben**, um neu zu beginnen.
- Verschieben zwischen Datenträgern oder Servern löscht eine Quelldatei nur, wenn sie sich während
  der Übertragung nicht geändert hat (Größe und Änderungszeit) und die Kopie dieselbe Größe hat. Mit
  **Vor dem Löschen der Quelle per Prüfsumme prüfen** (beim Verschieben auf einen Server
  standardmäßig an) werden beide zuvor erneut gelesen und verglichen; ein Unterschied wird gemeldet
  und die Quelle bleibt.
- **F2** oder **Shift+F6** benennt direkt in der Liste um: Der Name ohne Erweiterung ist markiert,
  **Enter** bestätigt, **Esc** bricht ab.
- **F7** erstellt einen Ordner, **F8** löscht (mit Bestätigung). Das Löschen läuft in der
  Übertragungswarteschlange mit der Zahl der gelöschten Einträge; **Abbrechen** hält es an (was bis
  dahin gelöscht ist, bleibt gelöscht).
- Pipes (FIFO), Sockets, Geräte und symbolische Links, die ins Leere führen (Ziel fehlt oder
  Schleife), werden nicht kopiert: Sie werden übersprungen und im Bericht am Ende aufgeführt, der
  Rest der Kopie läuft weiter.
- Ungewöhnliche Namen werden exakt kopiert: Zeilenumbrüche, ein Backslash, Namen mit 255 Bytes und
  auf den Laufwerken dieses Computers auch Namen, die kein gültiges UTF-8 sind. Die Liste zeigt sie
  in einer Zeile: Steuerzeichen als `\n` oder `\t`, jedes ungültige Byte als `\xFF`. Ein Name, der
  kein gültiges UTF-8 ist, kann nicht auf einen Server übertragen werden (SFTP nimmt Namen als
  Text); das wird gemeldet.
- Die Übertragungswarteschlange führt bis zu drei Übertragungen gleichzeitig aus. Das
  Geschwindigkeitslimit befindet sich im Kopierdialog.
- **Ziehen in andere Anwendungen** (X11): Ziehen Sie Dateien oder Ordner aus dem Fenster, etwa auf einen
  Dateimanager oder einen Editor. Lokale Dateien werden kopiert, mit **Shift** verschoben. Dateien von
  einem Server oder aus einem Archiv werden nur kopiert: Eine Anwendung, die direktes Speichern (XDS)
  unterstützt, bekommt sie über die Übertragungswarteschlange in ihren Ordner heruntergeladen; sonst
  werden sie zuerst in einen privaten temporären Ordner geladen (ab 1 GiB mit Rückfrage), der gelöscht
  wird, sobald ihn nichts mehr nutzt, und beim Beenden. Manche Anwendungen warten nur etwa 30 Sekunden
  auf die Dateien, große Downloads kommen auf diesem Weg daher eventuell nicht an. Zurück über dem
  Fenster geht das Ziehen darin weiter; **Esc** bricht es ab. Unter Wayland nicht verfügbar.

## Server

- **Ctrl+O** öffnet die Serververwaltung. Den Server als `host` oder `user@host:port` eingeben, als
  Profil mit Namen und Tabfarbe speichern und mit **Enter** verbinden.
- Einen Server als **Produktionsserver** kennzeichnen: Seine Tabs erhalten einen roten Kopf, und
  gefährliche Aktionen fragen noch einmal nach (Löschen lässt sich nicht mit Enter bestätigen).
- Auf einem Produktionsserver lassen sich auch Kopieren, Hochladen, Verschieben und das Anlegen
  eines Ordners nicht mit Enter bestätigen, und Kopieren beginnt nie mit Überschreiben (vorgewählt
  ist Vorhandene überspringen).
- Der Serverschlüssel wird mit `~/.ssh/known_hosts` abgeglichen. Ein neuer Schlüssel wird erst
  hinzugefügt, nachdem sein Fingerabdruck bestätigt wurde; ein geänderter Schlüssel wird abgelehnt.
- Die Anmeldung versucht ssh-agent, dann die eigenen Schlüssel, dann das Passwort. Gespeicherte
  Passwörter werden verschlüsselt aufbewahrt, geschützt durch ein Master-Passwort.
- Gespeicherte Passwörter werden eine einstellbare Zeit nach ihrer letzten Verwendung wieder gesperrt
  (Menü Einstellungen → Master-Passwort, standardmäßig 60 Sekunden); die Statusleiste zeigt die
  verbleibende Zeit. Beim Sperren werden auch die für die Wiederverbindung gehaltenen Passwörter
  vergessen.
- Bricht die Verbindung ab, verbindet sich RolidSCP von selbst neu, und unterbrochene Übertragungen
  laufen weiter.
- **Ctrl+P** öffnet das Systemterminal (`$TERMINAL` oder `x-terminal-emulator`) im Ordner des Felds.
  Auf einem Server startet es `ssh` in diesen Ordner, mit derselben Prüfung des Serverschlüssels wie
  RolidSCP (ein unbekannter oder geänderter Schlüssel beendet die Verbindung). Ein gespeichertes
  Passwort übergibt ein einmaliger Helfer (`SSH_ASKPASS`), der nur die Passwortfrage dieses Servers
  beantwortet, und zwar einmal; sind die Passwörter gesperrt, wird zuerst das Hauptpasswort
  abgefragt, sonst fragt ssh selbst. Bei Produktionsservern wird vorher nachgefragt. Das Passwort
  gelangt nie in eine Befehlszeile, eine Datei oder das Protokoll.
- **Ctrl+L** schaltet das aktive Panel zurück auf den lokalen Datenträger.

## Dateien anzeigen (F3)

- **F3** öffnet die Datei im Betrachter, einem eigenen Fenster (mehrere können gleichzeitig offen
  sein). Er steht auch im Kontextmenü, im Menü Datei und in der Tastenleiste. Der Betrachter führt
  nie etwas aus und lädt nichts von außen: HTML, XML und SVG werden als Text gezeigt.
- Dateien jeder Größe öffnen sich sofort: Gelesen wird nur der Teil auf dem Bildschirm. Eine Datei
  auf einem Server wird nicht ganz heruntergeladen, eine Datei in einem Archiv wird zuerst in eine
  temporäre Datei entpackt. Zeilennummern mit **~** sind geschätzt, bis die Zeilen gezählt sind.
- **1**, **2** und **3** wechseln zwischen Text, Hexadezimal und Bild; Binärdateien werden von selbst
  hexadezimal gezeigt.
- **W** bricht Zeilen um, **N** zeigt Zeilennummern, **Ctrl+G** springt zu einer Zeile, einem
  Prozentsatz oder einer Position, **Ctrl+Home** und **Ctrl+End** an den Anfang und das Ende. Die
  Menüs Kodierung und Hervorhebung ändern die Kodierung und die Sprache der Syntaxhervorhebung (bei
  Dateien über 20 MB aus, siehe **Einstellungen → Einstellungen… → Betrachter (F3)**).
- **F** folgt dem Ende einer wachsenden Datei, auch auf einem Server; Zurückscrollen hält das an.
  Ändert sich die Datei, bietet eine Leiste das Neuladen an. Eine verlorene Verbindung zeigt eine
  Leiste, und die Ansicht läuft von selbst weiter, sobald sie wieder da ist.
- **Ctrl+F** oder **F7** sucht Text (Groß-/Kleinschreibung, ganze Wörter, regulärer Ausdruck);
  **F3** und **Shift+F3** gehen zum nächsten und vorigen Treffer. Die ganze Datei wird im
  Hintergrund durchsucht; auf einem Server sucht dort `grep`, wenn möglich, sonst wird die Datei über
  SFTP gelesen, und die Suchleiste sagt, was davon.
- Auswählen mit der Maus oder mit Shift und den Pfeilen; **Ctrl+C** kopiert die Auswahl
  (höchstens 32 MB).
- Bilder (JPEG, PNG, GIF auch animiert, WebP, BMP): **F** passt das Bild an das Fenster an, **0**
  zeigt es in 100 %, das Mausrad zoomt, Ziehen verschiebt, **R** und **L** drehen die Ansicht, die
  Pfeile und **PgUp**/**PgDn** zeigen das vorige und das nächste Bild des Ordners. Ein Bild auf
  einem Server wird nur bis 50 MB heruntergeladen (einstellbar).
- **F4** bearbeitet die Datei wie im Panel; **Esc** oder **Q** schließt den Betrachter.

## Dateien bearbeiten und öffnen

- **F4** öffnet die Datei in einem externen Editor. Eine Datei von einem Server wird nach jedem
  Speichern zurück hochgeladen; wurde sie inzwischen auf dem Server geändert, wird nachgefragt.
- Die lokale Kopie liegt in einem privaten Ordner auf der Festplatte
  (`~/.local/state/rolidscp/edit`, nur für Sie) und wird nach dem Ende der Bearbeitung gelöscht. Ein
  Hochladen, das aus einem anderen Grund als einem Verbindungsverlust fehlschlägt, wird zweimal
  wiederholt und dann gemeldet. Die Zeit des letzten Hochladens wird mit Sekunden angezeigt.
- Ein Server ohne atomares Umbenennen erhält eine geänderte Datei in Schritten (das Original wird
  zuerst beiseitegelegt). Bricht dazwischen die Verbindung oder das Programm ab, wird der Schritt
  bei der nächsten Verbindung zu diesem Server abgeschlossen oder rückgängig gemacht, und Sie
  erfahren, welche Dateien es betraf.
- **Shift+F4** (auch **Datei → Neue Datei…** und das Kontextmenü) legt eine neue Datei an und öffnet
  sie im externen Editor. Vorgeschlagen wird der Name unter dem Cursor. Auf diesem Computer entsteht die
  leere Datei sofort, auf einem Server erst beim ersten Speichern – Schließen ohne Speichern legt nichts
  an. Ein vorhandener Name öffnet diese Datei zum Bearbeiten.
- **Shift+Enter** oder das Kontextmenü (Rechtsklick) bietet **Öffnen mit…**, Rechte
  (**Ctrl+Shift+A**) und das Kopieren von Name, Pfad oder `sftp://`-Adresse in die Zwischenablage.
- Nach dem Speichern im Editor (**F4**, **Shift+F4**) werden Größe und Datum einer Datei auf diesem
  Computer im Panel aktualisiert.

## Archive

- **Alt+F5** packt die ausgewählten Elemente als zip, 7z, tar, tar.gz oder tar.zst, wahlweise mit
  Passwort (zip und 7z).
- **Alt+F9** entpackt ein Archiv ins andere Panel, mit Schutz vor gefährlichen Pfaden und
  Archivbomben. **Alt+Shift+F9** testet ein Archiv.
- **Enter** auf einem Archiv öffnet es als schreibgeschützten Ordner; **F5** kopiert Dateien heraus.

## Suche und Prüfsummen

- **Ctrl+F7** (oder **Ctrl+F**) sucht nach Name (Maske, Text oder regulärer Ausdruck) und nach
  Inhalt. Auf einem Server läuft die Suche nach Möglichkeit dort mit `find` und `grep`.
- Prüfsummen (MD5, SHA-1, SHA-256, SHA-512, BLAKE3, CRC32) finden sich im Menü Datei und im
  Kontextmenü; sie lassen sich im Format von `sha256sum -c` speichern.

## Zustand der Platte (SMART)

- Das Abzeichen in der Pfadzeile zeigt den Zustand der Platte: **OK**, **Warnung** (orange) oder
  **kritisch** (rot). Ein Klick zeigt eine verständliche Erklärung und eine Empfehlung.
- Die Details zeigen Modell und Seriennummer, Typ (HDD, SSD, NVMe), Abnutzung und geschätzte
  Restlebensdauer, Temperatur, Betriebszeit, Einschaltvorgänge, unerwartete Abschaltungen, defekte und
  instabile Sektoren, Übertragungsfehler (CRC; meist ein Kabel oder Stecker) und die geschriebenen
  Daten. Alle Attribute stehen im Abschnitt Erweitert.
- Die Daten kommen von UDisks2 (ohne Administratorrechte) oder von `smartctl`, wenn es ohne sie
  läuft. Sie werden im Hintergrund gelesen, höchstens einmal am Tag pro Platte (gespeichert in
  `~/.cache/rolidscp/smart.json`); **Neu laden** liest die Platte sofort.
- Warnung: SSD-Abnutzung ab 60 %, beliebige defekte, instabile oder unlesbare Sektoren, eine
  Festplatte ab 45 °C oder eine SSD ab 60 °C. Kritisch: Abnutzung ab 95 %, mehr als 50 defekte
  Sektoren oder eine wachsende Zahl, eine Festplatte ab 55 °C oder eine SSD ab 75 °C, eine kritische
  NVMe-Warnung oder wenig Reserve, ein fehlgeschlagener Selbsttest. Die Temperatur wird alle
  5 Minuten geprüft.
- Netzwerk-Dateisysteme, virtuelle Platten und USB-Adapter ohne SMART haben ein graues Abzeichen; sein
  Hinweis und die Details sagen, warum.
- Das Abzeichen ist ein kleines Herzschlag-Symbol: grau, wenn der Zustand unbekannt ist, grün, wenn die
  Platte in Ordnung ist, orange oder rot mit dem ernstesten Grund daneben (z. B. „Abnutzung 87 %“,
  „Defekte Sektoren“ oder „61 °C“). Mit der Maus darüber sehen Sie alle Gründe. Der Balken des freien
  Speichers wird unter 10 % freiem Platz orange und unter 5 % rot.

## Einstellungen

- **Einstellungen → Einstellungen…** enthält Sprache, Datumsformat, Schrift (integriert oder vom
  System), Textgröße, Hinting und Kontrast, die Werkzeugleiste, Farben nach Dateityp, Miniaturen und
  Archivgrenzen.
- Die Sprache richtet sich standardmäßig nach dem System (`LC_ALL`, `LC_MESSAGES`, `LANG`). Sie lässt
  sich in den Einstellungen wählen und wechselt sofort. Eine eigene Übersetzung
  `~/.config/rolidscp/lang/<Sprache>.po` wird anstelle der integrierten verwendet.

## Dateien und Protokoll

- Konfiguration: `~/.config/rolidscp/` (nur für den eigenen Benutzer lesbar).
- Protokoll: `~/.local/state/rolidscp/rolidscp.log` (Menü Hilfe → Protokoll öffnen
  (Anmeldediagnose)). Es enthält nie Passwörter.
