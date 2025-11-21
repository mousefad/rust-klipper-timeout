# Klipper Timeout

This program watches the `klipper` clipboard for KDE/Plasma desktop setups and clears items from the clipboard history after some specified time. 


## TODO

* Upgrade zbus version to 5.x and port code as necessary.
* Review Cursor-generated code, clean-up and refactor as needed.
* Add --daemonize option with path to log file as arg.
* Add signal handling to dump out clipboard values and times
* Feature: immediately filter out items from the history that match 
  certain patterns (e.g. crypto keys)
