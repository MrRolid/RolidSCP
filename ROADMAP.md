# Roadmap

RolidSCP moves your files, often on servers that matter. So reliability comes first and new features
second: the 0.8 series gets no new features, only fixes and stress tests of what is already there.
Small new features come in 0.9, large ones from 1.0 alpha.

This is a plan, not a promise. There are no dates, and the order within a section can change.
Releases are listed on the [Releases](https://github.com/MrRolid/RolidSCP/releases) page.

## 0.8.x: hardening

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
- **0.8.20 Polish:** moved from 0.8.18 and 0.8.19: X11 against Wayland, high-DPI screens, ACLs,
  extended attributes, and time zones and server clocks that are off. A few symbols of the window
  (the sort arrow, the plus of the tabs, the disk and info signs) come from the system's fonts and
  are missing on a system without any; they should come from the program's own icons. Consistent
  controls in every dialog, fuzz testing of everything the program reads (archives, server listings,
  `known_hosts`), a licence check of all dependencies and a check of the code for matches with other
  projects, and performance measured on every build. Two copies of the program running at once;
  settings from older versions, going back to an older version and damaged settings files.
  Keyboard-only use, screen readers and long translated texts. Signed checksums of the release files
  and reproducible builds.

## 0.9.x: small new features

- Settings in a window of their own, with a tree of sections and a filter.
- Ignored files by masks.
- Dark theme and color themes built on one palette.
- Display density: icon size, row height, font.
- Layout: show or hide parts of the window (hiding the toolbar and the bar above the panels already
  works).
- Your own viewer, file comparison tool and terminal (your own editor already works).
- Mouse and keyboard behavior.
- `~/.ssh/config`.
- Connecting through a jump host (ProxyJump): a connection can go through another saved
  connection, with its own key or password; the keys of both servers are checked, the connection
  is restored after a drop-out, and the terminal (Ctrl+P) goes the same way.
- Several ways to one server: a connection can have more than one address, for example one in the
  local network and one through a jump host. They are tried in the order you set, the local network
  first; when one does not work, the next is used. After a drop-out the program starts again from
  the first one, and it shows which way you are connected.
- SOCKS5 and HTTP CONNECT proxies and `ProxyCommand`, also when they come from `~/.ssh/config`.
- Server keys checked against `~/.ssh/known_hosts` too, next to the program's own file. The program
  never writes to `~/.ssh/known_hosts`: keys you accept go only to its own file. When the two files
  disagree about a server, the program does not connect and tells you why.
- Duplicating a connection in the site manager: a copy with all settings; you choose whether the
  saved password is copied too.
- Trash for local deleting.
- Custom keyboard shortcuts.
- Custom columns: which columns, in which order.
- A command line under the panels: on the left the current folder, then a field for a command. In
  a local panel the command runs in the current folder, in a server panel over SSH in the current
  remote folder. The output appears in a window. On a production server the command needs a
  confirmation first. A history of commands, never with passwords in it.
- A log of operations you can browse.
- Import of saved sites from WinSCP and FileZilla.

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
