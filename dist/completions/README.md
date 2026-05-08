# Proteus shell completions

Hand-written completion scripts for `proteus`. Each is small,
re-source-able, and covers every subcommand in `src/cli.rs`. Regenerate
the bundled scripts from the binary with `proteus completions <bash|zsh|fish>`.

`install.sh` installs these to standard system paths automatically.
The instructions below are for manual installation or per-user use.

## Bash

System-wide (preferred when bash-completion is installed):

    sudo cp dist/completions/proteus.bash /etc/bash_completion.d/proteus
    # or, on systems that prefer the bash-completion data dir:
    sudo cp dist/completions/proteus.bash /usr/share/bash-completion/completions/proteus

Per-user:

    cp dist/completions/proteus.bash ~/.proteus-completion.bash
    echo 'source ~/.proteus-completion.bash' >> ~/.bashrc

## Zsh

Copy to a directory in your `$fpath`. The file must be named `_proteus`:

    sudo cp dist/completions/proteus.zsh /usr/share/zsh/site-functions/_proteus

Per-user:

    mkdir -p ~/.zsh/completions
    cp dist/completions/proteus.zsh ~/.zsh/completions/_proteus
    # in ~/.zshrc:
    #   fpath=(~/.zsh/completions $fpath)
    #   autoload -Uz compinit && compinit

## Fish

Per-user:

    cp dist/completions/proteus.fish ~/.config/fish/completions/proteus.fish

System-wide:

    sudo cp dist/completions/proteus.fish /usr/share/fish/vendor_completions.d/proteus.fish

Fish picks them up on the next shell start; no `source` needed.

## Coverage

Each script completes:

- All subcommands present in `src/cli/command.rs` (status, session,
  current, original, show-config, show-defaults, apply, revert, rotate,
  rotate-if-needed, pin, unpin, diff, dry-run, reset, uninstall,
  bluetooth, hostname, ipv6, enterprise-wifi, stack, dns, resolved, ntp,
  dhcp, wiki, help, timer, config, doctor, probe, kill, resume, nft,
  portal, rf, persona, ssid, events, completions).
- Global flags (`-v`, `-q`, `--config`, `--state`, `--no-color`,
  `--format`, `-h`, `-V`).
- `--json` on read commands (`status`, `session`, `current`, `original`,
  `show-config`, `show-defaults`, `diff`, `doctor`, `probe`).
- `--yes` on mutating commands (`apply`, `revert`, `rotate`,
  `rotate-if-needed`, `reset`, `uninstall`, `resume`, `kill`).
- `--watch` / `--interval` on `status`, `session`, `current`.
- `--iface` on `current`, `rotate`, `rotate-if-needed`, `dhcp renew`,
  with interface names pulled from `ip link`.
- `--purge` on `uninstall`, `--dry-run` on `reset`, `--explain` on
  `rotate`, `--quick` on `doctor`/`probe`, `--mac` on `pin`,
  `--cooldown`/`--ssid` on `rotate-if-needed`.
- Wiki page names for `wiki <page>` and `help <feature>` — full embedded
  set sourced from `wiki/*.md`.
- Action enums for nested commands (e.g. `bluetooth {status,apply,revert}`,
  `config {show,get,set,enable,disable,edit,validate,reset,keys,set-profile}`,
  `persona {list,show,use,clear,current,random,new,edit,validate,import,export}`,
  `timer {status,list,enable,disable,set,reset,logs}`).
- Shell argument for `completions` (`bash`, `zsh`, `fish`).
- Interface names + NetworkManager connection profiles for `pin`/`unpin`
  (the latter via `nmcli` when available).

The wiki page list is static and matches the embedded wiki at the
binary's build time.
