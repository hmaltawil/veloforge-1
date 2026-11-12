VeloForge for Windows
=====================

The Windows version of VeloForge is new. The app, the 3D world, workouts,
sounds and the phone remote work. Connecting to a smart trainer or heart
rate strap over Bluetooth has NOT been tested on Windows yet - if you try
it, please report what happened (see step 3).

Needs: Windows 10 or 11 (64-bit), Bluetooth LE, and a graphics card
with OpenGL 3.3 (almost any from the last ten years).


1. Start it
-----------
Unzip this folder anywhere (for example Documents\VeloForge) and
double-click veloforge.exe.

Windows may say "Windows protected your PC", because the program is new
and not signed. Click "More info", then "Run anyway".

To look around without a trainer, open a terminal in this folder
(right-click in the folder > "Open in Terminal") and run:

    .\veloforge.exe --demo --workout


2. Test Bluetooth
-----------------
- Make sure Bluetooth is on: Settings > Bluetooth & devices.
- Close Zwift or the Wahoo app on any phone or tablet (a trainer accepts
  only one controlling app).
- In VeloForge, open Devices, pedal to wake the trainer, click Scan,
  then Connect next to it. Do the same for a heart rate strap.
- Ride a few minutes of a workout (ERG) and a few minutes of Free ride,
  and check that the resistance changes.


3. Report what happened
-----------------------
In Devices, click "Copy log", then open an issue at
https://github.com/hmaltawil/veloforge-1/issues and paste the log there.
The log is also saved in %APPDATA%\veloforge\data\veloforge.log

Your settings and rides are kept in %APPDATA%\veloforge\data
