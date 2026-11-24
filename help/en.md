# RolidSCP help

RolidSCP is a dual-panel file manager for local disks and SFTP servers. Each panel has tabs; a tab is
one place (a local folder or a folder on a server) with its own name and color. Every function is in
the menus, most also on the toolbar and the function-key bar at the bottom. The list of all keyboard
shortcuts is on the second page of this window.

**F10** opens the menu bar without the mouse: **←** **→** go between the menus, **↑** **↓** through the
items, **→** into a submenu, **Enter** picks an item, **Esc** closes the menu.

With a screen reader (Orca) the list says the item under the cursor (its name, "folder" or the size,
and whether it is selected), the buttons and dialogs their names.

## Panels and tabs

- **Tab** switches between the left and the right panel. The active panel has a stronger header.
- **Enter** or a double click opens a folder, an archive (as a read-only folder) or a file in its
  default application. Programs and scripts are never started without asking; files on a server are
  never run. **Backspace** goes one level up.
- **Ctrl+T** opens a menu for a new tab: the home folder, favorites, saved servers and recent places;
  type to filter it. **Ctrl+W** closes the tab, **Ctrl+Tab** switches tabs. A right click on a tab
  offers Duplicate, Rename, Color, Close and Close other tabs; a double click renames it. Hover over a
  tab to see its server, address and path. **Ctrl+K** finds a tab or a saved server by name.
- **Ctrl+D** adds the folder to the favorites or removes it. The favorites and the recent folders are
  under the ★ button of the bar (`*` when the bar is hidden); **File → Favorite folders…** renames and
  orders them.
- In the **Ctrl+T** menu (and the **+** button by the tabs) the favorites are always shown: **＋ Add the
  current folder** adds the folder of the active panel (also on a server), ✏ renames a favorite and 🗑
  removes it (click once more to confirm, **Esc** goes back).
- Start typing to filter the list; **Esc** clears the filter.
- **Ctrl+F1**, **Ctrl+F2** and **Ctrl+Shift+F1** switch between the brief list, the detailed list and
  thumbnails. **Alt+←** and **Alt+→** go back and forward in the folder history.
- Every dialog works the same: **Enter** presses the highlighted button, **Esc** or the ✕ cancels,
  **Tab** moves between the fields. Drag a dialog by its title bar; ⓘ shows more help.
- Tabs, colors and folders are saved all the time and restored at the next start.
- Above each panel there is a bar: a drop-down with the folders on the path from the root (**Alt+↓**;
  arrows, **Enter**, **Esc**), for a local panel also the disks of this computer with their size; then
  Back, Forward, Up one level, Root folder (**`Ctrl+\`**), Home folder (**Ctrl+H**), Refresh and the
  favorites (★). Inside an archive the drop-down starts with the folder the archive is in; in search
  results it is not available. **View → Bar above the panels** hides the bar; the `/ .. ~ *` buttons
  then return to the path row.
- Every part of the path is a button: a click opens that folder (also on a server and inside an
  archive; a click on the folder you are in refreshes it), the middle button opens it in a new tab
  next to the current one, and the part under the mouse is highlighted. A path too long for the row
  is shortened from the left with “…” (a click on it opens the drop-down of the path); the last part
  always stays whole. To type a path, double-click the empty part of the row, click the pencil at
  its end or press **Ctrl+G**.
- On its right the path row shows the free space of the disk (a thin bar and “Free: X of Y”), its
  file system (ext4, btrfs, nfs4, fuse.sshfs…) and, for disks of this computer, a badge with the
  disk health; in a narrow panel “Free:”, then the size of the disk and then the file system are
  left out. Inside an archive it shows the disk the archive is on. Click it for the details.
- A right click on the empty space of the list (or on `..`) offers **New file…**, **New folder**,
  **Refresh**, **Add folder to favorites**, **Open terminal here** and **Disk properties…** (free
  space, file system and disk health).
- In the detailed list thin lines separate the columns. Drag a line between the column titles to change
  the width of the column on its right; a double click fits the column to its content. Name takes the
  rest of the panel, so it grows with a wider window; in a narrow panel Permissions and then Modified
  are hidden. The widths are kept for the left and the right panel separately (also after a restart);
  **View → Reset column widths** brings back the defaults. A name or an extension that does not fit ends
  with “…”; hover over it to see it whole.
- The listings keep up by themselves. A folder on this computer shows changes made by other programs
  within a moment. A folder on a server (or on a network disk) is refreshed when you come back to it –
  another tab, the other panel or back to the window – if the listing is older than 5 seconds, and while
  the window is active also every 30 seconds (**View → Automatic refresh**: off, 10 s, 30 s, 1 min or
  5 min; a folder with more than 20 000 items at most every 5 minutes). The cursor, the marks, the
  scroll position, the sort and the computed folder sizes stay. Nothing is refreshed while you rename in
  the list, drag or a dialog is open, and a lost connection is not tried. After your own operations all
  the tabs showing the folder are refreshed; **Ctrl+R** refreshes at once.

## Selecting files

- **Insert**, **Space** or **Ctrl+click** marks an item; **Ctrl+A** marks all or nothing.
- **Shift+click** marks the items from the last clicked or marked one to the clicked one. **Shift**
  with **↑ ↓ PgUp PgDn Home End** marks from there to the cursor as it moves; going back unmarks only
  what this range marked. **Tools → Select to the top / Select to the bottom** does the same as
  **Shift+Home** and **Shift+End**. A click without a key moves the cursor and keeps the marks.
  The start of the range is found again by its name after a refresh, a sort or the quick filter;
  `..` is never marked. Dragging still copies, and **Shift** held when the files are dropped moves
  them (Shift+click alone never moves anything).
- **Num+** marks a group by a mask such as `*.jpg;*.png`, **Alt+−** unmarks one, **Alt+I**
  inverts the selection.
- The mask dialogs of **Num+** and **Alt+−** remember the last mask and whether **Include folders**
  is on, each for itself and also after a restart. On a server marked as production
  **Include folders** is always off when the dialog opens, and the choice made there is not
  remembered.
- The line under the list shows how many items are marked and their size.
- **Alt+Space** (or **Shift+Space**) counts the size of the folder under the cursor or of the marked
  folders, **Alt+Shift+Enter** of all folders in the panel.

## Copying, moving, renaming

- **F5** copies the marked items (or the item under the cursor) to the folder of the other panel,
  **F6** moves them. Both work between disks and servers, also between two servers.
- The target folder of copying and moving can be edited, picked from the recent targets or chosen with
  **Browse…** (on the disk or on the server).
- Drag files with the mouse to the other panel, onto a folder, onto `..` or onto a tab (hold them over a
  tab to switch to it). Dragging copies, with **Shift** it moves, **Esc** cancels the drag. The copy or
  move dialog always asks first.
- When a file already exists in the target, choose: skip, overwrite only older files, overwrite, or
  resume an interrupted transfer. The default choice can be set in the Settings menu.
- Files are written under a temporary name and renamed when they are complete, so a broken
  transfer never leaves a half-written file under the real name.
- Resuming continues only a part file (`.filepart`) of the same source file: its path, size and
  modification time are kept next to it. A part file of another or an unknown source is left alone
  and reported; choose **Overwrite** to start over.
- When the target disk fills up or the disk quota is used up, the transfer stops and says so; an
  older file of the same name stays as it was and the written part is kept. Free some space and press
  **Try again** in the transfer queue: the transfer continues where it stopped. A server often
  reports only "Failure"; the program then looks at the free space on the server to tell.
- Moving between disks or servers deletes a source file only when it did not change during the
  transfer (size and modification time) and the copy has the same size. With **Verify by checksum
  before deleting the source** (on by default when moving to a server) both are read again and
  compared first; a difference is reported and the source stays.
- **F2** or **Shift+F6** renames right in the list: the name without the extension is selected,
  **Enter** confirms, **Esc** cancels.
- **F7** creates a folder, **F8** deletes (with a confirmation). Deleting runs in the transfer queue
  with the number of deleted items; **Cancel** stops it (what is deleted by then stays deleted).
- Pipes (FIFO), sockets, devices and symbolic links that lead nowhere (a missing target or a loop) are
  not copied: they are skipped and listed in the report at the end, the rest of the copy goes on.
- Unusual names are copied exactly: line breaks, a backslash, names of 255 bytes, and on this
  computer's disks also names that are not valid UTF-8. The list shows them on one line: control
  characters as `\n` or `\t`, each invalid byte as `\xFF`. A name that is not valid UTF-8 cannot be
  sent to a server (SFTP takes names as text); it is reported.
- The transfer queue runs up to three transfers at a time. The speed limit is in the copy dialog.
- **What a copy keeps** besides the content: the permissions (read, write, execute; setuid, setgid
  and sticky are left out) and the modification time. It does not keep the owner and group (the copy
  belongs to the user who writes it), ACLs, extended attributes (also SELinux labels and file
  capabilities) or hard links (each link becomes a file of its own). SFTP cannot carry ACLs or extended
  attributes at all. An overwritten file is replaced by a new one, so its own ACLs and attributes are
  gone too; the new file gets what the folder gives new files (for example a default ACL).
- **Times** are shown in this computer's time zone, the times of files on a server too, whatever the
  server's own time zone. **Overwrite only older** compares the times to the second, independently of
  time zones. A copy keeps the time of its source, so a wrong clock on the server does not matter for
  copied files; it matters for files changed on the server, which get the server's time: with the
  server's clock an hour ahead, a file changed there looks an hour newer.
- **Dragging out to other applications** (X11): drag files or folders out of the window, for example
  onto a file manager or an editor. Local files are copied, with **Shift** moved. Files from a server or
  an archive are only copied: an application that supports saving directly (XDS) gets them downloaded
  into its folder through the transfer queue; otherwise they are downloaded into a private temporary
  folder first (over 1 GiB you are asked), which is deleted when no longer used and at exit. Some
  applications wait only about 30 seconds for the files, so large downloads may not get through that
  way. Back over the window the drag continues inside; **Esc** cancels it. Not available on Wayland.

## Servers

- **Ctrl+O** opens the server manager. Enter the server as `host` or `user@host:port`, save it as a
  profile with a name and a tab color, and connect with **Enter**.
- Mark a server as a **production server**: its tabs get a red header, and dangerous actions ask
  once more (deleting cannot be confirmed with Enter).
- On a production server copying, uploading, moving and creating a folder cannot be confirmed with
  Enter either, and copying never starts with Overwrite (Skip existing is preselected instead).
- The server's key is checked against `~/.ssh/known_hosts`. A new key is added only after you
  confirm its fingerprint; a changed key is refused.
- Login tries ssh-agent, your keys, then the password. Saved passwords are kept encrypted, protected
  by a master password.
- Saved passwords lock again a set time after their last use (Settings menu → master password, 60
  seconds by default); the status bar shows the time left. Locking also forgets the passwords kept for
  reconnecting.
- When the connection drops, RolidSCP connects again by itself and interrupted transfers continue. A
  transfer goes on as long as it gets further between the drop-outs; after five drop-outs in a row
  without progress it stops, and you can start it again from the queue.
- When nothing comes from the server for 15 seconds while the program waits for it, the connection
  is taken as dead and made again at once (a short check after 5 seconds tells a slow server from a
  dead line). In the first three minutes after a drop-out it is tried every few seconds, then every
  10 seconds, after ten minutes every 30 seconds. An upload continues from the last part the server
  confirmed.
- The same holds when the network cable is pulled out for minutes or the server goes to sleep: the
  program keeps trying, and the transfer continues as soon as the server can be reached again.
- On a server without a shell or without some tools (`du`, `find`, `df`, `sha256sum`, for example
  BusyBox or an SFTP-only account), everything is done over SFTP alone: folder sizes and searching
  by walking the folders, checksums by reading the files.
- A faulty or hostile server cannot make the program write outside the target folder: names with
  `/`, `..` or nothing in them are refused, control characters in names are shown as `\xNN`. A
  folder listing of more than 5 million entries is stopped, and a file the server sends far past its
  size is not taken.
- **Ctrl+P** opens the system terminal (`$TERMINAL` or `x-terminal-emulator`) in the panel's folder.
  On a server it runs `ssh` into the folder, with the same host key check as RolidSCP (an unknown or
  changed key ends the connection). A saved password is passed by a one-time helper (`SSH_ASKPASS`)
  that answers only the password question of that server, once; with locked passwords you are
  asked for the master password first, otherwise ssh asks itself. Production servers ask before
  the terminal opens. The password never goes into a command line, a file or the log.
- **Ctrl+L** switches the active panel back to the local disk.

## Viewing files (F3)

- **F3** opens the file in the viewer, a window of its own (several can be open at once). It is also
  in the context menu, the File menu and the key bar. The viewer never runs anything and loads nothing
  from outside: HTML, XML and SVG are shown as text.
- Files of any size open at once: only the part on the screen is read. A file on a server is not
  downloaded whole; a file in an archive is unpacked to a temporary file first. Line numbers with
  **~** are estimates until the lines are counted.
- **1**, **2** and **3** switch between text, hexadecimal and image; binary files are shown as hex by
  themselves.
- **W** wraps lines, **N** shows line numbers, **Ctrl+G** goes to a line, a percentage or an offset,
  **Ctrl+Home** and **Ctrl+End** go to the beginning and the end. The Encoding and Highlighting menus
  change the encoding and the language of the syntax highlighting (off for files over 20 MB, see
  **Settings → Settings… → Viewer (F3)**).
- **F** follows the end of a growing file, also on a server; scrolling up pauses it. When the file
  changes, a bar offers to reload it. A lost connection is shown in a bar and the view continues by
  itself when it is back.
- **Ctrl+F** or **F7** finds text (match case, whole words, regular expression); **F3** and
  **Shift+F3** go to the next and the previous hit. The whole file is searched in the background; on
  a server `grep` searches there when it can, otherwise the file is read over SFTP, and the search bar
  says which.
- Select with the mouse or with Shift and the arrows; **Ctrl+C** copies the selection (at most 32 MB).
- Images (JPEG, PNG, GIF also animated, WebP, BMP): **F** fits the picture to the window, **0** shows
  it at 100 %, the wheel zooms, dragging moves it, **R** and **L** rotate the view, the arrows and
  **PgUp**/**PgDn** show the previous and the next image of the folder. An image on a server is
  downloaded only up to 50 MB (adjustable).
- **F4** edits the file as in the panel; **Esc** or **Q** closes the viewer.

## Editing and opening files

- **F4** opens the file in an external editor. A file from a server is uploaded back after every
  save; if it changed on the server in the meantime, you are asked what to do.
- The local copy is kept in a private folder on disk (`~/.local/state/rolidscp/edit`, for you only)
  and deleted when editing ends. An upload that fails for another reason than a lost connection is
  tried again twice, then reported. The time of the last upload is shown with seconds.
- A server without atomic rename gets a changed file in steps (the original is moved aside first).
  If the connection or the program breaks in between, the step is finished or undone at the next
  connection to that server, and you are told which files it concerned.
- **Shift+F4** (also **File → New file…** and the context menu) creates a new file and opens it in the
  external editor. The name under the cursor is offered. On this computer the empty file is created at
  once; on a server it is created by the first save, so closing the editor without saving creates
  nothing. An existing name opens that file for editing.
- **Shift+Enter** or the context menu (right click) offers **Open with…**, permissions
  (**Ctrl+Shift+A**) and copying the name, path or `sftp://` address to the clipboard.
- After a save in the editor (**F4**, **Shift+F4**) the size and the date of a file on this computer are
  updated in the panel.

## Archives

- **Alt+F5** packs the marked items into zip, 7z, tar, tar.gz or tar.zst, optionally with a password
  (zip and 7z).
- **Alt+F9** extracts an archive into the other panel, with protection against dangerous paths and
  archive bombs. **Alt+Shift+F9** tests an archive.
- Packing or extracting stopped by a crash or a kill leaves its unfinished `.filepart` file only
  until the next start of the program, which deletes it.
- **Enter** on an archive opens it as a read-only folder; **F5** copies files out of it.
- Times in a zip are kept to the second and in UTC (as Info-ZIP does), so they are right in another time
  zone too. A zip from a program that writes only the old DOS time (no time zone, even seconds only) is
  read in this computer's time zone.

## Search and checksums

- **Ctrl+F7** (or **Ctrl+F**) searches by name (mask, text or regular expression) and by content.
  On a server the search runs there with `find` and `grep` when possible.
- Checksums (MD5, SHA-1, SHA-256, SHA-512, BLAKE3, CRC32) are in the File menu and the context
  menu; they can be saved in the format of `sha256sum -c`.

## Disk health (SMART)

- The badge in the path row shows the state of the disk: **OK**, **warning** (orange) or **critical**
  (red). Click it for a plain-language explanation and advice.
- The details show the model and serial number, the type (HDD, SSD, NVMe), wear and the estimated
  remaining life, temperature, power-on time, power cycles, unexpected shutdowns, bad and unstable
  sectors, transfer (CRC) errors (usually a cable or a connector) and the data written. All the
  attributes are in the Advanced section.
- The data come from UDisks2 (no administrator rights needed), or from `smartctl` when it runs
  without them. They are read in the background, at most once a day per disk (kept in
  `~/.cache/rolidscp/smart.json`); **Reload** reads the disk again now.
- Warning: SSD wear from 60 %, any bad, unstable or unreadable sectors, a hard disk from 45 °C or an
  SSD from 60 °C. Critical: wear from 95 %, more than 50 bad sectors or a growing number of them, a
  hard disk from 55 °C or an SSD from 75 °C, an NVMe critical warning or low spare space, a failed
  self-test. The temperature is checked every 5 minutes.
- Network file systems, virtual disks and USB adapters without SMART have a grey badge; its tooltip and
  the details say why.
- The badge is a small heart-rate icon: grey when the state is not known, green when the disk is fine,
  orange or red with the most serious reason next to it (for example “Wear 87 %”, “Bad sectors” or
  “61 °C”). Hover over it to see all the reasons. The free-space bar turns orange under 10 % free space
  and red under 5 %.

## Settings

- **Settings → Settings…** holds the language, the date format, the font (built-in or from the
  system), text size, hinting and contrast, the toolbar, file type colors, thumbnails and archive
  limits.
- The language follows the system by default (`LC_ALL`, `LC_MESSAGES`, `LANG`). It can be chosen in
  Settings and changes at once. Your own translation `~/.config/rolidscp/lang/<language>.po` is used
  instead of the built-in one.

## Files and log

- Configuration: `~/.config/rolidscp/` (readable only by you).
- **The program started twice:** the two windows work together. Servers, settings and saved
  passwords that one window saves appear in the other within a few seconds, and when both change
  them nothing is lost (where both changed the same thing, the later change counts). Each window
  saves its own tabs; the window closed last decides which tabs open next time.
- **A damaged settings file** (edited by hand, cut off) does not cost everything in it: what can be
  read is used, the rest is named in the error line, and the file as it was is kept next to it as
  `<name>.damaged`. Going back to an older version and returning keeps the settings (tested back to
  0.8.13); from 0.8.20 on, values that the older version does not know stay in the files.
- Log: `~/.local/state/rolidscp/rolidscp.log` (Help menu → Open log). It never contains passwords.
- The AppImage can be updated with AppImageUpdate (or `appimageupdatetool
  RolidSCP-<version>-x86_64.AppImage`): it finds the newest release on GitHub and downloads only the
  parts that changed. The new AppImage is saved next to the old one under the name of the release;
  the old one stays.
