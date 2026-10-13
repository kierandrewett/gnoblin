# Shell Completion

Install shell completion for `gnoblinctl`.

Completion commands work without a running Gnoblin session. They use the installed CLI command's generated completion table.

## Install completion

```sh
# Bash: add to ~/.bashrc
eval "$(gnoblinctl completion bash)"

# Zsh: add to ~/.zshrc after compinit
eval "$(gnoblinctl completion zsh)"

# Fish: write the generated completion file
gnoblinctl completion fish > ~/.config/fish/completions/gnoblinctl.fish
```

Accepted shell names are `bash`, `zsh`, and `fish`. The commands print shell-specific completion code; the Fish command writes it to the standard user completion directory.
