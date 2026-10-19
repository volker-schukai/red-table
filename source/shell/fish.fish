# red-table shell integration; source with: red-table shell-init fish | source
complete -c red-table -n '__fish_use_subcommand' -a shell-init -d 'Emit shell integration'
complete -c red-table -n '__fish_seen_subcommand_from shell-init' -f -a 'bash zsh fish'
complete -c red-table -n 'not __fish_seen_subcommand_from shell-init' -s h -l help -d 'Print help'
complete -c red-table -n 'not __fish_seen_subcommand_from shell-init' -s V -l version -d 'Print version'
complete -c red-table -n 'not __fish_seen_subcommand_from shell-init' -s s -l thumbnail-size -x -a '12x6 32x14 40x18 120x60' -d 'Thumbnail size in terminal cells'
complete -c red-table -n 'not __fish_seen_subcommand_from shell-init' -l quality -x -a '1 2 3 4 5 6 7 8 9' -d 'Thumbnail quality'
complete -c red-table -n 'not __fish_seen_subcommand_from shell-init' -l graphics-protocol -x -a 'auto kitty sixel iterm2 halfblocks' -d 'Graphics protocol'
complete -c red-table -n 'not __fish_seen_subcommand_from shell-init' -l config -r -F -d 'Configuration file'
complete -c red-table -n 'not __fish_seen_subcommand_from shell-init' -l no-config -d 'Disable configuration discovery'
complete -c red-table -n 'not __fish_seen_subcommand_from shell-init' -l select -d 'Confirm marked paths to stdout'
complete -c red-table -n 'not __fish_seen_subcommand_from shell-init' -l print0 -d 'NUL-delimit confirmed paths'
complete -c red-table -n 'not __fish_seen_subcommand_from shell-init' -l files0-from -x -a '-' -d 'Read NUL-delimited paths from stdin'

function _red_table_pick
    set -l prefix (commandline --cut-at-cursor | string collect --allow-empty)
    printf '%s\n' "$prefix" | command fish --no-config --no-execute 2>/dev/null
    or return 0
    set -l tempdir /tmp
    if set -q TMPDIR[1]; and test -n "$TMPDIR"
        set tempdir "$TMPDIR"
    end
    set -l result (command mktemp "$tempdir/red-table-shell.XXXXXXXX")
    test -n "$result"; or return 0
    set -l insertion ''
    if command red-table --select --print0 > "$result"
        while read --null --local selected_path
            set insertion "$insertion"(string escape -- "$selected_path")' '
        end < "$result"
    end
    command rm -f -- "$result"
    if test -n "$insertion"
        commandline --insert -- " $insertion"
    end
    commandline -f repaint
end
if status is-interactive
    bind \ei _red_table_pick
    bind -M insert \ei _red_table_pick
end
