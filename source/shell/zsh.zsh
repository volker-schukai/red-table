# red-table shell integration; run compinit first, then:
# eval "$(red-table shell-init zsh)"
_red_table_target() {
    _alternative 'command:command:(shell-init)' 'path:image or directory:_files'
}
_red_table_complete() {
    if [[ $words[2] == shell-init && CURRENT -ge 3 ]]; then
        _values 'shell' bash zsh fish
        return
    fi
    _arguments -s -S \
        '-h[Print help]' '--help[Print help]' \
        '-V[Print version]' '--version[Print version]' \
        '-s[Thumbnail size]:WIDTHxHEIGHT:(12x6 32x14 40x18 120x60)' \
        '--thumbnail-size=[Thumbnail size]:WIDTHxHEIGHT:(12x6 32x14 40x18 120x60)' \
        '--quality=[Thumbnail quality]:level:(1 2 3 4 5 6 7 8 9)' \
        '--graphics-protocol=[Graphics protocol]:protocol:(auto kitty sixel iterm2 halfblocks)' \
        '--config=[Configuration file]:file:_files' \
        '--no-config[Disable configuration discovery]' \
        '--select[Confirm marked image paths to stdout]' \
        '--print0[NUL-delimit confirmed paths]' \
        '--files0-from=[Read NUL-delimited paths from stdin]:source:(-)' \
        '1:image or directory:_red_table_target'
}
if (( $+functions[compdef] )); then
    compdef _red_table_complete red-table
fi

_red_table_pick() {
    emulate -L zsh
    local prefix=$LBUFFER suffix=$RBUFFER
    print -r -- "$prefix" | command zsh -f -n 2>/dev/null || return 0
    local result selected_path insertion=
    result=$(command mktemp "${TMPDIR:-/tmp}/red-table-shell.XXXXXXXX") || return 0
    if command red-table --select --print0 > "$result"; then
        while IFS= read -r -d '' selected_path; do
            insertion+="${(q)selected_path} "
        done < "$result"
    fi
    command rm -f -- "$result"
    [[ -n $insertion ]] || return 0
    LBUFFER="$prefix $insertion"
    RBUFFER=$suffix
    if [[ -o zle ]]; then
        zle reset-prompt
    fi
}
if [[ -o interactive ]]; then
    zle -N _red_table_pick
    bindkey -M emacs '^[i' _red_table_pick
    bindkey -M viins '^[i' _red_table_pick
fi
