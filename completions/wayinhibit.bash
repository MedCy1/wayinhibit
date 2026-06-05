_wayinhibit() {
    local cur prev
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"

    local i
    for (( i = 1; i < COMP_CWORD; i++ )); do
        if [[ "${COMP_WORDS[i]}" == -- ]]; then
            COMPREPLY=($(compgen -c -- "$cur"))
            return
        fi
    done

    case "$prev" in
        -t|--timeout)
            COMPREPLY=($(compgen -W "30s 1m 5m 10m 30m 1h 2h" -- "$cur"))
            return
            ;;
    esac

    case "$prev" in
        -p|--pid-file)
            COMPREPLY=($(compgen -f -- "$cur"))
            return
            ;;
    esac

    if [[ "$cur" == -* ]]; then
        COMPREPLY=($(compgen -W "-t --timeout -q --quiet -p --pid-file -h --help -V --version" -- "$cur"))
    fi
}

complete -F _wayinhibit wayinhibit
