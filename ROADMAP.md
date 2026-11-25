# Roadmap

RolidSCP moves your files, often on servers that matter. So reliability comes first and new features
second: the 0.8 series (closed with 0.8.20) got no new features, only fixes and stress tests of
what was already there. Small new features come in 0.9, large ones from 1.0 alpha.

This is a plan, not a promise. There are no dates, and the order within a section can change.
Releases are listed on the [Releases](https://github.com/MrRolid/RolidSCP/releases) page.

## 0.8.x: hardening (done)

No new features. Each release tests one area under load, fixes what breaks and adds tests that would
catch it again.

- **0.8.15 Many files:** a folder with a million files, deep folder trees, memory use, sorting,
  search, folder sizes, copying and deleting millions of small files. Unusual file names (invalid
  UTF-8, line breaks, different Unicode forms) and special files such as pipes, sockets and devices.
- **0.8.16 Big files and marking:** what was tested for real and what was simulated:
  - Real: one file of 2 GiB copied to a local SFTP server and back through the program's copy code,
    a copy canceled and one killed in the middle, both continued from the part file, every copy
    compared by checksum. The planned test with a real file over 100 GB was not finished and moves
    to 0.8.17.
  - Simulated: files of 1 TiB, 1 PiB, 8 EiB and up to the largest size a file can have, from a
    synthetic data source whose content is computed instead of stored: resuming near the end, the
    counters, totals larger than the numbers can hold, the remaining time. A whole terabyte streamed
    through the copy code in memory.
  - Real: a full disk in the middle of a transfer on both sides (a small RAM disk, also behind the
    local SFTP server), and sparse files. A disk quota is simulated (the error a quota gives).
  - Real: the program killed (SIGKILL) at random moments of copying and moving to and from a server,
    uploading an edited file, packing, extracting, and saving the settings and the password vault.
  - Marking as in other file managers: Shift+click marks a range from the last clicked item, Shift
    with the arrows, Page Up, Page Down, Home and End extends it and going back unmarks only that
    range, a click without a key keeps the marks, Ctrl+A marks all and a second press unmarks.
    Measured in a folder of a million files.
- **0.8.17 Bad connections:** what was tested:
  - Transfers to and from a local SFTP server in a private network namespace whose line was made
    bad on purpose: high latency, jitter, packet loss, a speed changing every few seconds, repeated
    short and long drop-outs, a connection frozen for longer than the keepalive limit, and all of it
    at once. A big file and a folder of small files went up and back through the program's own
    copy code, and every file was compared by checksum. Fixed on the way: a slow line no longer
    drops a working connection, a transfer goes on as long as it gets further, and a file
    continued several times is counted once.
  - Also: the path above a panel is clickable part by part, bigger colored icons in the bar above
    the panels, the mask dialogs remember their options, packing and extracting clean up after a
    kill, the first key of the quick filter in a folder of a million files under 0.1 s, the news in
    the program translated into all eight languages, and release tags always annotated.
  - Not done, moved to 0.8.18: a real file over 100 GB copied to a server and back, a transfer
    running overnight with memory checks, and the computer going to sleep and waking up during a
    session.
- **0.8.18 Servers and distributions:** what was tested:
  - A line that is down for 30 seconds and up for only 8, again and again, is survived: a
    connection on which nothing comes from the server for 15 seconds while the program waits is
    taken as dead and made again at once (a ping after 5 seconds tells a slow server from a dead
    line), reconnection is tried every few seconds in the first three minutes, and an upload
    continues from the part the server confirmed. The other bad-line scenarios of 0.8.17 measured
    again, next to OpenSSH's `sftp`.
  - Downloading over a line with a long round trip: the SSH channel window of the library (2 MB)
    held the transfer back; with 8 MiB a download at 100 ms takes about half the time.
  - Servers without a physical machine, in user namespaces without root: OpenSSH 7.6, 8.2, 9.2,
    9.6 and 10.0, Dropbear 2022 and 2025, an SFTP-only account without a shell, BusyBox (Alpine)
    and a chroot SFTP server. Each: connecting, listing, transfers both ways with SHA-256, F4,
    permissions, renaming, deleting, archives, and what the program does without `du`, `find`,
    `df` or a shell.
  - The deb, rpm and AppImage packages installed by each distribution's package manager and used
    in the program's window (on a virtual screen): Debian, Ubuntu, Linux Mint, Fedora, openSUSE
    and Arch.
  - A faulty and hostile server: false sizes, names with `..`, `/` and control characters, links
    pointing outside the target folder, huge and endless listings, a file without end, a server
    that stops answering. Nothing is written outside the target folder, and nothing crashes or
    hangs.
  - The AppImage on a minimal system without network, as the AppImage catalog tests it: it now
    carries the libraries the program opens at run time that such a system lacks (keyboard, X11
    cursor and input, Wayland cursor and EGL) and uses them only where the system has none, and a
    start that fails before the window opens shows its error in a window instead of ending
    silently. Both are checked on every build.
  - Moved to a separate test machine: a real file over 100 GB, a transfer running overnight with
    memory checks, the computer going to sleep and waking up during a session, a network cable
    pulled out (all done in 0.8.19), a Windows server and a NAS (not done yet).
  - Not done yet, moved to 0.8.19 and then to 0.8.20: X11 against Wayland, high-DPI screens, ACLs,
    extended attributes, and time zones and server clocks that are off.
- **0.8.19 Updates and real hardware:** what was done:
  - Done: the AppImage carries update information for AppImageUpdate (the newest release on
    GitHub, pre-releases included) and every release has its `.zsync` file, listed in
    `SHA256SUMS` (suggested by the AppImage catalog). Checked on every build: an older AppImage
    finds the update and updating it gives the new AppImage byte for byte.
  - Done on a separate test machine (Linux Mint 22.3, 1 Gbit/s network): a file of 110 GiB
    uploaded over the network and downloaded, each canceled at half and resumed, and killed at
    half and resumed by a new process, every copy equal by SHA-256 (the upload at up to 117 MB/s,
    OpenSSH's `sftp` 116 MB/s; memory under 18 MB, 9 open files); the real window uploading 100 GiB
    through the night at its speed limit of 4 MiB/s for 7 hours, its memory constant (224 MB, less
    than 0.2 MB more after 7 hours), 20 open files throughout, the copy equal; the AppImage of 0.8.18
    started on the real system by an ordinary user, without installing it.
  - Done on the same test machine, by hand, with the program's real window: its network cable
    pulled out for 30 seconds and for 5 minutes during an upload and during a download, and the
    test machine put to sleep and woken during an upload. Each time the program noticed the loss
    within 16 seconds, connected again within 7 seconds of the test machine being reachable,
    finished the transfer without the user, and the copy was equal by SHA-256.
  - Not done yet: a Windows server and a NAS.
- **0.8.20 Polish:** what was done:
  - The remaining time of a transfer is calm: it counts down by itself, does not go up because of a
    short drop in speed and shows "unknown" while the line stands still. Replayed on the recorded
    outages of 0.8.19 it went up 0 to 3 times instead of 94 to 304 times; uploading 110 GiB on the
    test machine it was within 5 % at every quarter.
  - X11 and Wayland (Weston at scale 1 and 2), high-DPI screens at 125, 150 and 200 %, and the 22
    dialogs in German, French and Polish: nothing cut off. The function-key bar keeps the labels that
    fit. The symbols of the window come from the program's own icons, so a system without fonts shows
    them too.
  - The menu bar by keyboard (F10); screen readers hear the folder list, the item under the cursor,
    the buttons and the dialogs; the transfer error summary is a dialog like the others.
  - ACLs, extended attributes and times: what a copy keeps is tested against a real OpenSSH server
    and written in the help; zip times are kept in UTC to the second, right in every time zone.
  - Fuzz testing of everything the program reads, on every build (one crash found and fixed); the
    licences, sources and security advisories of all dependencies checked with cargo deny on every
    build; the code compared with all 904 libraries in the local cache (nothing taken over); a load
    test with 10 000 files on every build, compared with the build before.
  - Two copies of the program at once keep each other's changes (servers, settings, saved
    passwords). A damaged settings file is kept as a copy and costs only what is damaged; values
    of newer versions stay in the files, so going back to an older version and returning loses
    nothing. Tested with the settings of every version from 0.1 to 0.8.19.
  - Signed checksums of the release files and reproducible packages, checked by a second build in
    CI; the AppImage update from 0.8.19 is checked against the release's checksums.
  - Moved to 0.9: keeping ACLs and extended attributes on local copies, a warning for a server clock
    that is off, context menus by keyboard. Not done yet: a Windows server and a NAS.

## 0.9.x: small new features

Each release brings three to five of them. Cheap features and the ones others build on come first.
Every new feature comes with tests of its error cases, help and translations in all eight languages,
and takes over the settings of older versions; before each release the kill, bad-line and two-copies
tests are run again.

- **0.9.0 Settings window:**
  - Settings in a window of their own, with a tree of sections and a filter. The settings now
    spread over the Settings menu (speed limit, conflict choices, editor, locking of saved
    passwords, automatic refresh) are there too. The later settings of 0.9 go into it.
  - Duplicating a connection in the site manager: a copy with all settings; you choose whether the
    saved password is copied too.
  - Context menus by keyboard (the Menu key and Shift+F10), for files and for tabs: the tab color and
    "Close other tabs" can only be reached with the mouse now (everything else has a key or is in
    the menu bar, F10).
  - A warning when a server's clock is off by more than a minute (it matters for "Overwrite only
    older" with files changed on that server).
- **0.9.1 Jump host:**
  - Server keys checked against `~/.ssh/known_hosts` too, next to the program's own file. The program
    never writes to `~/.ssh/known_hosts`: keys you accept go only to its own file. When the two files
    disagree about a server, the program does not connect and tells you why. (Comes first: the jump
    host checks the keys of two servers.)
  - Connecting through a jump host (ProxyJump): a connection can go through another saved
    connection, with its own key or password; the keys of both servers are checked, the connection
    is restored after a drop-out, and the terminal (Ctrl+P) goes the same way.
  - Trash for local deleting.
- **0.9.2 SSH config and proxies:**
  - `~/.ssh/config`.
  - SOCKS5 and HTTP CONNECT proxies and `ProxyCommand`, also when they come from `~/.ssh/config`.
  - Several ways to one server: a connection can have more than one address, for example one in the
    local network and one through a jump host. They are tried in the order you set, the local network
    first; when one does not work, the next is used. After a drop-out the program starts again from
    the first one, and it shows which way you are connected.
- **0.9.3 Looks:**
  - Dark theme and color themes built on one palette.
  - Display density: icon size, row height, font.
  - Layout: show or hide parts of the window (hiding the toolbar and the bar above the panels already
    works).
  - Custom columns: which columns, in which order.
- **0.9.4 Control:**
  - Mouse and keyboard behavior.
  - Custom keyboard shortcuts.
  - Ignored files by masks.
  - Your own viewer, file comparison tool and terminal (your own editor already works).
- **0.9.5 Command line and records:**
  - A command line under the panels: on the left the current folder, then a field for a command. In
    a local panel the command runs in the current folder, in a server panel over SSH in the current
    remote folder. The output appears in a window. On a production server the command needs a
    confirmation first. A history of commands, never with passwords in it.
  - A log of operations you can browse.
  - Import of saved sites from WinSCP and FileZilla.
  - Keeping ACLs and extended attributes when copying between disks (a choice, off by default). SFTP
    cannot carry them, so to and from a server it stays as it is.

What turns out to be hard or risky moves to 1.0, with the reason written here.

Already available: ssh-agent and passphrase-protected keys, colors by file type, an external editor,
and "Open with" associations per file type.

## 1.0 alpha and later: large features

- A Windows version from the same code, with the system-specific parts in separate modules.
- FTP and FTPS (explicit and implicit TLS) as another protocol: listings, transfers, resuming,
  reconnecting after a drop-out and the same protections of your data as with SFTP. Passwords only
  in the password vault; plain FTP without encryption only when you allow it in the connection
  settings.
- File sharing in the local network, compatible with the LocalSend protocol.
- A transfer queue that survives a restart of the program.
- Comparing and synchronizing folders.
- Renaming many files at once.
- A customizable toolbar.
- A built-in editor.
- Sharper text: letter positions on whole pixels.

Then beta, release candidates and 1.0.

## Not planned

- Plugins in the style of Double Commander or Total Commander (archive, file system, viewer and
  content plugins).
- A Flatpak package. RolidSCP is offered as deb, rpm and AppImage.

## Suggesting a feature

Open an issue: [github.com/MrRolid/RolidSCP/issues/new](https://github.com/MrRolid/RolidSCP/issues/new).
Describe what you want to do and why. A real situation from your work helps more than a list of
features from another program. Bugs, and above all anything that could lose data, go before new
features.
