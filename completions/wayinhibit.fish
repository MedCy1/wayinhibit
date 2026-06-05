complete -c wayinhibit -f

complete -c wayinhibit -s t -l timeout \
    -d 'Stop after a given duration (e.g. 30s, 5m, 2h)' \
    -r -a '30s 1m 5m 10m 30m 1h 2h'
complete -c wayinhibit -s q -l quiet \
    -d 'Suppress all output'
complete -c wayinhibit -s h -l help \
    -d 'Print help and exit'
complete -c wayinhibit -s V -l version \
    -d 'Print version and exit'
