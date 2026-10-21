RustyCorn @VERSION@ for Windows
===============================

RustyCorn records and shows the runs of an ÄKTAprime or ÄKTAprime plus that
is connected to this PC with a USB-to-serial (RS-232) cable, saves each run
as a CSV file and keeps the methods the instrument stores on the PC.

RustyCorn is an independent, unofficial project. It is not affiliated with,
authorised, endorsed or supported by Cytiva (formerly GE Healthcare).


Starting
--------
Unzip this folder anywhere (for example into Documents) and double-click
RustyCorn.exe. Nothing is installed and no administrator rights are needed.

A console window opens first. It shows where the data are saved and asks for
a log-file prefix (a label such as a sample name or your initials). Press
ENTER to keep the last one. Then the main window opens. Leave the console
window open: closing it ends the program at once.

Windows may warn "Windows protected your PC" because the program is not
signed. Click "More info", then "Run anyway".

From a Command Prompt in this folder:

    RustyCorn.exe              main window (live or demo, as set in Settings)
    RustyCorn.exe COM3         use serial port COM3 (and remember it)
    RustyCorn.exe run.csv      open a saved log, read-only
    RustyCorn.exe --methods    only the method editor (no instrument needed)

To stop, close the main window (or press Ctrl-C in the console).


Choosing the COM port
---------------------
1. Plug in the cable and switch the instrument on.
2. Open Device Manager, expand "Ports (COM & LPT)" and note the number of
   "USB Serial Port (COMn)". Use an FTDI cable; Prolific (PL2303) cables
   are unreliable with this instrument.
3. In RustyCorn, click Settings. Switch Demo mode off, pick the port from the
   list or type it (e.g. COM3) and press Enter.
4. Volume unit: 1 µL per count for an ÄKTAprime, 10 µL per count for an
   ÄKTAprime plus. If volumes and flow rates are 10x off, change this.

Only one program can use a COM port at a time: close any other program that
uses the instrument (such as PrimeView) before starting RustyCorn.


Data folder
-----------
Each run is saved as its own CSV file, <prefix>_<date>_<time>.csv, in the
data folder. Set it under Settings -> Data folder -> Choose. Until one is set,
files go to the folder RustyCorn was started from (when double-clicked: this
folder). Logs that never received data are deleted.


Methods folder
--------------
Methods the instrument saves to the PC ("Save method -> PC") and loads from it
("Run Stored Method -> PC") are kept in the methods folder:
PrimeMeth_001.txt ... PrimeMeth_999.txt, the same files PrimeView uses. By
default it is the "methods" folder inside the data folder; choose another one
under Settings -> Folders -> Methods folder (for example the folder PrimeView
used). Overwritten and deleted methods are moved to its "old" subfolder.
Click "Methods" in the top bar to view, edit and name them, and to enter each
method's run setup (column, column volume, buffers, notes).

Click "Analyze Run..." to open a saved run, look at it and export it as PDF.


Settings
--------
RustyCorn keeps its settings in
    %APPDATA%\RustyCorn\settings.toml
Delete that file to start over with the defaults.


Licence
-------
RustyCorn is released under the MIT licence (LICENSE.txt). It contains the
fonts Inter and JetBrains Mono, both under the SIL Open Font License 1.1
(licenses\Inter-LICENSE.txt, licenses\JetBrainsMono-OFL.txt).
