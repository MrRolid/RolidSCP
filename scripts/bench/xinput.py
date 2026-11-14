#!/usr/bin/env python3
"""Send keyboard and mouse input to an X display through XTEST (for the
frame-time measurement in gui-frames.sh; uses only libX11 and libXtst).

  xinput.py focus TITLE | key COMBO | type TEXT | wheel DOWN|UP N | move X Y | sleep S ...

COMBO is a key name with optional modifiers: pgdn, ctrl+end, shift+f5...
Run it with DISPLAY set to the test display (never your own session).
"""
import ctypes
import sys
import time

x = ctypes.cdll.LoadLibrary("libX11.so.6")
t = ctypes.cdll.LoadLibrary("libXtst.so.6")
x.XOpenDisplay.restype = ctypes.c_void_p
x.XOpenDisplay.argtypes = [ctypes.c_char_p]
x.XStringToKeysym.restype = ctypes.c_ulong
x.XStringToKeysym.argtypes = [ctypes.c_char_p]
x.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x.XDefaultRootWindow.restype = ctypes.c_ulong
x.XFlush.argtypes = [ctypes.c_void_p]
x.XQueryTree.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong),
                         ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)), ctypes.POINTER(ctypes.c_uint)]
x.XFetchName.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_char_p)]
x.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
x.XRaiseWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
t.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
t.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
t.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]

d = x.XOpenDisplay(None)
if not d:
    sys.exit("xinput.py: no display (set DISPLAY)")

MODS = {"ctrl": "Control_L", "shift": "Shift_L", "alt": "Alt_L"}
NAMES = {"enter": "Return", "esc": "Escape", "tab": "Tab", "up": "Up", "down": "Down", "pgdn": "Next", "pgup": "Prior",
         "home": "Home", "end": "End", "bs": "BackSpace", "space": "space"}


def code(name: str) -> int:
    return x.XKeysymToKeycode(d, x.XStringToKeysym(NAMES.get(name, name).encode()))


def key(combo: str) -> None:
    parts = combo.split("+")
    mods, k = parts[:-1], parts[-1]
    for m in mods:
        t.XTestFakeKeyEvent(d, code(MODS[m]), 1, 0)
    c = code(k)
    t.XTestFakeKeyEvent(d, c, 1, 0)
    x.XFlush(d)
    time.sleep(0.02)
    t.XTestFakeKeyEvent(d, c, 0, 0)
    for m in reversed(mods):
        t.XTestFakeKeyEvent(d, code(MODS[m]), 0, 0)
    x.XFlush(d)
    time.sleep(0.03)


def find(win: int, title: str) -> int:
    nm = ctypes.c_char_p()
    if x.XFetchName(d, win, ctypes.byref(nm)) and nm.value and title.encode() in nm.value:
        return win
    root, parent = ctypes.c_ulong(), ctypes.c_ulong()
    kids, n = ctypes.POINTER(ctypes.c_ulong)(), ctypes.c_uint()
    if x.XQueryTree(d, win, ctypes.byref(root), ctypes.byref(parent), ctypes.byref(kids), ctypes.byref(n)):
        for i in range(n.value):
            w = find(kids[i], title)
            if w:
                return w
    return 0


args = sys.argv[1:]
i = 0
while i < len(args):
    c = args[i]
    if c == "key":
        key(args[i + 1])
        i += 2
    elif c == "type":
        for ch in args[i + 1]:
            key(ch)
        i += 2
    elif c == "wheel":
        button = 5 if args[i + 1] == "down" else 4
        for _ in range(int(args[i + 2])):
            t.XTestFakeButtonEvent(d, button, 1, 0)
            t.XTestFakeButtonEvent(d, button, 0, 0)
            x.XFlush(d)
            time.sleep(0.02)
        i += 3
    elif c == "move":
        t.XTestFakeMotionEvent(d, -1, int(args[i + 1]), int(args[i + 2]), 0)
        x.XFlush(d)
        time.sleep(0.1)
        i += 3
    elif c == "sleep":
        time.sleep(float(args[i + 1]))
        i += 2
    elif c == "focus":
        w = find(x.XDefaultRootWindow(d), args[i + 1])
        if not w:
            sys.exit("xinput.py: window not found: " + args[i + 1])
        x.XRaiseWindow(d, w)
        x.XSetInputFocus(d, w, 1, 0)
        x.XFlush(d)
        time.sleep(0.2)
        i += 2
    else:
        sys.exit("xinput.py: unknown command " + c)
x.XFlush(d)
