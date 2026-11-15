VeloForge for macOS
===================

The macOS version of VeloForge is new. It is built for both Apple Silicon
and Intel Macs. If something does not work, please report it (see step 4).

Needs: macOS 11 Big Sur or newer, and Bluetooth LE.


1. Install
----------
Drag VeloForge onto the Applications folder in this window.


2. Start it the first time
--------------------------
VeloForge is not signed with an Apple developer certificate, so macOS
stops it the first time ("Apple could not verify VeloForge..."). To allow
it once:

- Double-click VeloForge in Applications and click Done (not Move to Bin).
- Open System Settings > Privacy & Security, scroll down to the message
  about VeloForge and click "Open Anyway", then confirm.

Or, in Terminal:

    xattr -dr com.apple.quarantine /Applications/VeloForge.app

After that it opens normally. macOS asks once whether VeloForge may use
Bluetooth: allow it, or trainers and heart rate straps cannot connect.
You can change that later in System Settings > Privacy & Security >
Bluetooth.

To look around without a trainer, run in Terminal:

    /Applications/VeloForge.app/Contents/MacOS/veloforge --demo --workout


3. Connect your trainer
-----------------------
- Close Zwift or the Wahoo app on any phone or tablet (a trainer accepts
  only one controlling app).
- In VeloForge, open Devices, pedal to wake the trainer, click Scan,
  then Connect next to it. Do the same for a heart rate strap.


4. Report what happened
-----------------------
In Devices, click "Copy log", then open an issue at
https://github.com/hmaltawil/veloforge-1/issues and paste the log there.
The log is also saved in
~/Library/Application Support/veloforge/veloforge.log

Your settings and rides are kept in ~/Library/Application Support/veloforge
