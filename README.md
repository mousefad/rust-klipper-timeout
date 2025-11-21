# Klipper Timeout

This program watches the `klipper` clipboard for KDE/Plasma desktop and clears 
items from the clipboard history after some specified time.  Run this program
from your Autostart tasks to add clipboard history timeouts.

Configuration can be put in `$XDG_CONFIG_HOME/klipper-timeout.toml`, or with
command line options (the latter over-riding the former if both are used).


## Example Config File

```
# Expire clipboard history items after 10 mins.
expiry_seconds = 600

# Check every 10 seconds.
resync_interval_seconds = 10
```


## TODO

* Upgrade zbus version to 5.x and port code as necessary.
* Feature: immediately filter out items from the history that match 
  certain patterns (e.g. crypto keys)
