# Klipper Timeout

This program watches the `klipper` clipboard for KDE/Plasma desktop and clears 
items from the clipboard history after some specified time.  Run this program
from your Autostart tasks to add clipboard history timeouts.

Configuration can be put in `$XDG_CONFIG_HOME/klipper-timeout.toml`, or with
command line options (the latter over-riding the former if both are used).


## Example Config File 

Typically located at: `~/.config/klipper-timeout.toml`

```
# Expire clipboard history items after 10 mins (600 seconds)
item_expiry_seconds = 600

# Check for an remove expired items every 10 seconds
update-interval-seconds = 10

# Prevent matching items from being added to the clipboard history
# Note this does not prevent one from copy-pasting an item matching
# one of these regular expressions, just that it will not be put
# into the clipboard history (the current item is stored separately)
always_remove_patterns = [
  "^ssh-ed25519",
  "BEGIN [A-Z ]*PRIVATE KEY"
]

# Never expire matching items based on the time (they can still be 
# "pushed out" of the clipboard history when newer items are added)
never_remove_patterns = [
  "keep this"
]
```
