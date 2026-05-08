Full removal. Two paths: the one command that does it, and the manual fallback for when the binary itself is broken.

## The simple way

```sh
sudo proteus uninstall --purge --yes
```

This:

1. Runs `proteus revert --yes` — restores the original MAC and hostname, removes systemd drop-ins, removes nft rules, removes the per-connection settings Proteus wrote to NetworkManager.
2. Disables and removes the timers and services: `proteus-rotate.timer`, `proteus-check.timer`, `proteus-boot.service`.
3. Removes the binary at `/usr/local/bin/proteus` (or wherever `install.sh` placed it).
4. With `--purge`: removes `/etc/proteus/` (config) and `/var/lib/proteus/` (state, including the cached original-MAC).

Without `--purge`, config and state are preserved so you can reinstall and resume where you left off. With `--purge`, the cached original-MAC is gone too — that cache is sacred (see `proteus wiki concepts`), so `--purge` is only for "I am done with this tool".

## Manual removal if `proteus uninstall` fails

If the binary is broken or missing and the one command above doesn't work:

```sh
# 1. Stop and disable the timers and services.
sudo systemctl disable --now proteus-rotate.timer proteus-check.timer proteus-boot.service
sudo rm -f /etc/systemd/system/proteus-rotate.timer
sudo rm -f /etc/systemd/system/proteus-rotate.service
sudo rm -f /etc/systemd/system/proteus-check.timer
sudo rm -f /etc/systemd/system/proteus-check.service
sudo rm -f /etc/systemd/system/proteus-boot.service
sudo systemctl daemon-reload

# 2. Remove the drop-ins Proteus wrote.
sudo rm -f /etc/sysctl.d/95-proteus.conf
sudo rm -f /etc/systemd/resolved.conf.d/10-proteus-*.conf
sudo rm -f /etc/systemd/timesyncd.conf.d/10-proteus.conf

# 3. Reload sysctls and restart resolved / timesyncd so the drop-in removals take effect.
sudo sysctl --system
sudo systemctl restart systemd-resolved systemd-timesyncd

# 4. Remove the nft table.
sudo nft delete table inet proteus 2>/dev/null || true

# 5. Restore NetworkManager per-connection settings.
# Proteus tags every connection it touches with `proteus-managed` in connection.user-data.
# Walk the tagged connections and reset the fields Proteus writes back to NM defaults.
nmcli -g name connection show | while read cn; do
  if nmcli -g connection.user-data connection show "$cn" | grep -q proteus-managed; then
    echo "resetting $cn"
    nmcli connection modify "$cn" wifi.cloned-mac-address ""
    nmcli connection modify "$cn" 802-1x.anonymous-identity ""
    nmcli connection modify "$cn" ipv4.dhcp-send-hostname yes
    nmcli connection modify "$cn" ipv4.dhcp-client-id ""
    nmcli connection modify "$cn" ipv4.dhcp-vendor-class-identifier ""
    nmcli connection modify "$cn" connection.user-data ""
  fi
done

# 6. Restore the original MAC on every interface from the cached state.
sudo jq -r '.original_macs | to_entries[] | "ip link set \(.key) addr \(.value)"' \
  /var/lib/proteus/state.json | sudo bash

# 7. Restore the original hostname from the cached state.
sudo hostnamectl set-hostname "$(sudo jq -r .original_hostname /var/lib/proteus/state.json)"

# 8. Remove the binary and the directories.
sudo rm -f /usr/local/bin/proteus
sudo rm -rf /etc/proteus /var/lib/proteus
```

Order matters. Read the cached state in step 6 and step 7 before deleting `/var/lib/proteus/` in step 8.

## Verification after removal

```sh
which proteus                       # empty
test -d /etc/proteus     && echo "STILL HERE"
test -d /var/lib/proteus && echo "STILL HERE"
systemctl list-units 'proteus-*'    # no units
nft list table inet proteus         # error: no such table
sysctl net.ipv4.tcp_timestamps      # back to your distro default (likely 1)
```

Anything left over is a bug — file it.

## What removal does not undo

- Hostname changes you made manually with `hostnamectl` after Proteus first ran. Set them yourself.
- Manual edits to NetworkManager connections after Proteus initially modified them. Proteus only knows what to undo for connections it owns (the `proteus-managed` tag).
- Sysctls or nft rules another tool wrote with the same names. Proteus removes only its own files.
- Reboot is sometimes the cleanest "make sure everything is fresh" step after removal.

## Reinstall

```sh
git clone https://github.com/Kit3713/Proteus.git
cd Proteus
cargo build --release --locked
sudo ./install.sh
```

If you didn't `--purge`, the config and state files are still there and Proteus picks up where it left off. If you did `--purge`, the original-MAC cache will be re-captured from whatever the current MAC is on first run — which means whatever MAC was applied when you uninstalled becomes the new "original". Reinstall before purge if that matters.

## Cross-refs

- `proteus wiki cli` — full uninstall command details and exit codes.
- `proteus wiki troubleshooting` — symptoms of partial removal and how to finish the job.
- `proteus wiki concepts` — why the original-MAC cache in `/var/lib/proteus/state.json` is sacred.
