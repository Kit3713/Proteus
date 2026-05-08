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

- All subcommands present in `src/cli.rs`.
- Global flags (`-v`, `-q`, `--config`, `--state`, `--no-color`, `-h`, `-V`).
- `--json` on read commands (`status`, `current`, `original`, `show-config`,
  `show-defaults`, `diff`).
- `--yes` on mutating commands (`apply`, `revert`, `rotate`, `reset`,
  `uninstall`).
- `--iface` on `current` and `rotate`, with interface name suggestions
  pulled from `ip link`.
- `--purge` on `uninstall`.
- Wiki page names for `wiki <page>` and `help <feature>`.
- Interface names + NetworkManager connection profiles for `pin`/`unpin`
  (the latter via `nmcli` when available).

The wiki page list is static and matches the embedded wiki at the
binary's build time.
