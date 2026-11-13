VeloForge for Linux
===================

Install
-------
Open a terminal in this folder and run:

    ./install.sh

This copies VeloForge to ~/.local/bin and adds it to your app menu (no
administrator password needed). Then open VeloForge from the menu, or run:

    ~/.local/bin/veloforge

To look around without a trainer:

    ~/.local/bin/veloforge --demo --workout

Update: run  veloforge-update  in a terminal. It downloads the newest
release from GitHub, checks it and installs it. (Or download the new
version yourself and run ./install.sh again.)
Remove: ./install.sh --uninstall (your settings and rides are kept).


Needs
-----
- A 64-bit PC with Ubuntu 22.04, Debian 12, Fedora 36, Linux Mint 21 or
  newer (any distribution with glibc 2.35 or later).
- Bluetooth LE, through BlueZ (the "bluez" package).
- A graphics card with OpenGL 3.3 (almost any from the last ten years).
- For workout sounds: pw-play, paplay or aplay (usually already installed).

If it does not start and the terminal names a missing library (such as
libXcursor or libxkbcommon-x11), install it; on Fedora:
sudo dnf install libXcursor libXrandr libXi libxkbcommon-x11


Your data
---------
Settings and rides are kept in ~/.local/share/veloforge.
Problems? In Devices, click "Copy log", then open an issue at
https://github.com/hmaltawil/veloforge-1/issues and paste the log there.
