# red-table shell integration; source with: eval "$(red-table shell-init bash)"
_red_table_complete() {
    local cur=${COMP_WORDS[COMP_CWORD]} prev=${COMP_WORDS[COMP_CWORD-1]} word
    COMPREPLY=()
    for word in "${COMP_WORDS[@]:1:COMP_CWORD-1}"; do
        [[ $word != -- ]] || return 0
    done
    local values= prefix=
    if [[ ${COMP_WORDS[1]} == shell-init && $COMP_CWORD -ge 2 ]]; then
        values='bash zsh fish'
    else
        if [[ $prev == = && $COMP_CWORD -ge 2 ]]; then
            prev=${COMP_WORDS[COMP_CWORD-2]}
        elif [[ $cur == = ]]; then
            cur=
        fi
        case $cur in
            --quality=*|--graphics-protocol=*|--thumbnail-size=*|--files0-from=*)
                prev=${cur%%=*}; prefix=$prev=; cur=${cur#*=} ;;
        esac
        case $prev in
            --quality) values='1 2 3 4 5 6 7 8 9' ;;
            --graphics-protocol) values='auto kitty sixel iterm2 halfblocks' ;;
            -s|--thumbnail-size) values='12x6 32x14 40x18 120x60' ;;
            --files0-from) values='-' ;;
            --config) return 0 ;;
            *)
                if [[ $cur == -* ]]; then
                    values='--help --version --thumbnail-size --quality --graphics-protocol --config --no-config --select --print0 --files0-from'
                elif [[ $COMP_CWORD == 1 ]]; then
                    values='shell-init'
                else
                    return 0
                fi ;;
        esac
    fi
    while IFS= read -r word; do
        COMPREPLY+=("$prefix$word")
    done < <(compgen -W "$values" -- "$cur")
}
complete -o default -F _red_table_complete red-table

_red_table_pick() {
    # Readline positions are byte offsets; use the same units for slicing.
    local LC_ALL=C
    local prefix=${READLINE_LINE:0:READLINE_POINT} suffix=${READLINE_LINE:READLINE_POINT}
    # Do not insert shell-escaped words into unfinished quotes or substitutions.
    printf '%s\n' "$prefix" | command bash --noprofile --norc -n 2>/dev/null || return 0
    local result path quoted insertion=
    result=$(command mktemp "${TMPDIR:-/tmp}/red-table-shell.XXXXXXXX") || return 0
    if command red-table --select --print0 > "$result"; then
        while IFS= read -r -d '' path; do
            printf -v quoted '%q' "$path"
            insertion+="$quoted "
        done < "$result"
    fi
    command rm -f -- "$result"
    [[ -n $insertion ]] || return 0
    insertion=" $insertion"
    READLINE_LINE=$prefix$insertion$suffix
    READLINE_POINT=$((${#prefix} + ${#insertion}))
}
if [[ $- == *i* ]]; then
    bind -m emacs-standard -x '"\ei":_red_table_pick'
    bind -m vi-insertion -x '"\ei":_red_table_pick'
fi
